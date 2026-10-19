//! Read C compile commands from the commands printed by `make -n V=1`.
//!
//! FFmpeg's `ffbuild/common.mak` constructs compile commands in this form:
//!
//! ```make
//! $(CC) $(CCFLAGS) $(CC_DEPFLAGS) $(CC_C) $(CC_O) <source>
//! ```
//!
//! `CC_C` is `-c` and `CC_O` is `-o <object>`, so commands always end with
//! `... -c -o foo.o src/foo.c`. This parser assumes that form to extract the object,
//! source, and flags that affect preprocessing.
//!
//! For MSVC (`--toolchain=msvc`), `configure` sets `CC_O` to `-Fo<object>` and the
//! C standard option to `/std:c17`. The object path is not a separate word in this
//! form, which the parser also supports.

use std::path::Path;

use crate::Result;

/// A command that compiles one translation unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileCommand {
    /// The output object file, relative to the build directory.
    pub object: String,

    /// The source to compile, with its path as written in the command.
    pub source: String,

    pub flags: CompileFlags,
}

/// Compile command flags that affect preprocessing.
///
/// Only the flags needed to reproduce the translation unit in cinrs are retained.
/// Optimization and warning flags are deliberately discarded because cinrs has no
/// equivalent options.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompileFlags {
    /// The value of `-std=` (`/std:` for MSVC).
    pub std: Option<String>,

    /// The values of `-D`, as `NAME` or `NAME=VALUE`, in command order.
    pub defines: Vec<String>,

    /// The values of `-I`, with paths as written in the command, in command order.
    /// Do not sort them: include search order matters.
    pub include_dirs: Vec<String>,
}

/// Flags that affect preprocessing but are not yet supported.
///
/// Silently discarding these flags could make cinrs translate different code from
/// the original build without detection. Reject them so support can be added when
/// needed.
const UNSUPPORTED_FLAGS: &[&str] = &[
    "-U",
    "-include",
    "-imacros",
    "-isystem",
    "-iquote",
    "-idirafter",
    // MSVC forced includes, equivalent to `-include`.
    "-FI",
    "/FI",
];

/// Parse one command line and return its contents if it is a C compile command.
///
/// Return `Ok(None)` for other commands, such as `ar` or scripts.
///
/// Reject compile commands that cannot reproduce the original build when translated
/// as-is: those compiling non-C sources (such as assembly) or using
/// [`UNSUPPORTED_FLAGS`].
pub fn parse_line(line: &str) -> Result<Option<CompileCommand>> {
    // Ignore lines with unterminated quotes, which cannot be parsed as commands.
    // If a compile command is missed, the check against the object files after the
    // build will detect it.
    let Some(words) = split_shell_words(line) else {
        return Ok(None);
    };

    let Some(object) = find_object(&words) else {
        return Ok(None);
    };

    let source = words
        .last()
        .expect("an object implies a nonempty word list");

    if Path::new(source).extension().is_none_or(|ext| ext != "c") {
        return Err(format!(
            "a non-C source is being compiled: {source}\n\
             cinrs can only translate C; use a configuration without assembly or other languages."
        )
        .into());
    }

    Ok(Some(CompileCommand {
        object: object.to_owned(),
        source: source.clone(),
        flags: parse_flags(&words)?,
    }))
}

/// Return the object path for a compile command (one containing `-c` and either
/// `-o <name>.o` or `-Fo<name>.o`).
fn find_object(words: &[String]) -> Option<&str> {
    if !words.iter().any(|word| word == "-c") {
        return None;
    }

    let gcc_style = words
        .windows(2)
        .find(|pair| pair[0] == "-o" && pair[1].ends_with(".o"))
        .map(|pair| pair[1].as_str());

    gcc_style.or_else(|| {
        words
            .iter()
            .find_map(|word| word.strip_prefix("-Fo"))
            .filter(|object| object.ends_with(".o"))
    })
}

fn parse_flags(words: &[String]) -> Result<CompileFlags> {
    let mut flags = CompileFlags::default();
    let mut words = words.iter();

    while let Some(word) = words.next() {
        if let Some(unsupported) = UNSUPPORTED_FLAGS
            .iter()
            .find(|flag| word.starts_with(**flag))
        {
            return Err(format!(
                "the compile command contains unsupported flag {unsupported}: {word}"
            )
            .into());
        }

        if let Some(std) = word
            .strip_prefix("-std=")
            .or_else(|| word.strip_prefix("/std:"))
        {
            flags.std = Some(std.to_owned());
        } else if let Some(value) = flag_value(word, "-D", &mut words)? {
            push_unique(&mut flags.defines, value);
        } else if let Some(value) = flag_value(word, "-I", &mut words)? {
            push_unique(&mut flags.include_dirs, value);
        }
    }

    Ok(flags)
}

/// Extract a flag value from either an attached form (`-Dfoo`) or a separate word
/// (`-D foo`). Return `Ok(None)` if `word` does not start with `flag`.
fn flag_value<'a>(
    word: &'a str,
    flag: &str,
    rest: &mut impl Iterator<Item = &'a String>,
) -> Result<Option<String>> {
    let Some(joined) = word.strip_prefix(flag) else {
        return Ok(None);
    };

    if !joined.is_empty() {
        return Ok(Some(joined.to_owned()));
    }

    match rest.next() {
        Some(value) => Ok(Some(value.clone())),
        None => Err(format!("missing value for {flag}").into()),
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

/// Split one command line into words using shell rules.
///
/// Only quotes and backslashes as used in commands printed by `make` are handled;
/// variable expansion and other shell features are not interpreted. Return `None`
/// for lines that cannot be split, such as those with unterminated quotes.
pub(super) fn split_shell_words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();

    // Empty quotes (`""`) also form a word, so track whether a word is in progress
    // separately from whether the string is empty.
    let mut in_word = false;
    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '\'' => {
                in_word = true;

                loop {
                    match characters.next()? {
                        '\'' => break,
                        quoted => current.push(quoted),
                    }
                }
            }

            '"' => {
                in_word = true;

                loop {
                    match characters.next()? {
                        '"' => break,

                        // Inside double quotes, `\` only escapes certain characters.
                        '\\' if matches!(characters.peek(), Some('"' | '\\' | '$' | '`')) => {
                            current.push(characters.next()?);
                        }

                        quoted => current.push(quoted),
                    }
                }
            }

            '\\' => {
                in_word = true;
                current.push(characters.next()?);
            }

            character if character.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }

            character => {
                in_word = true;
                current.push(character);
            }
        }
    }

    if in_word {
        words.push(current);
    }

    Some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An actual command from building FFmpeg `n9.0.2` on macOS, with warning flags
    /// removed.
    const REAL_COMMAND: &str = "gcc -I. -Isrc/ -D_ISOC11_SOURCE -D_FILE_OFFSET_BITS=64 \
        -D_LARGEFILE_SOURCE -I/work/FFmpeg/compat/dispatch_semaphore -DPIC \
        -I/work/FFmpeg/compat/stdbit -DHAVE_AV_CONFIG_H -std=c17 -fomit-frame-pointer \
        -fPIC -g -O3 -fno-math-errno -MMD -MF libavutil/log.d -MT libavutil/log.o \
        -c -o libavutil/log.o src/libavutil/log.c";

    #[test]
    fn parses_real_compile_command() {
        let command = parse_line(REAL_COMMAND).unwrap().unwrap();

        assert_eq!(command.object, "libavutil/log.o");
        assert_eq!(command.source, "src/libavutil/log.c");
        assert_eq!(command.flags.std.as_deref(), Some("c17"));
        assert_eq!(
            command.flags.defines,
            [
                "_ISOC11_SOURCE",
                "_FILE_OFFSET_BITS=64",
                "_LARGEFILE_SOURCE",
                "PIC",
                "HAVE_AV_CONFIG_H",
            ],
        );
        assert_eq!(
            command.flags.include_dirs,
            [
                ".",
                "src/",
                "/work/FFmpeg/compat/dispatch_semaphore",
                "/work/FFmpeg/compat/stdbit",
            ],
        );
    }

    /// An actual command from configuring FFmpeg `n9.0.2` on Windows with
    /// `--toolchain=msvc`, with warning flags removed and paths shortened.
    const REAL_MSVC_COMMAND: &str = "cl.exe -I. -I/work/FFmpeg/ -D_ISOC11_SOURCE \
        -DWIN32_LEAN_AND_MEAN -D_WIN32_WINNT=0x0600 -I/work/FFmpeg/compat/atomics/win32 \
        -DHAVE_AV_CONFIG_H -nologo -options:strict  /std:c17  -Z7 -O2   -utf-8    \
        -c -Folibavutil/log.o /work/FFmpeg/libavutil/log.c";

    #[test]
    fn parses_real_msvc_compile_command() {
        let command = parse_line(REAL_MSVC_COMMAND).unwrap().unwrap();

        assert_eq!(command.object, "libavutil/log.o");
        assert_eq!(command.source, "/work/FFmpeg/libavutil/log.c");
        assert_eq!(command.flags.std.as_deref(), Some("c17"));
        assert_eq!(
            command.flags.defines,
            [
                "_ISOC11_SOURCE",
                "WIN32_LEAN_AND_MEAN",
                "_WIN32_WINNT=0x0600",
                "HAVE_AV_CONFIG_H",
            ],
        );
        assert_eq!(
            command.flags.include_dirs,
            [".", "/work/FFmpeg/", "/work/FFmpeg/compat/atomics/win32"],
        );
    }

    /// MSVC creates dependency files with a preprocessing-only invocation using
    /// `-showIncludes -Zs`. Do not count it as a translation unit: it does not compile.
    #[test]
    fn ignores_msvc_dependency_command() {
        let line = "cl.exe -I. -DHAVE_AV_CONFIG_H -showIncludes -Zs -nologo /std:c17 \
            /work/FFmpeg/libavutil/log.c 2>&1 | awk '/including/ { print \"libavutil/log.o:\", $0 }' \
            > libavutil/log.d";

        assert_eq!(parse_line(line).unwrap(), None);
    }

    /// `make -n` also prints non-compile commands, such as archive creation and script
    /// execution. Do not count them as translation units.
    #[test]
    fn ignores_commands_that_do_not_compile() {
        let lines = [
            "ar rc libavutil/libavutil.a libavutil/log.o libavutil/mem.o",
            "ranlib -D libavutil/libavutil.a",
            "/work/FFmpeg/ffbuild/pkgconfig_generate.sh avutil \"FFmpeg utility library\"",
            "gcc -o ffmpeg fftools/ffmpeg.o -lm",
            "",
        ];

        for line in lines {
            assert_eq!(parse_line(line).unwrap(), None, "{line}");
        }
    }

    /// A configuration that leaves assembly enabled compiles `.S` and similar files.
    /// cinrs cannot translate them, so reject them rather than silently dropping them
    /// from cinrs_manifest.toml. Otherwise, the issue would only surface as undefined
    /// symbols at link time.
    #[test]
    fn rejects_non_c_source() {
        let line = "gcc -I. -c -o libavutil/aarch64/cpu.o src/libavutil/aarch64/asm.S";

        assert!(parse_line(line).is_err());
    }

    /// Forced includes and similar flags change translation unit contents. Ignoring
    /// unsupported flags when writing cinrs_manifest.toml would make cinrs translate
    /// code that differs from the original build.
    #[test]
    fn rejects_unsupported_preprocessor_flags() {
        for flag in [
            "-include config.h",
            "-UPIC",
            "-isystem /opt/include",
            "-FIconfig.h",
            "/FIconfig.h",
        ] {
            let line = format!("gcc {flag} -c -o a.o src/a.c");

            assert!(parse_line(&line).is_err(), "{flag}");
        }
    }

    #[test]
    fn accepts_flag_value_in_next_word() {
        let command = parse_line("gcc -D FOO=1 -I include -c -o a.o src/a.c")
            .unwrap()
            .unwrap();

        assert_eq!(command.flags.defines, ["FOO=1"]);
        assert_eq!(command.flags.include_dirs, ["include"]);
    }

    /// Deduplicate repeated flags while preserving their first occurrence so the
    /// include search order does not change.
    #[test]
    fn deduplicates_flags_keeping_first_position() {
        let command = parse_line("gcc -Ia -Ib -Ia -DX -DX -c -o a.o src/a.c")
            .unwrap()
            .unwrap();

        assert_eq!(command.flags.include_dirs, ["a", "b"]);
        assert_eq!(command.flags.defines, ["X"]);
    }

    #[test]
    fn splits_quoted_words() {
        assert_eq!(
            split_shell_words(r#"a "b c" 'd e' f\ g -DX="y z" """#).unwrap(),
            ["a", "b c", "d e", "f g", "-DX=y z", ""],
        );
    }

    #[test]
    fn returns_none_for_unterminated_quote() {
        assert_eq!(split_shell_words("gcc \"unterminated"), None);
    }
}
