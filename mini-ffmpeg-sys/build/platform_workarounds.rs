//! Target-specific exceptions for unsupported platform declarations.
//!
//! A replacement is only a parsing aid, never an ABI conversion. Before using it,
//! inspect preprocessed C and its macro origins: only the declarations here may use
//! the unsupported type. Afterwards, remove their Rust declarations from the
//! public API and reject any remaining references. Implementation units must not
//! retain even a declaration of these functions after reachability filtering.

use std::path::Path;

use cinrs_core::{CommandLineMacro, FilePreprocessing, Options, include::HeaderKind, pp::Token};
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};

/// One unsupported function's original signature and excluded symbol.
struct Declaration {
    name: &'static str,
    arguments: usize,
}

/// macOS SDK math.h declares these functions with binary16 arguments and results.
/// Keep the signature and exclusion together so accepting a declaration cannot
/// accidentally expose its substituted binary32 ABI.
const HALF_FUNCTIONS: &[Declaration] = &[
    Declaration {
        name: "__fabsf16",
        arguments: 1,
    },
    Declaration {
        name: "__hypotf16",
        arguments: 2,
    },
    Declaration {
        name: "__sqrtf16",
        arguments: 1,
    },
    Declaration {
        name: "__ceilf16",
        arguments: 1,
    },
    Declaration {
        name: "__floorf16",
        arguments: 1,
    },
    Declaration {
        name: "__rintf16",
        arguments: 1,
    },
    Declaration {
        name: "__roundf16",
        arguments: 1,
    },
    Declaration {
        name: "__truncf16",
        arguments: 1,
    },
    Declaration {
        name: "__copysignf16",
        arguments: 2,
    },
    Declaration {
        name: "__nextafterf16",
        arguments: 2,
    },
    Declaration {
        name: "__fmaxf16",
        arguments: 2,
    },
    Declaration {
        name: "__fminf16",
        arguments: 2,
    },
    Declaration {
        name: "__fmaf16",
        arguments: 3,
    },
];

/// A target's parsing replacements, validation rules, and output exclusions.
#[derive(Clone, Copy)]
pub struct Workarounds {
    macos_half: bool,
}

impl Workarounds {
    pub fn for_target(target: &str) -> Self {
        Self {
            macos_half: target.ends_with("-apple-darwin"),
        }
    }

    /// Whether this target needs a separate pass over the original C tokens.
    pub fn needs_validation(self) -> bool {
        self.macos_half
    }

    /// Adds parsing replacements. Validation follows their macro origins to
    /// recover the original type without changing conditional preprocessing.
    pub fn apply(self, options: &mut Options) {
        if self.macos_half {
            options
                .macros
                .push(CommandLineMacro::Define("_Float16=float".into()));
        }
    }

    /// Checks active C tokens using original types from macro origins; skipped branches
    /// do not count. Unknown uses, changed signatures, and references to excluded
    /// functions fail closed, including references in unused inline functions.
    ///
    /// This deliberately recognizes only standalone SDK prototypes, not arbitrary
    /// C declarations. A new SDK spelling requires review rather than guesswork.
    pub fn validate(self, source: &FilePreprocessing) -> Result<(), String> {
        if !self.macos_half {
            return Ok(());
        }

        if source.diagnostics.has_errors() {
            let mut errors: Vec<_> = source
                .diagnostics
                .sorted()
                .into_iter()
                .map(|diagnostic| source.diagnostics.render(&source.map, diagnostic))
                .collect();

            if let Some(message) = source.diagnostics.fatal_message() {
                errors.push(message.to_owned());
            }

            return Err(errors.join("\n"));
        }

        // Split only at top-level boundaries. Semicolons inside function bodies
        // and records must never turn a field or local into an allowed prototype.
        let mut start = 0;
        let mut braces = 0usize;
        let mut parentheses = 0usize;

        for (index, token) in source.tokens.iter().enumerate() {
            match c_spelling(token) {
                "{" => braces += 1,
                "}" => braces = braces.saturating_sub(1),
                "(" => parentheses += 1,
                ")" => parentheses = parentheses.saturating_sub(1),
                _ => {}
            }

            let boundary =
                braces == 0 && parentheses == 0 && matches!(c_spelling(token), ";" | "}" | "");

            if !boundary {
                continue;
            }

            let declaration = &source.tokens[start..=index];
            start = index + 1;

            if let Some(sensitive) = declaration
                .iter()
                .find(|token| c_spelling(token) == "_Float16" || self.excluded(c_spelling(token)))
            {
                let file = source.map.file(source.map.file_of(sensitive.range.start));
                let sdk_math = source.headers.iter().any(|header| {
                    header.file == file.id()
                        && matches!(header.kind, HeaderKind::System)
                        && Path::new(&header.name)
                            .file_name()
                            .is_some_and(|name| name == "math.h")
                });

                if !sdk_math || !known_prototype(declaration) {
                    return Err(format!(
                        "unsupported macOS _Float16 use or excluded function reference in {} at offset {}: {}",
                        file.name(),
                        sensitive.range.start - file.base(),
                        declaration
                            .iter()
                            .map(|token| c_spelling(token))
                            .collect::<Vec<_>>()
                            .join(" "),
                    ));
                }
            }
        }

        Ok(())
    }

    /// Removes excluded foreign declarations and their attributes from the public
    /// API. Call `check_references` afterwards to reject any remaining callers.
    pub fn exclude_declarations(self, tokens: TokenStream) -> Result<TokenStream, String> {
        if !self.macos_half {
            return Ok(tokens);
        }

        self.filter_groups(tokens)
    }

    fn excluded(self, name: &str) -> bool {
        self.macos_half
            && HALF_FUNCTIONS
                .iter()
                .any(|declaration| declaration.name == name)
    }

    fn filter_groups(self, tokens: TokenStream) -> Result<TokenStream, String> {
        let tokens: Vec<_> = tokens.into_iter().collect();
        let mut out = TokenStream::new();

        for (index, token) in tokens.iter().enumerate() {
            if let TokenTree::Group(group) = token {
                let foreign = group.delimiter() == Delimiter::Brace
                    && index >= 2
                    && matches!(&tokens[index - 2], TokenTree::Ident(word) if word == "extern")
                    && matches!(&tokens[index - 1], TokenTree::Literal(abi) if abi.to_string() == "\"C\"");

                let contents = if foreign {
                    self.filter_foreign_items(group.stream())?
                } else {
                    self.filter_groups(group.stream())?
                };

                let mut replacement = Group::new(group.delimiter(), contents);
                replacement.set_span(group.span());

                out.extend([TokenTree::Group(replacement)]);
            } else {
                out.extend([token.clone()]);
            }
        }

        Ok(out)
    }

    fn filter_foreign_items(self, tokens: TokenStream) -> Result<TokenStream, String> {
        let mut out = TokenStream::new();
        let mut item = TokenStream::new();

        for token in tokens {
            let end = matches!(&token, TokenTree::Punct(punct) if punct.as_char() == ';');
            item.extend([token]);

            if end {
                let parts: Vec<_> = item.clone().into_iter().collect();
                let function = parts.windows(2).find_map(|pair| match pair {
                    [TokenTree::Ident(keyword), TokenTree::Ident(name)] if keyword == "fn" => {
                        Some(name.to_string())
                    }
                    _ => None,
                });
                let link_name = parts.iter().find_map(|token| match token {
                    TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket => {
                        link_name(group.stream())
                    }
                    _ => None,
                });

                // Match both the Rust function name and the C symbol specified by
                // #[link_name], since cinrs may give them different names.
                let excluded = function.as_deref().is_some_and(|name| self.excluded(name))
                    || link_name.as_deref().is_some_and(|name| self.excluded(name));

                if !excluded {
                    out.extend(item);
                }

                item = TokenStream::new();
            }
        }

        // cinrs currently emits semicolon-terminated foreign items. Refuse a new
        // output shape rather than silently dropping an unfinished item.
        if !item.is_empty() {
            return Err("unexpected unterminated foreign item from cinrs".into());
        }

        Ok(out)
    }

    /// Checks generated code without modifying it. Implementation units must not
    /// depend on excluded functions; public API units must pass this check after
    /// declaration removal so inline callers cannot retain an incorrect ABI.
    pub fn check_references(self, tokens: TokenStream) -> Result<(), String> {
        if !self.macos_half {
            return Ok(());
        }

        for token in tokens {
            match token {
                TokenTree::Ident(name) if self.excluded(&name.to_string()) => {
                    return Err(format!(
                        "generated code still requires excluded macOS function {name}"
                    ));
                }
                TokenTree::Group(group) => {
                    if link_name(group.stream())
                        .as_deref()
                        .is_some_and(|name| self.excluded(name))
                    {
                        return Err(
                            "generated code still links to an excluded macOS function".into()
                        );
                    }

                    self.check_references(group.stream())?;
                }
                _ => {}
            }
        }

        Ok(())
    }
}

/// Recover the replaced type even through nested macro expansions. Inspect the
/// same preprocessor configuration used for translation: removing the macro for
/// this pass could select a different #ifdef branch and miss a use entirely.
fn c_spelling(token: &Token) -> &str {
    let mut expansion = token.origin.expansion();

    while let Some(origin) = expansion {
        if origin.name == "_Float16" {
            return "_Float16";
        }

        expansion = origin.parent.as_ref();
    }

    token.kind.spelling()
}

/// Matches the exact binary16 signature, allowing trailing GNU attributes such
/// as SDK availability metadata. Unsupported types inside attributes still fail.
fn known_prototype(tokens: &[Token]) -> bool {
    let words: Vec<_> = tokens.iter().map(|token| c_spelling(token)).collect();

    HALF_FUNCTIONS.iter().any(|declaration| {
        let mut expected = vec!["extern", "_Float16", declaration.name, "("];

        for argument in 0..declaration.arguments {
            if argument != 0 {
                expected.push(",");
            }

            expected.push("_Float16");
        }

        expected.push(")");

        if !words.starts_with(&expected) {
            return false;
        }

        let mut rest = &words[expected.len()..];

        // Skip trailing GNU attributes, such as SDK availability metadata, while
        // checking for unsupported types and references to excluded functions.
        // Attribute contents are otherwise not validated.
        while rest.first() == Some(&"__attribute__") {
            if rest.get(1) != Some(&"(") || rest.get(2) != Some(&"(") {
                return false;
            }

            let mut depth = 0usize;
            let mut end = None;

            for (index, word) in rest.iter().enumerate().skip(1) {
                if *word == "_Float16" || HALF_FUNCTIONS.iter().any(|item| item.name == *word) {
                    return false;
                }

                match *word {
                    "(" => depth += 1,
                    ")" => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            end = Some(index + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }

            let Some(end) = end else {
                return false;
            };
            rest = &rest[end..];
        }

        rest == [";"]
    })
}

fn link_name(tokens: TokenStream) -> Option<String> {
    let tokens: Vec<_> = tokens.into_iter().collect();

    match tokens.as_slice() {
        [
            TokenTree::Ident(key),
            TokenTree::Punct(equals),
            TokenTree::Literal(value),
        ] if key == "link_name" && equals.as_char() == '=' => value
            .to_string()
            .strip_prefix('"')?
            .strip_suffix('"')
            .map(str::to_owned),
        [TokenTree::Ident(word), TokenTree::Group(inner)] if word == "unsafe" => {
            link_name(inner.stream())
        }
        _ => None,
    }
}
