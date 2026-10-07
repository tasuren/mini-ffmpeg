//! コンパイラが書き出す依存関係ファイル（`.d`）の読み取り。
//!
//! FFmpeg のビルドは `-MMD -MF foo.d` を付けてコンパイルするので、翻訳単位ごとに
//! 「この `.o` はどのファイルを include したか」が Makefile の規則の形で残る。
//!
//! ```text
//! libavformat/wavdec.o: \
//!   src/libavformat/wavdec.c config_components.h src/libavutil/avassert.h \
//!   config.h
//! ```
//!
//! `-MMD` はシステムヘッダを除外するので、載るのは基本的にプロジェクト内の
//! ファイルだけになる。

use crate::Result;

/// 依存関係ファイルの内容から、依存しているファイルのパスを取り出す。
///
/// 返すのは `:` の右側に書かれたパスで、書かれていた順のまま。重複の除去や
/// パスの解釈は行わない。
///
/// 規則の形になっていない行があった場合はエラーにする。黙って読み飛ばすと、
/// 取り込む対象の一覧から依存が欠けても気づけないため。
pub fn parse_dependencies(content: &str) -> Result<Vec<String>> {
    // 行末の `\` は行の継続。先に繋げてから、論理行ごとに規則として読む。
    let joined = content.replace("\\\r\n", " ").replace("\\\n", " ");

    let mut dependencies = Vec::new();

    for line in joined.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let Some(separator) = find_rule_separator(line) else {
            return Err(format!("依存関係の規則として読めない行です: {line}").into());
        };

        dependencies.extend(split_paths(&line[separator + 1..]));
    }

    Ok(dependencies)
}

/// 規則のターゲットと依存を区切る `:` の位置を探す。
///
/// 区切りの `:` は直後が空白か行末になる。`C:\foo` のようなドライブ文字の `:` を
/// 区切りと取り違えないよう、この条件で見分ける。
fn find_rule_separator(line: &str) -> Option<usize> {
    line.char_indices().find_map(|(index, character)| {
        let next = line[index + character.len_utf8()..].chars().next();

        (character == ':' && next.is_none_or(char::is_whitespace)).then_some(index)
    })
}

/// 空白区切りのパスの並びを、Makefile のエスケープを解きながら分割する。
///
/// コンパイラは、パス中の空白を `\ `、`#` を `\#`、`$` を `$$` と書き出す。
/// それ以外の `\` はそのままの文字として扱う（Windows のパス区切りのため）。
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

    /// 実際に clang が書き出した形（ターゲットの直後で改行し、依存を複数行に
    /// 分けて継続する）を読めること。
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

    /// パスに空白が含まれる場合、コンパイラは `\ ` と書き出す。ここで分割して
    /// しまうと 1 つのパスが 2 つの存在しないパスになる。Xcode の SDK のパスなど、
    /// FFmpeg の外のパスが依存に混ざることがあるので必要。
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

    /// Windows のパスにあるドライブ文字の `:` を、ターゲットと依存の区切りと
    /// 取り違えないこと。取り違えると、ターゲットの残りが依存として紛れ込む。
    #[test]
    fn does_not_split_at_drive_letter_colon() {
        let content = "C:\\build\\a.o: C:\\src\\a.c\n";

        assert_eq!(parse_dependencies(content).unwrap(), ["C:\\src\\a.c"]);
    }

    /// 依存が 1 つも無い規則（`foo.h:` だけの行）は、`-MP` を付けたときに
    /// ヘッダごとに書き出される。依存としては何も足さずに読み飛ばす。
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
