//! ビルドの出力に現れたパスが、FFmpeg のツリーのファイルかどうかの判定。
//!
//! FFmpeg の Makefile は、ツリーのファイルを `ffbuild/config.mak` の 2 つの変数で
//! 参照する。
//!
//! - `SRC_LINK`: コンパイルするソースと、`-I$(SRC_LINK)/` に使われる。
//!   `configure` がビルドディレクトリにツリーを指すシンボリックリンクを作れたら
//!   `src`、作れなかったらツリーの絶対パスになる。
//! - `SRC_PATH`: ツリーの絶対パス。`compat` 以下の `-I` と、インストールされる
//!   ヘッダに使われる。
//!
//! Windows の MSYS2 では、`ln -s` が既定でディレクトリのコピーになるので、
//! `configure` はリンクを作れない。どちらの変数も `/home/...` の形の MSYS の
//! パスになり、Windows のプログラムであるコンパイラには、MSYS2 が
//! `C:/msys64/home/...` の形に変換して渡す。そこで、2 つの変数の値と、xtask が
//! 知っているツリーのパスのいずれかで始まるパスを、ツリーのファイルとみなす。
//!
//! 生成物は、ふつうはビルドディレクトリからの相対パスで書かれる。ただし、MSVC の
//! 依存関係ファイルは `cl.exe -showIncludes` の出力から作るので、`config.h` も
//! `C:/.../ffmpeg-build/config.h` のような絶対パスになる。ビルドディレクトリの
//! 下の絶対パスも、生成物とみなす。

use std::{fs, path::Path};

use super::manifest::to_slash_path;
use crate::Result;

/// ビルドの出力に現れたパスが、どこのファイルかを表す。
#[derive(Debug, PartialEq, Eq)]
pub enum Location {
    /// FFmpeg のツリーにあるファイル。ツリーからの相対パス。
    Tree(String),

    /// `configure` や `make` が生成したファイル。ビルドディレクトリからの相対パス。
    Generated(String),

    /// どちらでもないファイル。システムのヘッダや SDK の設定ファイルなど。
    External,
}

/// FFmpeg のツリーと、ビルドディレクトリを指すパスの書き方の一覧。
#[derive(Debug)]
pub struct SourceTree {
    /// ツリーのルートの書き方。`/` 区切りで、末尾の `/` は付けない。
    roots: Vec<String>,

    /// ビルドディレクトリの絶対パスの書き方。形は `roots` と同じ。
    build_roots: Vec<String>,
}

impl SourceTree {
    /// ビルドディレクトリの `ffbuild/config.mak` から読む。`ffmpeg_src` は、
    /// xtask が知っているツリーのパス。
    pub fn read(build_dir: &Path, ffmpeg_src: &Path) -> Result<Self> {
        let path = build_dir.join("ffbuild/config.mak");
        let config = fs::read_to_string(&path)
            .map_err(|error| format!("{} を読めません: {error}", path.display()))?;

        Self::from_config(&config, ffmpeg_src, &build_dir.canonicalize()?)
            .map_err(|error| format!("{}: {error}", path.display()).into())
    }

    fn from_config(config: &str, ffmpeg_src: &Path, build_dir: &Path) -> Result<Self> {
        // `SRC_PATH:=...` の行もあるが、`=` の直前で区切るので取り違えない。
        let variable = |name: &str| {
            config
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
                .ok_or_else(|| format!("{name} がありません"))
        };

        Ok(Self::new(
            &[
                variable("SRC_LINK")?,
                variable("SRC_PATH")?,
                &native_path(ffmpeg_src),
            ],
            &[&native_path(build_dir)],
        ))
    }

    pub fn new(roots: &[&str], build_roots: &[&str]) -> Self {
        Self {
            roots: normalize_roots(roots),
            build_roots: normalize_roots(build_roots),
        }
    }

    pub fn classify(&self, raw: &str) -> Location {
        if let Some(relative) = find_relative(raw, &self.roots) {
            return Location::Tree(relative);
        }

        if let Some(relative) = find_relative(raw, &self.build_roots) {
            return Location::Generated(relative);
        }

        // ビルドディレクトリで `make` を実行しているので、相対パスはすべて
        // ビルドディレクトリが起点になる。
        if is_absolute(raw) {
            Location::External
        } else {
            Location::Generated(to_slash_path(Path::new(raw)))
        }
    }
}

/// 末尾の `/` を落とし、空のものと重複を除く。
fn normalize_roots(roots: &[&str]) -> Vec<String> {
    let mut unique = Vec::new();

    for root in roots {
        let root = root.trim_end_matches('/').to_owned();

        if !root.is_empty() && !unique.contains(&root) {
            unique.push(root);
        }
    }

    unique
}

/// `raw` がいずれかのルートの下にあれば、そのルートからの相対パスを返す。
fn find_relative(raw: &str, roots: &[String]) -> Option<String> {
    roots
        .iter()
        .find_map(|root| strip_root(raw, root))
        .map(|relative| to_slash_path(Path::new(relative)))
}

/// `raw` が `root` そのものか `root` の下であれば、`root` より後ろを返す。
///
/// 要素の単位で比べるので、`/work/FFmpegx` は `/work/FFmpeg` の下ではない。
fn strip_root<'a>(raw: &'a str, root: &str) -> Option<&'a str> {
    let head = raw.get(..root.len())?;

    // Windows のファイルシステムは大文字と小文字を区別しない。ドライブ文字は
    // `C:` とも `c:` とも書かれる。
    let same = if cfg!(windows) {
        head.eq_ignore_ascii_case(root)
    } else {
        head == root
    };

    let rest = &raw[root.len()..];

    (same && (rest.is_empty() || rest.starts_with('/'))).then(|| rest.trim_start_matches('/'))
}

/// `/` で始まるパスと、ドライブ文字で始まるパスも絶対パスとみなす。
///
/// Windows の Rust は、ドライブ文字の無い `/home/...` を絶対パスとしないが、
/// MSYS2 ではこれが絶対パスになる。逆に、macOS や Linux の Rust は
/// `C:/...` を絶対パスとしない。しかし、判定するのはコンパイラが書き出した
/// 文字列で、どの OS で xtask を動かしているかとは関係なく決まるべきなので、
/// ドライブ文字は文字列の形で見分ける。相対パスと取り違えると、システムの
/// ヘッダが生成物として扱われる。
fn is_absolute(raw: &str) -> bool {
    raw.starts_with('/') || has_drive_letter(raw) || Path::new(raw).is_absolute()
}

/// `C:/` や `C:\` で始まるか。
fn has_drive_letter(raw: &str) -> bool {
    matches!(
        raw.as_bytes(),
        [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic()
    )
}

/// xtask が知っているパスを、ビルドの出力と比べられる `/` 区切りの形にする。
///
/// Windows で `canonicalize` したパスには `\\?\` が付くが、コンパイラが書き出す
/// パスには付かないので外す。MSYS2 の bash にパスを渡すときも、この形にする。
pub(super) fn native_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);

    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// macOS や Linux のように、`src` のリンクを作れた構成。
    fn linked() -> SourceTree {
        SourceTree::new(&["src", "/work/FFmpeg", "/work/FFmpeg"], &["/work/build"])
    }

    /// Windows の MSYS2 のように、リンクを作れなかった構成。
    fn unlinked() -> SourceTree {
        SourceTree::new(
            &[
                "/home/u/FFmpeg",
                "/home/u/FFmpeg",
                "C:/msys64/home/u/FFmpeg",
            ],
            &["C:/msys64/home/u/build"],
        )
    }

    fn in_tree(path: &str) -> Location {
        Location::Tree(path.to_owned())
    }

    fn generated(path: &str) -> Location {
        Location::Generated(path.to_owned())
    }

    #[test]
    fn classifies_paths_through_src_link() {
        let tree = linked();

        assert_eq!(
            tree.classify("src/libavutil/log.c"),
            in_tree("libavutil/log.c")
        );
        assert_eq!(tree.classify("src/"), in_tree(""));
        assert_eq!(
            tree.classify("/work/FFmpeg/compat/stdbit"),
            in_tree("compat/stdbit")
        );
        assert_eq!(tree.classify("config.h"), generated("config.h"));
        assert_eq!(tree.classify("."), generated(""));
        assert_eq!(tree.classify("/usr/include/stdio.h"), Location::External);
    }

    /// リンクが無いと、ツリーは `make` からは MSYS のパスで、コンパイラからは
    /// Windows のパスで見える。どちらもツリーとして扱うこと。
    #[test]
    fn classifies_paths_without_src_link() {
        let tree = unlinked();

        assert_eq!(
            tree.classify("/home/u/FFmpeg/libavutil/log.c"),
            in_tree("libavutil/log.c"),
        );
        assert_eq!(
            tree.classify("C:/msys64/home/u/FFmpeg/libavutil/log.h"),
            in_tree("libavutil/log.h"),
        );
        assert_eq!(tree.classify("/home/u/FFmpeg/"), in_tree(""));
        assert_eq!(
            tree.classify("libavutil/ffversion.h"),
            generated("libavutil/ffversion.h")
        );
        assert_eq!(tree.classify("/opt/homebrew/include"), Location::External);
        assert_eq!(
            tree.classify("C:/Program Files/Windows Kits/10/Include/stdio.h"),
            Location::External,
        );
    }

    /// MSVC の依存関係ファイルでは、生成物もビルドディレクトリの絶対パスで
    /// 書かれる。システムのヘッダと同じ扱いにして、生成物から落とさないこと。
    #[test]
    fn classifies_absolute_path_in_build_directory_as_generated() {
        assert_eq!(
            unlinked().classify("C:/msys64/home/u/build/config.h"),
            generated("config.h"),
        );
        assert_eq!(
            unlinked().classify("C:/msys64/home/u/build/libavcodec/codec_list.c"),
            generated("libavcodec/codec_list.c"),
        );
        assert_eq!(
            linked().classify("/work/build/libavutil/avconfig.h"),
            generated("libavutil/avconfig.h"),
        );
    }

    /// 名前が同じ文字で始まるだけの別のディレクトリを、ツリーと取り違えないこと。
    #[test]
    fn does_not_match_prefix_of_component() {
        assert_eq!(linked().classify("srcfoo"), generated("srcfoo"));
        assert_eq!(
            unlinked().classify("/home/u/FFmpegx/a.h"),
            Location::External
        );
        assert_eq!(
            unlinked().classify("C:/msys64/home/u/buildx/config.h"),
            Location::External
        );
    }

    #[test]
    fn reads_roots_from_config_mak() {
        let config = "ARCH=x86\n\
                      SRC_PATH=/home/u/FFmpeg\n\
                      SRC_LINK=/home/u/FFmpeg\n\
                      SRC_PATH:=$(SRC_PATH:.%=..%)\n";

        let tree = SourceTree::from_config(
            config,
            Path::new("/elsewhere/FFmpeg"),
            Path::new("/elsewhere/build"),
        )
        .unwrap();

        assert_eq!(tree.roots, ["/home/u/FFmpeg", "/elsewhere/FFmpeg"]);
        assert_eq!(tree.build_roots, ["/elsewhere/build"]);
        assert!(
            SourceTree::from_config("SRC_PATH=/a\n", Path::new("/a"), Path::new("/b")).is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn normalizes_canonical_windows_path() {
        assert_eq!(
            native_path(Path::new(r"\\?\C:\msys64\home\u\FFmpeg")),
            "C:/msys64/home/u/FFmpeg",
        );
        assert_eq!(
            unlinked().classify("c:/msys64/home/u/ffmpeg/libavutil/log.h"),
            in_tree("libavutil/log.h"),
        );
    }
}
