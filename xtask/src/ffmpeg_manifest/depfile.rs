//! Read dependency files (`.d`) written by the compiler.
//!
//! FFmpeg compiles with `-MMD -MF foo.d`, so each translation unit leaves a Makefile
//! rule recording which files were included by its `.o`:
//!
//! ```text
//! libavformat/wavdec.o: \
//!   src/libavformat/wavdec.c config_components.h src/libavutil/avassert.h \
//!   config.h
//! ```
//!
//! `-MMD` excludes system headers, so the files listed are generally project files.

use crate::Result;

/// Extract dependency file paths from the contents of a dependency file.
///
/// Return paths on the right of `:` in their original order, without deduplicating
/// or interpreting them.
///
/// Reject lines that are not rules. Silently skipping them could leave dependencies
/// missing from the build manifest without detection.
pub fn parse_dependencies(content: &str) -> Result<Vec<String>> {
    // A trailing `\` continues a line. Join continuations before parsing each logical line as a rule.
    let joined = content.replace("\\\r\n", " ").replace("\\\n", " ");

    let mut dependencies = Vec::new();

    for line in joined.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let Some(separator) = find_rule_separator(line) else {
            return Err(format!("cannot parse line as a dependency rule: {line}").into());
        };

        dependencies.extend(split_paths(&line[separator + 1..]));
    }

    Ok(dependencies)
}

/// Find the `:` separating a rule's target from its dependencies.
///
/// The separator is followed by whitespace or the end of the line. This condition
/// distinguishes it from a drive-letter colon, such as the one in `C:\foo`.
fn find_rule_separator(line: &str) -> Option<usize> {
    line.char_indices().find_map(|(index, character)| {
        let next = line[index + character.len_utf8()..].chars().next();

        (character == ':' && next.is_none_or(char::is_whitespace)).then_some(index)
    })
}

/// Split whitespace-separated paths while unescaping Makefile escapes.
///
/// Compilers write spaces in paths as `\ `, `#` as `\#`, and `$` as `$$`.
/// Other backslashes remain literal to preserve Windows path separators.
fn split_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut current = String::new();
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '\\' if matches!(characters.peek(), Some(' ' | '#')) => {
                current.extend(characters.next());
            }

            '$' if characters.peek() == Some(&'$') => {
                characters.next();
                current.push('$');
            }

            character if character.is_whitespace() => {
                if !current.is_empty() {
                    paths.push(std::mem::take(&mut current));
                }
            }

            character => current.push(character),
        }
    }

    if !current.is_empty() {
        paths.push(current);
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse actual clang output, with a newline after the target and dependencies
    /// continued over multiple lines.
    #[test]
    fn parses_continued_rule() {
        let content = "libavformat/wavdec.o: \\\n  src/libavformat/wavdec.c config_components.h \\\n  config.h\n";

        assert_eq!(
            parse_dependencies(content).unwrap(),
            [
                "src/libavformat/wavdec.c",
                "config_components.h",
                "config.h"
            ],
        );
    }

    /// Compilers escape spaces in paths as `\ `. Splitting there would turn one path
    /// into two nonexistent paths. This matters because dependencies can include
    /// paths outside FFmpeg, such as Xcode SDK paths.
    #[test]
    fn keeps_escaped_spaces_in_path() {
        let content = "a.o: /Applications/My\\ Xcode.app/SDKSettings.json src/a.c\n";

        assert_eq!(
            parse_dependencies(content).unwrap(),
            ["/Applications/My Xcode.app/SDKSettings.json", "src/a.c"],
        );
    }

    #[test]
    fn unescapes_hash_and_dollar() {
        let content = "a.o: dir\\#1/a.h dir$$2/b.h\n";

        assert_eq!(
            parse_dependencies(content).unwrap(),
            ["dir#1/a.h", "dir$2/b.h"],
        );
    }

    /// Do not mistake a Windows drive-letter colon for the target/dependency separator.
    /// Otherwise, the rest of the target would appear among the dependencies.
    #[test]
    fn does_not_split_at_drive_letter_colon() {
        let content = "C:\\build\\a.o: C:\\src\\a.c\n";

        assert_eq!(parse_dependencies(content).unwrap(), ["C:\\src\\a.c"]);
    }

    /// With `-MP`, each header gets a rule without dependencies (just `foo.h:`).
    /// Skip these without adding any dependencies.
    #[test]
    fn accepts_rule_without_dependencies() {
        let content = "a.o: src/a.c src/a.h\nsrc/a.h:\n";

        assert_eq!(parse_dependencies(content).unwrap(), ["src/a.c", "src/a.h"]);
    }

    #[test]
    fn rejects_line_that_is_not_a_rule() {
        assert!(parse_dependencies("this is not a rule\n").is_err());
    }
}
