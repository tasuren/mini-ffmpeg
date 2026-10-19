//! Translate FFmpeg's C code to Rust with cinrs.
//!
//! Translate each translation unit in the build manifest
//! (`FFmpeg_build/<target>/cinrs_manifest.toml`) and write it to Cargo's output
//! directory (`OUT_DIR`), where `src/lib.rs` loads it.
//!
//! - `<library>/<name>.rs`: One translated translation unit, containing FFmpeg
//!   function and variable definitions.
//! - `public_api.rs`: Translated public headers (`public_headers` in the build
//!   manifest), containing function declarations, types, and enumerators for users.
//! - `generated.rs`: Module declarations loaded by `include!` in `src/lib.rs`.
//!
//! # Why translation runs in the build script
//!
//! Writing a cinrs macro (`c17!`, for example) for each translation unit is an
//! alternative, but is not used for the following reasons:
//!
//! - Macros cannot select the option that emits only items used by a translation
//!   unit. The entire contents of headers become Rust code, adding over a minute
//!   to the build. See `build/translate.rs` for the option's details.
//! - Macros expand sequentially every time the crate is compiled. The build script
//!   only reruns when its inputs change and can translate units in parallel.
//!
//! Translated Rust code is also not checked into the repository or included in the
//! crate package. In addition to FFmpeg code, it contains platform header declarations
//! and inline functions. Instead of distributing that output, we distribute the
//! unmodified FFmpeg source and translate it on the build machine using that
//! machine's headers, as a build with a C compiler would do.
//!
//! Building this crate therefore requires the FFmpeg source tree and platform
//! headers (the Command Line Tools SDK on macOS). A C compiler is not required.

#[path = "build/translate.rs"]
mod translate;

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    num::NonZero,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use cinrs_core::include::SYSTEM_PATH_ENV_VAR;

use self::translate::{Settings, Translated};

/// Errors are only displayed before exiting, so strings suffice without separate error types.
type Result<T> = std::result::Result<T, String>;

/// Directory containing target-specific build configurations. Each
/// `FFmpeg_build/<target-triple>/` contains a build manifest and files generated
/// by `configure` / `make`.
const BUILD_CONFIG_DIR: &str = "FFmpeg_build";

/// Build manifest within the target's configuration directory.
/// Written by `cargo xtask ffmpeg-manifest`.
const MANIFEST_FILE: &str = "cinrs_manifest.toml";

/// Environment variable that passes the build manifest path to tests.
///
/// Each target has its build manifest in a different location, so `include_str!`
/// cannot use a fixed path. The build script passes its selected path via `rustc-env`.
const MANIFEST_PATH_ENV: &str = "MINI_FFMPEG_SYS_MANIFEST";

/// FFmpeg source tree (a submodule).
const FFMPEG_DIR: &str = "FFmpeg";

/// File containing module declarations, relative to `OUT_DIR`.
///
/// Keep this in sync with the `include!` in `src/lib.rs`.
const ROOT_FILE: &str = "generated.rs";

/// File containing translated public headers, relative to `OUT_DIR`.
const PUBLIC_API_FILE: &str = "public_api.rs";

/// Name of the translation unit that only includes public headers, used in diagnostics and `__FILE__`.
///
/// This file does not exist on disk. The build script assembles its C source and
/// passes it directly to cinrs as a string.
const PUBLIC_API_SOURCE_NAME: &str = "public_api.c";

/// Common prefix of FFmpeg's public function and variable names.
///
/// The public API translation unit contains declarations from platform headers
/// (such as `strcat`) included by public headers, as well as FFmpeg declarations.
/// Matching declarations to definitions ([`check_public_declarations`]) must only
/// consider FFmpeg declarations. cinrs does not report the originating header for
/// each declaration, so names are used to distinguish them.
///
/// FFmpeg's public names all start with `av`, such as `av_`, `avio_`, `avcodec_`,
/// `avformat_`, and `avutil_`. Deviations behave as follows:
///
/// - A platform function starting with `av` causes a false missing-definition error.
///   Decide whether to add it to [`UNDEFINED_PUBLIC_DECLARATIONS`].
/// - An FFmpeg public name that does not start with `av` is not checked.
const FFMPEG_PUBLIC_PREFIX: &str = "av";

/// Names declared by public headers but not defined in this configuration.
///
/// They cause no problem unless called, at which point linking fails. Keep the
/// reasons here to document why visible declarations cannot be used.
const UNDEFINED_PUBLIC_DECLARATIONS: &[&str] = &[
    // Android-only functions unconditionally declared by `libavcodec/jni.h`. Their
    // definitions are inside `#if defined(__ANDROID__)` in `libavcodec/jni.c`.
    // The header also documents them as available only on Android.
    "av_jni_get_android_app_ctx",
    "av_jni_set_android_app_ctx",
];

/// Stack size for translation worker threads.
///
/// Rendering translated code to a string recurses to the depth of nested
/// expressions. Expanded C macros can produce deeply nested expressions that may
/// not fit in a thread's default 2 MiB stack, so reserve extra space. The reservation
/// uses virtual memory; unused portions do not consume physical memory.
const WORKER_STACK_SIZE: usize = 64 << 20;

fn main() -> ExitCode {
    // Once a build script emits any `rerun-if-changed` directives, Cargo reruns it
    // only when a listed file changes. Without any directives, every file in the
    // package is tracked. Emit these first so they also apply when the script fails.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/translate.rs");
    // Track the directory so adding a platform module also reruns the build.
    println!("cargo:rerun-if-changed=build/platform_workarounds");

    // Track the current target's manifest, since each target has a separate file.
    // Track missing manifests too, so creating one reruns the build script.
    if let Ok(target) = env::var("TARGET") {
        println!(
            "cargo:rerun-if-changed={}",
            manifest_path(&target).display()
        );
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,

        Err(error) => {
            // `cargo::error` accepts only one line. Translation errors span multiple lines,
            // so write the full message to stderr. Cargo displays stderr when a build
            // script fails.
            eprintln!("{error}");

            let summary = error.lines().next().unwrap_or_default();
            println!("cargo::error={summary}");

            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    // Cargo runs build scripts with the package root as the working directory.
    // All subsequent paths are relative to that directory. Passing absolute paths
    // to cinrs is undesirable, but these relative paths avoid that issue here.
    // See `Settings::translate` for the reason.

    let target = cargo_env("TARGET")?;
    let manifest_path = manifest_path(&target);

    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "cannot read {}: {error}\n\
             A configuration for {target} may not have been recorded. \
             Available targets: {}",
            manifest_path.display(),
            available_targets(),
        )
    })?;
    let manifest = Manifest::parse(&manifest_text)?;

    println!(
        "cargo:rustc-env={MANIFEST_PATH_ENV}={}",
        env::current_dir()
            .map_err(|error| format!("cannot get the working directory: {error}"))?
            .join(&manifest_path)
            .display(),
    );

    check_target(&manifest, &target)?;
    require_ffmpeg_tree()?;

    // Sort translation units by name and remove duplicates so the output does not
    // depend on their order in the build manifest.
    let units: Vec<Unit> = manifest
        .translation_units
        .iter()
        .map(|path| Unit::parse(path))
        .collect::<Result<BTreeSet<_>>>()?
        .into_iter()
        .collect();

    let settings = Settings {
        c_standard: manifest.c_standard,
        target,
        defines: manifest.defines,
        include_dirs: manifest.include_dirs.iter().map(PathBuf::from).collect(),
    };

    // The single public API translation unit is not filtered, so it takes longer
    // than one implementation translation unit. Translate it alongside the
    // implementation translation units.
    let (translated, public_api) = thread::scope(|scope| {
        let public_api = thread::Builder::new()
            .stack_size(WORKER_STACK_SIZE)
            .spawn_scoped(scope, || {
                translate_public_api(&settings, &manifest.public_headers)
            })
            .map_err(|error| format!("cannot start a translation thread: {error}"))?;

        let translated = translate_units(&settings, &units);
        let public_api = public_api
            .join()
            .map_err(|_| "translation thread panicked".to_owned())?;

        Ok::<_, String>((translated?, public_api?))
    })?;

    check_duplicate_symbols(&units, &translated)?;
    check_public_declarations(&public_api, &translated)?;

    let out_dir = PathBuf::from(cargo_env("OUT_DIR")?);

    for (unit, output) in units.iter().zip(&translated) {
        write_file(
            &out_dir.join(unit.module_file()),
            &render_unit(unit, &output.rust),
        )?;

        // Flatten warnings to one line before passing them to Cargo. Otherwise,
        // subsequent lines could be interpreted as Cargo directives.
        for warning in &output.warnings {
            let warning = warning.replace('\n', " ");
            println!("cargo::warning={}: {warning}", unit.source_path());
        }
    }

    write_file(
        &out_dir.join(PUBLIC_API_FILE),
        &render_public_api(&public_api.rust),
    )?;

    for warning in &public_api.warnings {
        let warning = warning.replace('\n', " ");
        println!("cargo::warning=public headers: {warning}");
    }

    write_file(&out_dir.join(ROOT_FILE), &render_root(&out_dir, &units)?)?;

    // Retranslate when any input changes: each translation unit's `.c` file and all
    // headers it reads. Including platform headers also triggers retranslation
    // when the SDK is updated.
    let mut inputs = BTreeSet::new();

    for (unit, output) in units.iter().zip(&translated) {
        inputs.insert(unit.source_file().display().to_string());
        inputs.extend(output.headers.iter().cloned());
    }

    inputs.extend(public_api.headers.iter().cloned());

    for input in inputs {
        println!("cargo:rerun-if-changed={input}");
    }

    Ok(())
}

/// Target build manifest path, relative to the package root.
fn manifest_path(target: &str) -> PathBuf {
    Path::new(BUILD_CONFIG_DIR).join(target).join(MANIFEST_FILE)
}

/// Available target triples, formatted for error messages.
fn available_targets() -> String {
    let mut targets: Vec<String> = fs::read_dir(BUILD_CONFIG_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    targets.sort();

    if targets.is_empty() {
        "none".to_owned()
    } else {
        targets.join(", ")
    }
}

/// Read an environment variable set by Cargo for the build script.
fn cargo_env(name: &str) -> Result<String> {
    env::var(name).map_err(|error| format!("cannot read environment variable {name}: {error}"))
}

/// Parts of the build manifest used for translation.
#[derive(Debug)]
struct Manifest {
    /// Target triple for this configuration, in the same format as Cargo's `TARGET`.
    triple: String,

    /// C standard, in the same format as a `-std=` value.
    c_standard: String,

    /// Macro definitions in `NAME` or `NAME=VALUE` form.
    defines: Vec<String>,

    /// Include directories, relative to the package root.
    include_dirs: Vec<String>,

    /// Translation units, relative to the FFmpeg source tree.
    translation_units: Vec<String>,

    /// Public headers, using their `#include` names (such as `libavutil/avutil.h`).
    public_headers: Vec<String>,
}

impl Manifest {
    fn parse(text: &str) -> Result<Self> {
        let table: toml::Table = text
            .parse()
            .map_err(|error| format!("cannot read {MANIFEST_FILE}: {error}"))?;

        Ok(Self {
            triple: string(&table, "target", "triple")?,
            c_standard: string(&table, "compile", "std")?,
            defines: string_array(&table, "compile", "defines")?,
            include_dirs: string_array(&table, "compile", "include_dirs")?,
            translation_units: string_array(&table, "sources", "translation_units")?,
            public_headers: string_array(&table, "sources", "public_headers")?,
        })
    }
}

fn string(table: &toml::Table, section: &str, key: &str) -> Result<String> {
    table
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{MANIFEST_FILE} is missing a string at {section}.{key}"))
}

fn string_array(table: &toml::Table, section: &str, key: &str) -> Result<Vec<String>> {
    let missing = || format!("{MANIFEST_FILE} is missing an array of strings at {section}.{key}");

    table
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_array)
        .ok_or_else(missing)?
        .iter()
        .map(|value| value.as_str().map(str::to_owned).ok_or_else(missing))
        .collect()
}

/// Check that the build target is supported by the recorded FFmpeg configuration.
///
/// There are two reasons to stop. Continuing would either produce a successful
/// translation with mismatched behavior or flood the output with platform header
/// errors that obscure the cause.
fn check_target(manifest: &Manifest, target: &str) -> Result<()> {
    // `FFmpeg_build/<target>/config.h` records the machine examined by `configure`,
    // including its type sizes and available functions. Reusing it for another target
    // can make FFmpeg call unavailable functions or select incorrect branches.
    //
    // The configuration is selected from the target directory, so it normally matches.
    // A mismatch can occur if another target's directory is copied under a new name.
    if manifest.triple != target {
        return Err(format!(
            "{MANIFEST_FILE} in {BUILD_CONFIG_DIR}/{target} contains a configuration for {}.\n\
             The directory name does not match the target. \
             Regenerate it by running `cargo xtask ffmpeg-manifest` on a {target} machine.",
            manifest.triple,
        ));
    }

    check_system_include_path(target)
}

/// Check that platform header locations can be supplied for the target.
///
/// Translation reads platform headers on the machine running the build script.
/// cinrs only discovers directories appropriate for that machine:
///
/// - A target different from the host: host headers have different layouts (such
///   as `struct stat` offsets), so cinrs does not discover them.
/// - Platforms other than Linux and macOS (such as Windows): no discovery rules exist.
///
/// Both cases require target directories via [`SYSTEM_PATH_ENV_VAR`]. cinrs checks
/// this during translation too, but stopping here explains the required setup in
/// terms of mini-ffmpeg-sys.
///
/// This cannot establish whether the supplied headers belong to the target.
/// It only checks that all supplied directories exist.
fn check_system_include_path(target: &str) -> Result<()> {
    // cinrs uses this variable for discovery even in native builds. Retranslate
    // when its value changes.
    println!("cargo:rerun-if-env-changed={SYSTEM_PATH_ENV_VAR}");

    let host = cargo_env("HOST")?;
    let target_os = cargo_env("CARGO_CFG_TARGET_OS")?;

    let reason = if host != target {
        format!(
            "the host is {host} and the target is {target}; host headers cannot be used for the target"
        )
    } else if !matches!(target_os.as_str(), "linux" | "macos") {
        format!("cinrs cannot discover header locations for {target_os}")
    } else {
        return Ok(());
    };

    // Follow cinrs's parsing rules: ignore empty entries; no remaining entries means unset.
    let directories: Vec<PathBuf> = env::var_os(SYSTEM_PATH_ENV_VAR)
        .iter()
        .flat_map(env::split_paths)
        .filter(|directory| !directory.as_os_str().is_empty())
        .collect();

    if directories.is_empty() {
        return Err(format!(
            "{SYSTEM_PATH_ENV_VAR} is required ({reason}).\n\
             Set {SYSTEM_PATH_ENV_VAR} to the directories containing platform headers \
             (C library headers) for {target}. Separate multiple directories with the path separator \
             (`:` on Unix, `;` on Windows)."
        ));
    }

    if let Some(missing) = directories.iter().find(|directory| !directory.is_dir()) {
        return Err(format!(
            "directory {} specified in {SYSTEM_PATH_ENV_VAR} does not exist.",
            missing.display(),
        ));
    }

    Ok(())
}

/// Check that the FFmpeg source tree exists.
///
/// The tree is a Git submodule, so cloning without `--recurse-submodules` leaves
/// an empty directory. Translating immediately would produce a cinrs "file not
/// found" error for every translation unit, obscuring the cause. Stop here first
/// and explain the required action.
///
/// Packages downloaded from crates.io include the source tree, so they pass
/// this check.
fn require_ffmpeg_tree() -> Result<()> {
    // Any file would do; use libavutil's central header as the marker.
    let marker = Path::new(FFMPEG_DIR).join("libavutil/avutil.h");

    // Rerun the build script after the submodule is fetched.
    println!("cargo:rerun-if-changed={}", marker.display());

    if marker.is_file() {
        Ok(())
    } else {
        Err(format!(
            "FFmpeg source tree is missing ({} was not found). \
             Fetch the submodule: git submodule update --init",
            marker.display(),
        ))
    }
}

/// One translation unit. For `libavutil/log.c`, the library is `libavutil` and the name is `log`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Unit {
    library: String,
    name: String,
}

impl Unit {
    /// Split a path relative to the FFmpeg source tree into a library and a name.
    ///
    /// The library and name are used directly as Rust module names. Reject paths that
    /// cannot serve as module names unchanged. None occur in the current configuration,
    /// so naming rules can be decided when needed:
    ///
    /// - Translation units in subdirectories (such as `libavcodec/aac/aacdec.c`) require
    ///   nested modules and can collide with a same-named unit (`libavcodec/aac.c`).
    /// - Names that are not Rust identifiers or that match reserved keywords.
    fn parse(path: &str) -> Result<Self> {
        let unsupported =
            |reason: &str| format!("cannot create a module for translation unit {path}: {reason}");

        let (library, file) = path
            .split_once('/')
            .ok_or_else(|| unsupported("not inside a library directory"))?;

        if file.contains('/') {
            return Err(unsupported(
                "translation units in subdirectories are not supported",
            ));
        }

        let name = file
            .strip_suffix(".c")
            .ok_or_else(|| unsupported("extension is not .c"))?;

        for identifier in [library, name] {
            if !is_plain_identifier(identifier) {
                return Err(unsupported("name is not a valid Rust identifier"));
            }

            if RUST_KEYWORDS.contains(&identifier) {
                return Err(unsupported("name is a reserved Rust keyword"));
            }
        }

        Ok(Self {
            library: library.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Path relative to the FFmpeg source tree.
    fn source_path(&self) -> String {
        format!("{}/{}.c", self.library, self.name)
    }

    /// Translation unit file, relative to the package root.
    fn source_file(&self) -> PathBuf {
        Path::new(FFMPEG_DIR).join(self.source_path())
    }

    /// Translated file, relative to `OUT_DIR`.
    fn module_file(&self) -> PathBuf {
        Path::new(&self.library).join(format!("{}.rs", self.name))
    }
}

fn is_plain_identifier(text: &str) -> bool {
    let mut characters = text.chars();

    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && characters.all(|rest| rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '_')
}

/// Rust reserved keywords. `Self` is omitted because only lowercase names are accepted.
const RUST_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// Translate all translation units, returning results in the order of `units`.
///
/// Translation units are independent, so process them in parallel using the number
/// of CPU cores. Report all failed units together to avoid retranslating everything
/// after fixing each individual error.
fn translate_units(settings: &Settings, units: &[Unit]) -> Result<Vec<Translated>> {
    let next_index = AtomicUsize::new(0);
    let worker_count = thread::available_parallelism().map_or(1, NonZero::get);

    let work = || {
        let mut finished = Vec::new();

        // Translation times vary widely, so idle threads take the next unit instead
        // of using a fixed partition of the work.
        while let Some(unit) = units.get(next_index.fetch_add(1, Ordering::Relaxed)) {
            finished.push((unit, settings.translate(&unit.source_file())));
        }

        finished
    };

    let finished = thread::scope(|scope| -> Result<Vec<_>> {
        let workers = (0..worker_count)
            .map(|_| {
                thread::Builder::new()
                    .stack_size(WORKER_STACK_SIZE)
                    .spawn_scoped(scope, work)
            })
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| format!("cannot start a translation thread: {error}"))?;

        let mut finished = Vec::new();

        for worker in workers {
            finished.extend(worker.join().map_err(|_| "translation thread panicked")?);
        }

        Ok(finished)
    })?;

    let mut translated = BTreeMap::new();
    let mut failures = BTreeMap::new();

    for (unit, result) in finished {
        match result {
            Ok(output) => {
                translated.insert(unit, output);
            }
            Err(error) => {
                failures.insert(unit, error);
            }
        }
    }

    if !failures.is_empty() {
        let mut message = format!("failed to translate {} translation units.", failures.len());

        for (unit, error) in &failures {
            let _ = write!(message, "\n\n{}:\n{error}", unit.source_path());
        }

        return Err(message);
    }

    Ok(units
        .iter()
        .map(|unit| {
            translated
                .remove(&unit)
                .expect("all translation unit results are present")
        })
        .collect())
}

/// Assemble and translate a translation unit that only includes all public headers.
///
/// cinrs accepts translation units (`.c` files and their included headers), not
/// standalone headers. Build C source consisting only of `#include` directives.
fn translate_public_api(settings: &Settings, public_headers: &[String]) -> Result<Translated> {
    let mut source = String::from(
        "/* Translation unit assembled by the mini-ffmpeg-sys build script.\n\
         * Includes public_headers from the build manifest. */\n",
    );

    for header in public_headers {
        let _ = writeln!(source, "#include \"{header}\"");
    }

    let output = settings
        .translate_public_api(PUBLIC_API_SOURCE_NAME, &source)
        .map_err(|error| format!("failed to translate public headers.\n\n{error}"))?;

    // The public API translation unit contains only declarations and types. Symbol
    // definitions would duplicate those in implementation translation units and
    // cause downstream linking failures.
    if !output.symbols.is_empty() {
        return Err(format!(
            "public header translation unit defines symbols: {:?}",
            output.symbols,
        ));
    }

    Ok(output)
}

/// Check that every FFmpeg function and variable declared by the public API is
/// defined in an implementation translation unit.
///
/// Names declared without definitions appear callable from Rust but fail to link
/// when used downstream. Building this crate cannot detect that, so check here.
///
/// Names known to lack definitions in this configuration are listed with reasons
/// in [`UNDEFINED_PUBLIC_DECLARATIONS`]. Also fail if a listed declaration disappears
/// or gains a definition, so obsolete entries do not remain.
fn check_public_declarations(public_api: &Translated, translated: &[Translated]) -> Result<()> {
    let defined: BTreeSet<&str> = translated
        .iter()
        .flat_map(|output| output.symbols.iter().map(String::as_str))
        .collect();

    let declared: BTreeSet<&str> = public_api
        .declarations
        .iter()
        .map(String::as_str)
        .filter(|name| name.starts_with(FFMPEG_PUBLIC_PREFIX))
        .collect();

    let missing: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|name| !defined.contains(name) && !UNDEFINED_PUBLIC_DECLARATIONS.contains(name))
        .collect();

    if !missing.is_empty() {
        return Err(format!(
            "{} names declared by public headers are not defined in any translation unit.\n\
             If these declarations belong to components disabled in this configuration, \
             add them to UNDEFINED_PUBLIC_DECLARATIONS with reasons.\n{missing:?}",
            missing.len(),
        ));
    }

    for name in UNDEFINED_PUBLIC_DECLARATIONS {
        if !declared.contains(name) || defined.contains(name) {
            return Err(format!(
                "{name} in UNDEFINED_PUBLIC_DECLARATIONS is no longer declared by public headers \
                 or now has a definition. Remove the entry."
            ));
        }
    }

    Ok(())
}

/// Check that no C symbol is defined by multiple translation units.
///
/// Translation units are separate Rust modules, so rustc does not reject duplicate
/// names. Duplicates are only detected when linking the final executable, not when
/// building this crate alone.
///
/// A C build does not duplicate symbols within the static library. A duplicate
/// likely means cinrs emitted a definition that should not be a symbol (such as
/// an inline function) as a symbol.
fn check_duplicate_symbols(units: &[Unit], translated: &[Translated]) -> Result<()> {
    let mut owners: BTreeMap<&str, &Unit> = BTreeMap::new();

    for (unit, output) in units.iter().zip(translated) {
        for symbol in &output.symbols {
            if let Some(first) = owners.insert(symbol, unit) {
                return Err(format!(
                    "symbol {symbol} is defined by both {} and {}.",
                    first.source_path(),
                    unit.source_path(),
                ));
            }
        }
    }

    Ok(())
}

/// Assemble a file for one translation unit, adding documentation to cinrs's output.
fn render_unit(unit: &Unit, rust: &str) -> String {
    format!(
        "//! FFmpeg `{}` translated to Rust with cinrs.\n\n{rust}\n",
        unit.source_path(),
    )
}

/// Assemble the translated public header file, adding documentation to cinrs's output.
fn render_public_api(rust: &str) -> String {
    format!(
        "//! FFmpeg public headers (`public_headers` in the build manifest) translated\n\
         //! to Rust with cinrs.\n\
         //!\n\
         //! Functions are declarations only; their definitions reside in translation\n\
         //! units under library modules. They link through C symbol names.\n\
         \n\
         {rust}\n"
    )
}

/// Assemble module declarations. Declare each translation unit inside its library's
/// module so `libavutil/time.c` is accessible as `libavutil::time`.
///
/// Translation unit files use absolute paths in `#[path]`. A `mod time;` inside an
/// `include!`d file is resolved relative to the including file (`src/lib.rs`), not
/// `OUT_DIR`, so omitting the path would fail to locate it. Only this file within
/// `OUT_DIR` contains absolute paths; it is not distributed.
fn render_root(out_dir: &Path, units: &[Unit]) -> Result<String> {
    let mut by_library: BTreeMap<&str, Vec<&Unit>> = BTreeMap::new();

    for unit in units {
        by_library.entry(&unit.library).or_default().push(unit);
    }

    let path_literal = |file: PathBuf| -> Result<String> {
        let file = file.to_str().ok_or_else(|| {
            format!(
                "output path {} cannot be represented as a UTF-8 string",
                file.display()
            )
        })?;

        // `{:?}` renders the path as a Rust string literal, escaping characters such
        // as `\` in Windows paths.
        Ok(format!("{file:?}"))
    };

    let mut out = String::new();

    // Re-export the public API at the crate root so users can call it as
    // `mini_ffmpeg_sys::av_log`.
    let _ = writeln!(
        out,
        "/// Translated FFmpeg public headers, re-exported at the crate root.\n\
         #[path = {}]\n\
         mod public_api;\n\
         \n\
         pub use public_api::*;\n",
        path_literal(out_dir.join(PUBLIC_API_FILE))?,
    );

    for (library, units) in by_library {
        // Implementation translation units are not intended for users, so hide them
        // from documentation. Their `#[no_mangle]` symbols are emitted regardless of
        // module visibility and remain accessible through public API declarations.
        let _ = writeln!(
            out,
            "/// {library} translation units, with one module per FFmpeg `.c` file.\n\
             #[doc(hidden)]\n\
             pub mod {library} {{",
        );

        for unit in units {
            let file = path_literal(out_dir.join(unit.module_file()))?;

            let _ = writeln!(out, "    #[path = {file}]\n    pub mod {};", unit.name);
        }

        out.push_str("}\n\n");
    }

    Ok(out)
}

/// Write `content` to `path`, creating parent directories if needed.
fn write_file(path: &Path, content: &str) -> Result<()> {
    let failed = |error| format!("cannot write to {}: {error}", path.display());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }

    fs::write(path, content).map_err(failed)
}
