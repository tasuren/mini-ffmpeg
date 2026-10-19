//! Translate one C translation unit to Rust with cinrs.
//!
//! Call `cinrs-core` directly to perform the translation normally done at compile
//! time by cinrs macros (such as `c17!`), returning the result as a string. Macro
//! settings (pragmas, `#define`, include directories) are passed as fields of
//! [`cinrs_core::Options`]; [`Settings::options`] maps them.
//!
//! Macros are not used because they cannot select `Options::reachable_only`, which
//! emits only items used by the translation unit.
//!
//! Two kinds of translation units ([`UnitKind`]) use slightly different settings:
//! FFmpeg `.c` files and units that only include public headers.

#[path = "platform_workarounds/mod.rs"]
mod platform_workarounds;

use self::platform_workarounds::WorkaroundSet;
use std::path::{Path, PathBuf};

use cinrs_core::{
    CommandLineMacro, Dialect, FileTranslation, Level, Options, Standard, TargetModel,
    include::{HeaderKind, System},
};
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

/// Macro defined only when building FFmpeg itself.
///
/// When defined, FFmpeg headers read `config.h` and enable architecture-specific
/// internal headers (such as `arm/bswap.h` from `libavutil/bswap.h`) and internal
/// assertions. Programs using FFmpeg do not define it.
const FFMPEG_INTERNAL_DEFINE: &str = "HAVE_AV_CONFIG_H";

/// Translation unit kind, selecting the settings that differ between kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    /// An FFmpeg `.c` file containing function and variable definitions emitted as C symbols.
    ///
    /// Unused items are omitted. Rust code does not reference them by name; other
    /// translation units link to them by C symbol names.
    Implementation,

    /// A translation unit that only includes public headers, gathering user-facing
    /// function declarations, types, and enumerators in one namespace.
    ///
    /// It has no definitions and emits no symbols. Its declarations link to definitions
    /// in [`UnitKind::Implementation`] units. It reads headers as an FFmpeg consumer
    /// would, without defining [`FFMPEG_INTERNAL_DEFINE`].
    PublicApi,
}

/// Translation settings shared by all translation units.
#[derive(Debug, Clone)]
pub struct Settings {
    /// C standard, in `-std=` value syntax (such as `c17` or `gnu17`).
    pub c_standard: String,

    /// Target triple for compiling the translated code.
    pub target: String,

    /// Macro definitions in `NAME` or `NAME=VALUE` form, corresponding to `-D`.
    pub defines: Vec<String>,

    /// Include directories in search order, corresponding to `-I`.
    pub include_dirs: Vec<PathBuf>,
}

/// Result of translating one translation unit.
#[derive(Debug)]
pub struct Translated {
    /// Translated Rust code: the translation unit's module and re-exports of its contents.
    ///
    /// Not formatted for human readers: no indentation, only line breaks at
    /// statement and brace boundaries.
    pub rust: String,

    /// C symbols defined by the translation unit: function and variable names with external linkage.
    pub symbols: Vec<String>,

    /// C names of external functions and variables only declared by the translation
    /// unit, appearing in `extern` blocks in the translated code.
    pub declarations: Vec<String>,

    /// Header paths read by the translation unit, excluding cinrs's bundled headers.
    ///
    /// FFmpeg header paths are relative to the supplied include directories;
    /// platform header paths are absolute.
    pub headers: Vec<String>,

    /// cinrs warnings, each formatted with its location and description.
    pub warnings: Vec<String>,
}

impl Settings {
    /// Translate the translation unit at `source`.
    ///
    /// `__FILE__` and diagnostics use the supplied source and include directory paths
    /// unchanged. FFmpeg uses `__FILE__` in `av_assert0` and elsewhere, so absolute paths
    /// would embed build-machine paths in executables. Supply both as relative paths
    /// and call this with their base directory as the working directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, the C source has errors, or the
    /// translated code requires the cinrs runtime.
    pub fn translate(&self, source: &Path) -> Result<Translated, String> {
        let mut options = self.options(UnitKind::Implementation)?;
        let workarounds = WorkaroundSet::for_target(&self.target);
        workarounds.apply(&mut options);

        // Check that unsupported platform types occur only in known declarations
        // covered by the workarounds.
        if workarounds.needs_validation() {
            workarounds.validate(&cinrs_core::preprocess_file(source, &options)?)?;
        }

        finish(
            cinrs_core::translate_file(source, &options)?,
            workarounds,
            false,
        )
    }

    /// Translate a translation unit that only includes public headers.
    ///
    /// `name` identifies the C source string (`source`) supplied instead of a file and
    /// is used in diagnostics and `__FILE__`. Use a relative path for the same reason
    /// as in [`Settings::translate`].
    ///
    /// # Errors
    ///
    /// The same conditions as [`Settings::translate`].
    pub fn translate_public_api(&self, name: &str, source: &str) -> Result<Translated, String> {
        let mut options = self.options(UnitKind::PublicApi)?;
        let workarounds = WorkaroundSet::for_target(&self.target);
        workarounds.apply(&mut options);

        // Check that unsupported platform types occur only in known declarations
        // covered by the workarounds.
        if workarounds.needs_validation() {
            workarounds.validate(&cinrs_core::preprocess_source(
                name.to_owned(),
                source.to_owned(),
                &options,
            ))?;
        }

        finish(
            cinrs_core::translate_source(name.to_owned(), source.to_owned(), &options),
            workarounds,
            true,
        )
    }

    /// Assemble options for cinrs.
    ///
    /// Defaults match cinrs macros, so set only options normally specified inside a
    /// macro and options unavailable to macros. cinrs also provides `ccinrs`, which
    /// accepts C compiler command lines and changes more options (see `options` in
    /// cinrs's `crates/ccinrs/src/driver.rs`). If a cinrs update changes translation
    /// results, compare that function for newly configured options.
    fn options(&self, kind: UnitKind) -> Result<Options, String> {
        let (standard, dialect) = parse_c_standard(&self.c_standard)?;

        // Set the target explicitly. Otherwise cinrs infers it from its host or the
        // `CINRS_TARGET` environment variable. A mismatch with the build target causes
        // assertions in the translated code to fail compilation.
        let target = TargetModel::from_triple(&self.target)
            .map_err(|error| format!("cinrs does not support target {}: {error}", self.target))?;

        let mut options = Options::with_dialect(standard, dialect).for_target(target);

        options.include_paths = self.include_dirs.clone();

        // Emit functions and variables with external linkage under their C symbol names,
        // corresponding to `#pragma cinrs export`. Translation units refer to each other
        // by C symbol names, so linking requires this.
        //
        // The public API translation unit has no definitions and does not need it.
        // Disabling it also avoids duplicate implementation symbols if a header
        // unexpectedly contains definitions.
        options.export = kind == UnitKind::Implementation;

        // Search platform headers before cinrs's bundled headers, corresponding to
        // `#pragma cinrs system_include first`.
        //
        // Searching bundled headers first mixes the two sets and causes conflicting
        // `int64_t` definitions on macOS. `config.h` records checks against platform
        // headers, so using the same headers is also consistent with that configuration.
        options.system_include = System::First;

        // Accept standard violations that GCC treats as warnings. This matters only
        // for ISO standards (`c17`); GNU dialects (`gnu17`) already accept them.
        //
        // The original build passes `-std=c17` to the C compiler, which does not reject
        // these constructs. Platform headers also use them: macOS's
        // `<mach_debug/ipc_info.h>` uses enumerator values outside the `int` range and
        // is included by `libavutil/random_seed.c` through `<mach/mach_time.h>`.
        options.gnu_leniencies = true;

        // Emit only symbols defined by the translation unit and items reachable from them.
        //
        // C's `#include` brings every header declaration into the translation unit.
        // By default cinrs translates unused declarations, `static inline` functions,
        // and types too. They account for roughly three quarters of the output in the
        // current configuration, increasing rustc's work and build time by over a minute.
        //
        // This is disabled by default because Rust surrounding a macro may reference its
        // items by name. Translated units only link through C symbol names, so definitions
        // visible to the linker suffice as roots.
        //
        // The tradeoff is that unused types and declarations become inaccessible from Rust.
        //
        // Disable filtering for the public API translation unit: exposing declarations
        // and types is its purpose, and it has no definitions to use as roots, so filtering
        // would leave nothing.
        options.reachable_only = kind == UnitKind::Implementation;

        // Reject complex types (`_Complex`). cinrs translates complex arithmetic to calls
        // into its runtime crate (`cinrs-rt`), which would add a runtime dependency to
        // `mini-ffmpeg-sys`. Disabling it keeps translated code dependent only on `core`.
        //
        // cinrs macros enable this by default. Disabling it defines `__STDC_NO_COMPLEX__`,
        // so headers conditional on that macro take a different branch than with macros.
        // If FFmpeg starts using complex types, translation fails here.
        options.complex = false;

        for define in &self.defines {
            let name = define
                .split_once('=')
                .map_or(define.as_str(), |(name, _)| name);

            if kind == UnitKind::PublicApi && name == FFMPEG_INTERNAL_DEFINE {
                continue;
            }

            options
                .macros
                .push(CommandLineMacro::Define(define.clone()));
        }

        Ok(options)
    }
}

/// Validate cinrs's translation result and prepare it for writing to a file.
///
/// # Errors
///
/// Returns an error if the C source has errors or the translated code requires
/// the cinrs runtime.
fn finish(
    translation: FileTranslation,
    workarounds: WorkaroundSet,
    public_api: bool,
) -> Result<Translated, String> {
    let diagnostics = &translation.diagnostics;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for diagnostic in diagnostics.sorted() {
        let rendered = diagnostics.render(&translation.map, diagnostic);

        match diagnostic.level {
            Level::Error => errors.push(rendered),
            Level::Warning => warnings.push(rendered),
        }
    }

    // Errors without locations, such as failure to open a file.
    if let Some(message) = diagnostics.fatal_message() {
        errors.push(message.to_owned());
    }

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }

    let items = translation
        .items
        .ok_or("cinrs returned no translated code without reporting an error")?;

    // With complex types disabled, no runtime calls should be emitted. If cinrs
    // starts using its runtime for another purpose, reject the output rather than
    // silently emitting references to a crate that is not a dependency.
    if translation.uses_runtime {
        return Err(
            "The translated code requires the cinrs runtime (cinrs-rt).\n\
             Decide whether to add this dependency to mini-ffmpeg-sys before proceeding."
                .to_owned(),
        );
    }

    let headers = translation
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::Bundled))
        .map(|header| header.name.clone())
        .collect();

    let items = if public_api {
        workarounds.exclude_declarations(items)?
    } else {
        items
    };
    workarounds.check_references(items.clone())?;

    let mut declarations = Vec::new();
    collect_declarations(items.clone(), &mut declarations);

    let mut writer = TokenWriter::default();
    writer.write(items);

    Ok(Translated {
        rust: writer.out,
        symbols: translation.symbols,
        declarations,
        headers,
        warnings,
    })
}

/// Collect C names of functions and variables in `extern` blocks in translated code.
///
/// cinrs places declaration-only functions and variables in `unsafe extern "C" { … }`
/// blocks. Find brace groups following `extern` and an ABI string, then collect
/// `fn` and `static` names inside them. Recurse through groups to find blocks
/// nested in modules or elsewhere.
///
/// Declarations whose Rust and C names differ have `#[link_name = "…"]`; use
/// that attribute's value as the C name.
fn collect_declarations(tokens: TokenStream, out: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();

    for (index, token) in tokens.iter().enumerate() {
        let TokenTree::Group(group) = token else {
            continue;
        };

        let is_extern_block = group.delimiter() == Delimiter::Brace
            && index >= 2
            && matches!(&tokens[index - 2], TokenTree::Ident(ident) if ident == "extern")
            && matches!(&tokens[index - 1], TokenTree::Literal(_));

        if is_extern_block {
            collect_extern_items(group.stream(), out);
        } else {
            collect_declarations(group.stream(), out);
        }
    }
}

/// Collect C names of declared functions and variables from an `extern` block's contents.
fn collect_extern_items(tokens: TokenStream, out: &mut Vec<String>) {
    let mut link_name = None;
    let mut tokens = tokens.into_iter().peekable();

    while let Some(token) = tokens.next() {
        match token {
            // Remember a `link_name` attribute as the C name of the next declaration.
            TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket => {
                if let Some(name) = find_link_name(group.stream()) {
                    link_name = Some(name);
                }
            }

            TokenTree::Ident(ident) if ident == "fn" || ident == "static" => {
                // Skip `mut` in `static mut name`.
                if matches!(tokens.peek(), Some(TokenTree::Ident(next)) if next == "mut") {
                    tokens.next();
                }

                if let Some(TokenTree::Ident(name)) = tokens.next() {
                    out.push(link_name.take().unwrap_or_else(|| name.to_string()));
                }
            }

            _ => {}
        }
    }
}

/// Extract the name string from attribute contents (`link_name = "…"`).
///
/// The value is a string literal token; remove only its surrounding quotes without
/// interpreting escapes. Escapes appear mainly in platform header `__asm` labels
/// (such as `"\u{1}_nanosleep"`), not in FFmpeg names.
fn find_link_name(tokens: TokenStream) -> Option<String> {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();

    match tokens.as_slice() {
        [
            TokenTree::Ident(key),
            TokenTree::Punct(equals),
            TokenTree::Literal(value),
        ] if key == "link_name" && equals.as_char() == '=' => {
            let value = value.to_string();
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .map(str::to_owned)
        }

        // Also handle the `#[unsafe(link_name = "…")]` form.
        [TokenTree::Ident(word), TokenTree::Group(inner)] if word == "unsafe" => {
            find_link_name(inner.stream())
        }

        _ => None,
    }
}

/// Convert a C standard name to a cinrs standard and dialect pair.
///
/// Names use `-std=` syntax; a `gnu` prefix selects the GNU dialect. Reject
/// unsupported values rather than silently translating with another standard.
fn parse_c_standard(name: &str) -> Result<(Standard, Dialect), String> {
    let (dialect, revision) = match name.strip_prefix("gnu") {
        Some(revision) => (Dialect::Gnu, revision),
        None => (Dialect::Iso, name.strip_prefix('c').unwrap_or_default()),
    };

    let standard = match revision {
        "89" => Standard::C89,
        "99" => Standard::C99,
        "11" => Standard::C11,
        "17" => Standard::C17,
        "23" => Standard::C23,
        _ => return Err(format!("cinrs does not support C standard {name}")),
    };

    Ok((standard, dialect))
}

/// Once a line exceeds this length, break at the next `,`. This handles sequences
/// without statement or brace boundaries, such as array initializers.
const LINE_WIDTH: usize = 100;

/// Render a token stream to a string with moderate line lengths.
///
/// Stringifying cinrs's token stream directly puts the entire translation unit
/// on one line. That is valid Rust, but rustc handles extremely long lines slowly:
/// compilation of `mini-ffmpeg-sys` increased from about 6 to about 33 seconds.
/// Error and panic locations would also all point to line 1.
///
/// Formatting crates (`syn` and `prettyplease`) would improve readability, but
/// these files live in `OUT_DIR` and are not intended for human reading. Add only
/// line breaks, avoiding extra dependencies: after `;`, inside and after braces,
/// and after `,` on long lines. Inserting breaks only between tokens preserves
/// code semantics. String literals are single tokens and remain untouched.
#[derive(Default)]
struct TokenWriter {
    out: String,

    /// Number of characters written on the current line.
    line_width: usize,
}

impl TokenWriter {
    fn write(&mut self, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Group(group) => {
                    let (open, close, is_block) = match group.delimiter() {
                        Delimiter::Brace => ("{", "}", true),
                        Delimiter::Parenthesis => ("(", ")", false),
                        Delimiter::Bracket => ("[", "]", false),
                        Delimiter::None => ("", "", false),
                    };

                    self.push(open);
                    self.separate(is_block);

                    self.write(group.stream());

                    self.push(close);
                    self.separate(is_block);
                }

                TokenTree::Punct(punct) => {
                    let character = punct.as_char();
                    self.push(character.encode_utf8(&mut [0; 4]));

                    // `Joint` marks punctuation joined to the next token as one operator (`::`,
                    // `->`, or a lifetime's `'`). Inserting whitespace would split it into
                    // different tokens.
                    if punct.spacing() == Spacing::Alone {
                        let is_long_line = self.line_width > LINE_WIDTH;
                        self.separate(character == ';' || (character == ',' && is_long_line));
                    }
                }

                TokenTree::Ident(ident) => {
                    self.push(&ident.to_string());
                    self.separate(false);
                }

                TokenTree::Literal(literal) => {
                    self.push(&literal.to_string());
                    self.separate(false);
                }
            }
        }
    }

    fn push(&mut self, text: &str) {
        self.out.push_str(text);
        self.line_width += text.len();
    }

    /// Write a token separator: a newline if `break_line`, otherwise a space.
    fn separate(&mut self, break_line: bool) {
        if break_line {
            self.out.push('\n');
            self.line_width = 0;
        } else {
            self.out.push(' ');
            self.line_width += 1;
        }
    }
}
