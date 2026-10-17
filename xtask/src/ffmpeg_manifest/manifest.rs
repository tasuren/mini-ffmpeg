//! ビルドマニフェスト（`cinrs_manifest.toml`）のデータと、その書き出し。

use std::{fmt::Write as _, path::Path};

use super::{
    FFMPEG_DIR, GENERATED_DIR,
    source_tree::{Location, SourceTree},
};
use crate::Result;

/// FFmpeg のビルド結果から読み取った、cinrs で取り込むために必要な情報。
///
/// パスはすべて `/` 区切りの相対パスで持つ。絶対パスを入れると、生成した
/// マシンによって内容が変わり、コミットした結果が再生成のたびに差分になる。
#[derive(Debug)]
pub struct Manifest {
    /// `libavutil/ffversion.h` の `FFMPEG_VERSION`。
    pub ffmpeg_version: String,

    /// `configure` に渡した引数。
    pub configure_args: Vec<String>,

    /// 生成を行った環境の CPU アーキテクチャと OS。
    ///
    /// `config.h` の内容は生成した環境に依存するので、どの環境のものかを残す。
    /// アーキテクチャは、xtask 自身のものではなく、`configure` が使ったコンパイラの
    /// ターゲットのもの。
    pub host_arch: String,
    pub host_os: String,

    /// C の規格（`-std=` の値）。
    pub c_standard: String,

    /// 全翻訳単位に共通のマクロ定義（`-D` の値）。
    pub defines: Vec<String>,

    /// 全翻訳単位に共通のインクルードディレクトリ。探索する順に並ぶ。
    pub include_dirs: Vec<IncludeDir>,

    /// コンパイルされる `.c`。FFmpeg のツリーからの相対パス。
    pub translation_units: Vec<String>,

    /// 翻訳単位から include されるだけで、単独ではコンパイルされないソース。
    /// テンプレートとして使われる `.c` などが入る。FFmpeg のツリーからの相対パス。
    pub included_sources: Vec<String>,

    /// 翻訳単位が include するヘッダ。FFmpeg のツリーからの相対パス。
    pub headers: Vec<String>,

    /// 翻訳単位が include する、`configure` と `make` の生成物。
    /// ビルドディレクトリからの相対パスで、`FFmpeg_generated/` 以下に同じパスで
    /// コピーされる。
    pub generated: Vec<String>,

    /// 翻訳単位が include したヘッダのうち、FFmpeg が公開の API としてインストール
    /// するもの。`headers` と `generated` の一部で、`#include` に書く名前
    /// （`libavutil/avutil.h` の形）で持つ。
    ///
    /// ライブラリを使う側に見せる宣言と型を、ここから取り込む。
    pub public_headers: Vec<String>,
}

/// インクルードディレクトリが、どこを起点にしたパスかを表す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncludeRoot {
    /// `configure` と `make` の生成物を置いたディレクトリ（`FFmpeg_generated/`）。
    Generated,

    /// FFmpeg のツリーのルート（`FFmpeg/`）。
    Ffmpeg,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeDir {
    pub root: IncludeRoot,

    /// 起点からの相対パス。起点そのものは `.`。
    pub path: String,
}

impl IncludeDir {
    /// コンパイルコマンドに書かれていた `-I` の値を、起点と相対パスに分ける。
    ///
    /// FFmpeg のビルドでは、ビルドディレクトリで `make` を実行し、FFmpeg の
    /// ツリーは `src` というシンボリックリンク越しか、絶対パスで参照される。
    ///
    /// - `.` などの相対パスは、ビルドディレクトリが起点。
    /// - `tree` がツリーとみなすパスは、FFmpeg のツリーが起点。
    ///
    /// どちらにも当たらない絶対パスはエラーにする。マシン固有のディレクトリを
    /// 見ていることになり、他の環境で再現できないため。
    pub fn classify(raw: &str, tree: &SourceTree) -> Result<Self> {
        match tree.classify(raw) {
            Location::Tree(path) => Ok(Self::new(IncludeRoot::Ffmpeg, path)),
            Location::Generated(path) => Ok(Self::new(IncludeRoot::Generated, path)),
            Location::External => Err(format!(
                "FFmpeg のツリーの外にあるインクルードディレクトリには未対応です: {raw}"
            )
            .into()),
        }
    }

    /// `mini-ffmpeg-sys` のルートからの相対パスにする。
    ///
    /// cinrs の `#pragma cinrs include_path` は、相対パスをクレートのルートを
    /// 起点に解決する。どちらの起点もクレートの中にあるので、この形にしておけば、
    /// 取り込みのコードにそのまま書ける。
    pub fn crate_relative(&self) -> String {
        let root = match self.root {
            IncludeRoot::Generated => GENERATED_DIR,
            IncludeRoot::Ffmpeg => FFMPEG_DIR,
        };

        if self.path == "." {
            root.to_owned()
        } else {
            format!("{root}/{}", self.path)
        }
    }

    fn new(root: IncludeRoot, path: String) -> Self {
        Self {
            root,
            path: if path.is_empty() {
                ".".to_owned()
            } else {
                path
            },
        }
    }
}

/// パスを `/` 区切りの文字列にする。`.` の要素と末尾の区切りは落とす。
pub fn to_slash_path(path: &Path) -> String {
    path.components()
        .filter(|component| !matches!(component, std::path::Component::CurDir))
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

impl Manifest {
    /// TOML として書き出す。
    ///
    /// 配列は 1 要素 1 行にする。コミットする生成物なので、再生成したときの
    /// 差分が要素単位で読めるようにするため。
    pub fn to_toml(&self) -> String {
        let mut out = String::new();

        out.push_str(
            "# `cargo xtask ffmpeg-manifest` が生成したファイル。手で編集しない。\n\
             # 構成を変えるときは、xtask の CONFIGURE_ARGS を変更して再生成する。\n",
        );

        out.push_str("\n[ffmpeg]\n");
        write_string(&mut out, "version", &self.ffmpeg_version);
        write_array(&mut out, "configure_args", &self.configure_args);

        out.push_str(
            "\n# 生成を行った環境。`FFmpeg_generated/` の内容はこの環境に依存する。\n[host]\n",
        );
        write_string(&mut out, "arch", &self.host_arch);
        write_string(&mut out, "os", &self.host_os);

        out.push_str("\n# 全翻訳単位に共通の、コンパイル時の指定。\n[compile]\n");
        write_string(&mut out, "std", &self.c_standard);
        write_array(&mut out, "defines", &self.defines);

        let include_dirs: Vec<String> = self
            .include_dirs
            .iter()
            .map(IncludeDir::crate_relative)
            .collect();

        out.push_str(
            "\n# 探索する順。`mini-ffmpeg-sys` のルートからの相対パスで、\n\
             # `#pragma cinrs include_path` にそのまま書ける。\n",
        );
        write_array(&mut out, "include_dirs", &include_dirs);

        out.push_str("\n[sources]\n");

        out.push_str("# コンパイルされる `.c`。FFmpeg のツリーからの相対パス。\n");
        write_array(&mut out, "translation_units", &self.translation_units);

        out.push_str("\n# 翻訳単位から include されるだけで、単独ではコンパイルされないソース。\n");
        write_array(&mut out, "included_sources", &self.included_sources);

        out.push_str("\n# 翻訳単位が include するヘッダ。FFmpeg のツリーからの相対パス。\n");
        write_array(&mut out, "headers", &self.headers);

        out.push_str(
            "\n# 翻訳単位が include する、`configure` と `make` の生成物。\n\
             # `FFmpeg_generated/` からの相対パス。\n",
        );
        write_array(&mut out, "generated", &self.generated);

        out.push_str(
            "\n# 上の `headers` と `generated` のうち、FFmpeg が公開の API としてインストール\n\
             # するヘッダ。`#include` に書く名前で、インクルードディレクトリから見つかる。\n",
        );
        write_array(&mut out, "public_headers", &self.public_headers);

        out
    }
}

fn write_string(out: &mut String, key: &str, value: &str) {
    let _ = writeln!(out, "{key} = {}", toml_string(value));
}

fn write_array(out: &mut String, key: &str, values: &[String]) {
    let _ = writeln!(out, "{key} = [");

    for value in values {
        let _ = writeln!(out, "    {},", toml_string(value));
    }

    out.push_str("]\n");
}

/// TOML の基本文字列（二重引用符の文字列）として書き出す。
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');

    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),

            // TOML の基本文字列には、残りの制御文字をそのまま書けない。
            character if character.is_control() => {
                let _ = write!(out, "\\u{:04X}", character as u32);
            }

            character => out.push(character),
        }
    }

    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> SourceTree {
        SourceTree::new(&["src", "/work/FFmpeg"], &["/work/build"])
    }

    fn classify(raw: &str) -> IncludeDir {
        IncludeDir::classify(raw, &tree()).unwrap()
    }

    fn dir(root: IncludeRoot, path: &str) -> IncludeDir {
        IncludeDir {
            root,
            path: path.to_owned(),
        }
    }

    /// `-I.` はビルドディレクトリを指す。`config.h` などの生成物の置き場所。
    #[test]
    fn classifies_build_directory() {
        assert_eq!(classify("."), dir(IncludeRoot::Generated, "."));
        assert_eq!(
            classify("libavcodec"),
            dir(IncludeRoot::Generated, "libavcodec")
        );
    }

    /// FFmpeg のビルドは、ツリーを `src` というシンボリックリンク越しに参照する。
    /// 末尾に `/` が付いた `-Isrc/` の形で渡されるので、その形でも読めること。
    #[test]
    fn classifies_src_symlink_as_ffmpeg_tree() {
        assert_eq!(classify("src/"), dir(IncludeRoot::Ffmpeg, "."));
        assert_eq!(classify("src"), dir(IncludeRoot::Ffmpeg, "."));
        assert_eq!(
            classify("src/compat/stdbit"),
            dir(IncludeRoot::Ffmpeg, "compat/stdbit")
        );
    }

    /// `compat` 以下のディレクトリは絶対パスで渡される。生成したマシンのパスが
    /// ビルドマニフェストに残らないよう、ツリーからの相対パスに直すこと。
    #[test]
    fn classifies_absolute_path_inside_ffmpeg_tree() {
        assert_eq!(
            classify("/work/FFmpeg/compat/stdbit"),
            dir(IncludeRoot::Ffmpeg, "compat/stdbit"),
        );
    }

    /// `src` で始まるだけの別のディレクトリ（`srcfoo`）を、シンボリックリンクの
    /// `src` と取り違えないこと。
    #[test]
    fn does_not_treat_src_prefixed_name_as_symlink() {
        assert_eq!(classify("srcfoo"), dir(IncludeRoot::Generated, "srcfoo"));
    }

    #[test]
    fn rejects_absolute_path_outside_ffmpeg_tree() {
        assert!(IncludeDir::classify("/opt/homebrew/include", &tree()).is_err());
    }

    /// ビルドマニフェストに書き出すパスは、取り込みのコードの `#pragma cinrs include_path` に
    /// そのまま書く。起点そのもの（`.`）のときに `FFmpeg/.` のような余計な要素を
    /// 付けないこと。
    #[test]
    fn renders_include_dir_relative_to_crate_root() {
        assert_eq!(classify(".").crate_relative(), "FFmpeg_generated");
        assert_eq!(classify("src/").crate_relative(), "FFmpeg");
        assert_eq!(
            classify("/work/FFmpeg/compat/stdbit").crate_relative(),
            "FFmpeg/compat/stdbit",
        );
    }

    #[test]
    fn escapes_toml_strings() {
        assert_eq!(toml_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(toml_string("a\tb\u{1}"), "\"a\\tb\\u0001\"");
    }
}
