//! `make -n V=1` が表示するコマンドから、C のコンパイルコマンドを読み取る。
//!
//! FFmpeg の `ffbuild/common.mak` は、コンパイルコマンドを次の形で組み立てる。
//!
//! ```make
//! $(CC) $(CCFLAGS) $(CC_DEPFLAGS) $(CC_C) $(CC_O) <ソース>
//! ```
//!
//! `CC_C` は `-c`、`CC_O` は `-o <オブジェクト>` なので、コマンドは必ず
//! `... -c -o foo.o src/foo.c` の形で終わる。ここではその形を前提に、
//! オブジェクト、ソース、プリプロセスに影響するフラグを取り出す。

use std::path::Path;

use crate::Result;

/// 1 つの翻訳単位をコンパイルするコマンド。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileCommand {
    /// 出力されるオブジェクトファイル。ビルドディレクトリからの相対パス。
    pub object: String,

    /// コンパイルされるソース。コマンドに書かれていた通りのパス。
    pub source: String,

    pub flags: CompileFlags,
}

/// コンパイルコマンドのうち、プリプロセスの結果を左右するフラグ。
///
/// cinrs で同じ翻訳単位を再現するために必要なものだけを持つ。最適化や警告の
/// フラグは cinrs の側に対応するものが無いので、意図的に捨てている。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompileFlags {
    /// `-std=` の値。
    pub std: Option<String>,

    /// `-D` の値。`NAME` または `NAME=VALUE` の形で、コマンドに現れた順。
    pub defines: Vec<String>,

    /// `-I` の値。コマンドに書かれていた通りのパスで、現れた順。
    /// インクルードの探索は順序に意味があるので、並べ替えない。
    pub include_dirs: Vec<String>,
}

/// プリプロセスの結果を左右するが、まだ扱っていないフラグ。
///
/// これらを黙って捨てると、cinrs で取り込んだときに元のビルドと違う内容に
/// なっても気づけない。現れたらエラーにして、対応を足すきっかけにする。
const UNSUPPORTED_FLAGS: &[&str] = &[
    "-U",
    "-include",
    "-imacros",
    "-isystem",
    "-iquote",
    "-idirafter",
];

/// コマンド 1 行を読み、C のコンパイルコマンドであればその内容を返す。
///
/// コンパイル以外のコマンド（`ar` や各種スクリプトなど）には `Ok(None)` を返す。
///
/// コンパイルコマンドではあるが、そのまま取り込むと元のビルドを再現できない
/// 場合はエラーにする。C 以外のソース（アセンブリなど）をコンパイルしている
/// 場合と、[`UNSUPPORTED_FLAGS`] が使われている場合がこれに当たる。
pub fn parse_line(line: &str) -> Result<Option<CompileCommand>> {
    // 引用符が閉じていない行は、コマンドとして解釈できないので対象外にする。
    // コンパイルコマンドを取りこぼした場合は、ビルド後にオブジェクトファイルの
    // 一覧と突き合わせる検査で検出される。
    let Some(words) = split_shell_words(line) else {
        return Ok(None);
    };

    let Some(object) = find_object(&words) else {
        return Ok(None);
    };

    let source = words.last().expect("オブジェクトがあるなら語は空でない");

    if Path::new(source).extension().is_none_or(|ext| ext != "c") {
        return Err(format!(
            "C 以外のソースがコンパイルされています: {source}\n\
             cinrs が取り込めるのは C だけなので、アセンブリなどを使わない構成にしてください。"
        )
        .into());
    }

    Ok(Some(CompileCommand {
        object: object.to_owned(),
        source: source.clone(),
        flags: parse_flags(&words)?,
    }))
}

/// コンパイルコマンド（`-c` と `-o <何か>.o` を持つ）であれば、オブジェクトの
/// パスを返す。
fn find_object(words: &[String]) -> Option<&str> {
    if !words.iter().any(|word| word == "-c") {
        return None;
    }

    words
        .windows(2)
        .find(|pair| pair[0] == "-o" && pair[1].ends_with(".o"))
        .map(|pair| pair[1].as_str())
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
                "未対応のフラグ {unsupported} がコンパイルコマンドに含まれています: {word}"
            )
            .into());
        }

        if let Some(std) = word.strip_prefix("-std=") {
            flags.std = Some(std.to_owned());
        } else if let Some(value) = flag_value(word, "-D", &mut words)? {
            push_unique(&mut flags.defines, value);
        } else if let Some(value) = flag_value(word, "-I", &mut words)? {
            push_unique(&mut flags.include_dirs, value);
        }
    }

    Ok(flags)
}

/// `-Dfoo` と `-D foo` のように、値を続けて書く形と次の語に書く形の両方から
/// 値を取り出す。`word` が `flag` で始まらなければ `Ok(None)` を返す。
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
        None => Err(format!("{flag} の値がありません").into()),
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

/// コマンド 1 行を、シェルの規則に沿って語に分割する。
///
/// 扱うのは、`make` が表示するコマンドに現れる範囲の引用符とバックスラッシュ
/// だけで、変数展開などは解釈しない。引用符が閉じていないなど、分割できない
/// 行には `None` を返す。
pub(super) fn split_shell_words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();

    // 空の引用符（`""`）も 1 つの語になるので、文字列が空かどうかとは別に、
    // 語の途中かどうかを持つ。
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

                        // 二重引用符の中では、`\` は特定の文字の前でだけエスケープになる。
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

    /// `n9.0.2` の FFmpeg を macOS でビルドしたときの実際のコマンドから、警告のフラグを
    /// 省いたもの。
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

    /// `make -n` の出力には、アーカイブの作成やスクリプトの実行など、コンパイル
    /// 以外のコマンドが混ざる。これらを翻訳単位として数えないこと。
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

    /// アセンブリを無効にし忘れた構成では `.S` などがコンパイルされる。cinrs では
    /// 取り込めないので、cinrs_manifest.tomlから黙って落とさずにエラーにする。落とすと、
    /// リンク時に未定義シンボルになるまで気づけない。
    #[test]
    fn rejects_non_c_source() {
        let line = "gcc -I. -c -o libavutil/aarch64/cpu.o src/libavutil/aarch64/asm.S";

        assert!(parse_line(line).is_err());
    }

    /// 強制インクルードなどは翻訳単位の内容を変える。扱いを決めていないフラグを
    /// 無視してcinrs_manifest.tomlを作ると、元のビルドと違うものを取り込むことになる。
    #[test]
    fn rejects_unsupported_preprocessor_flags() {
        for flag in ["-include config.h", "-UPIC", "-isystem /opt/include"] {
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

    /// 同じフラグが複数回現れても 1 つにまとめ、最初に現れた位置を保つこと。
    /// インクルードの探索順を変えないため。
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
