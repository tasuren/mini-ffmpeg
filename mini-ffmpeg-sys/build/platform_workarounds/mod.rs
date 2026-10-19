//! Target-specific exceptions for unsupported platform declarations.
//!
//! A replacement is only a parsing aid, never an ABI conversion. Before generating
//! Rust, inspect C tokens and their macro origins: only approved declarations may use
//! the unsupported type. Afterwards, remove their Rust declarations from the
//! public API and reject any remaining references. Implementation units must not
//! retain even a declaration of these functions after reachability filtering.

mod linux_aarch64_glibc;
mod macos;
mod model;

pub use model::WorkaroundSet;

use cinrs_core::{CommandLineMacro, FilePreprocessing, Options, include::HeaderKind, pp::Token};
use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use std::path::Path;

const SETS: &[WorkaroundSet] = &[macos::SET, linux_aarch64_glibc::SET];
const NONE: WorkaroundSet = WorkaroundSet {
    name: "no platform workaround",
    matches_target: |_| false,
    replacements: &[],
    headers: &[],
    is_sensitive_symbol: |_| false,
    accepts_declaration: |_| false,
    excludes_function: |_| false,
};

impl WorkaroundSet {
    /// Selects one policy by target ABI; unknown targets receive an inactive set.
    pub fn for_target(target: &str) -> Self {
        SETS.iter()
            .copied()
            .find(|set| (set.matches_target)(target))
            .unwrap_or(NONE)
    }

    /// Whether this set requires a pass over preprocessed C before translation.
    pub fn needs_validation(self) -> bool {
        !self.headers.is_empty()
    }

    /// Adds parsing aids whose original types are recovered by validation.
    pub fn apply(self, options: &mut Options) {
        for rule in self.replacements {
            options.macros.push(CommandLineMacro::Define(format!(
                "{}={}",
                rule.original, rule.replacement
            )));
        }
    }

    /// Checks active C tokens using original types from macro origins; skipped branches
    /// do not count. Unknown uses, changed signatures, and references to excluded
    /// functions fail closed, including references in unused inline functions.
    ///
    /// This deliberately recognizes only standalone SDK prototypes, not arbitrary
    /// C declarations. A new SDK spelling requires review rather than guesswork.
    pub fn validate(self, source: &FilePreprocessing) -> Result<(), String> {
        if !self.needs_validation() {
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
            match self.c_spelling(token) {
                "{" => braces += 1,
                "}" => braces = braces.saturating_sub(1),
                "(" => parentheses += 1,
                ")" => parentheses = parentheses.saturating_sub(1),
                _ => {}
            }

            let boundary =
                braces == 0 && parentheses == 0 && matches!(self.c_spelling(token), ";" | "}" | "");

            if !boundary {
                continue;
            }

            let declaration = &source.tokens[start..=index];
            start = index + 1;

            if let Some(sensitive) = declaration.iter().find(|token| {
                let name = self.c_spelling(token);
                self.replacements.iter().any(|rule| rule.original == name)
                    || self.excluded(name)
                    || (self.is_sensitive_symbol)(name)
            }) {
                let file = source.map.file(source.map.file_of(sensitive.range.start));
                let sdk_header = source.headers.iter().any(|header| {
                    header.file == file.id()
                        && matches!(header.kind, HeaderKind::System)
                        && Path::new(&header.name)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| self.headers.contains(&name))
                });

                let words: Vec<_> = declaration
                    .iter()
                    .map(|token| self.c_spelling(token))
                    .collect();
                let known = (self.accepts_declaration)(&words);

                if !sdk_header || !known {
                    return Err(format!(
                        "{}: unsupported type use or excluded symbol reference in {} at offset {}: {}",
                        self.name,
                        file.name(),
                        sensitive.range.start - file.base(),
                        declaration
                            .iter()
                            .map(|token| self.c_spelling(token))
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
        if !self.needs_validation() {
            return Ok(tokens);
        }

        self.filter_groups(tokens)
    }

    fn excluded(self, name: &str) -> bool {
        self.excluded_alias(name) || (self.excludes_function)(name)
    }

    fn excluded_alias(self, name: &str) -> bool {
        self.replacements
            .iter()
            .any(|rule| rule.alias == Some(name))
    }

    /// Restore only this policy's substituted types through nested expansions.
    /// Using the same macros for validation and translation preserves #ifdef
    /// selection while retaining the original type's identity.
    fn c_spelling(self, token: &Token) -> &str {
        let mut expansion = token.origin.expansion();

        while let Some(origin) = expansion {
            if self
                .replacements
                .iter()
                .any(|rule| origin.name == rule.original)
            {
                return &origin.name;
            }

            expansion = origin.parent.as_ref();
        }

        token.kind.spelling()
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

        Ok(self.remove_aliases(out))
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
        if !self.needs_validation() {
            return Ok(());
        }

        for token in tokens {
            match token {
                TokenTree::Ident(name) if self.excluded(&name.to_string()) => {
                    return Err(format!(
                        "generated code still requires excluded platform symbol {name}"
                    ));
                }
                TokenTree::Group(group) => {
                    if link_name(group.stream())
                        .as_deref()
                        .is_some_and(|name| self.excluded(name))
                    {
                        return Err(
                            "generated code still links to an excluded platform symbol".into()
                        );
                    }

                    self.check_references(group.stream())?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Drop whole generated typedef items, including attributes, after their C
    /// declarations have been validated. References in retained items are rejected
    /// by check_references, just like references to the excluded vector functions.
    fn remove_aliases(self, tokens: TokenStream) -> TokenStream {
        let mut out = TokenStream::new();
        let mut item = Vec::new();

        for token in tokens {
            let boundary = matches!(&token, TokenTree::Punct(punct) if punct.as_char() == ';')
                || matches!(&token, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace);
            item.push(token);

            if boundary {
                let alias = item.windows(2).any(|pair| {
                    matches!(pair,
                    [TokenTree::Ident(keyword), TokenTree::Ident(name)]
                        if keyword == "type" && self.excluded_alias(&name.to_string()))
                });

                if !alias {
                    out.extend(item.drain(..));
                } else {
                    item.clear();
                }
            }
        }

        out.extend(item);
        out
    }
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
