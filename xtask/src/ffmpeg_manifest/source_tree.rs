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

/// FFmpeg のツリーを指すパスの書き方の一覧。
#[derive(Debug)]
pub struct SourceTree {
    /// ツリーのルートの書き方。`/` 区切りで、末尾の `/` は付けない。
    roots: Vec<String>,
}

impl SourceTree {
    /// ビルドディレクトリの `ffbuild/config.mak` から読む。`ffmpeg_src` は、
    /// xtask が知っているツリーのパス。
    pub fn read(build_dir: &Path, ffmpeg_src: &Path) -> Result<Self> {
        let path = build_dir.join("ffbuild/config.mak");
        let config = fs::read_to_string(&path)
            .map_err(|error| format!("{} を読めません: {error}", path.display()))?;

        Self::from_config(&config, ffmpeg_src)
            .map_err(|error| format!("{}: {error}", path.display()).into())
    }

    fn from_config(config: &str, ffmpeg_src: &Path) -> Result<Self> {
        // `SRC_PATH:=...` の行もあるが、`=` の直前で区切るので取り違えない。
        let variable = |name: &str| {
            config
                .lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
                .ok_or_else(|| format!("{name} がありません"))
        };

        Ok(Self::new(&[
            variable("SRC_LINK")?,
            variable("SRC_PATH")?,
            &native_path(ffmpeg_src),
        ]))
    }

    pub fn new(roots: &[&str]) -> Self {
        let mut unique = Vec::new();

        for root in roots {
            let root = root.trim_end_matches('/').to_owned();

            if !root.is_empty() && !unique.contains(&root) {
                unique.push(root);
            }
        }

        Self { roots: unique }
    }

    pub fn classify(&self, raw: &str) -> Location {
        if let Some(relative) = self.roots.iter().find_map(|root| strip_root(raw, root)) {
            return Location::Tree(to_slash_path(Path::new(relative)));
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

/// `/` で始まるパスも絶対パスとみなす。Windows の Rust は、ドライブ文字の無い
/// `/home/...` を絶対パスとしないが、MSYS2 ではこれが絶対パスになる。
fn is_absolute(raw: &str) -> bool {
    raw.starts_with('/') || Path::new(raw).is_absolute()
}

/// ツリーのパスを、ビルドの出力と比べられる `/` 区切りの形にする。
///
/// Windows で `canonicalize` したパスには `\\?\` が付くが、コンパイラが書き出す
/// パスには付かないので外す。
fn native_path(path: &Path) -> String {
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
        SourceTree::new(&["src", "/work/FFmpeg", "/work/FFmpeg"])
    }

    /// Windows の MSYS2 のように、リンクを作れなかった構成。
    fn unlinked() -> SourceTree {
        SourceTree::new(&[
            "/home/u/FFmpeg",
            "/home/u/FFmpeg",
            "C:/msys64/home/u/FFmpeg",
        ])
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

    /// 名前が同じ文字で始まるだけの別のディレクトリを、ツリーと取り違えないこと。
    #[test]
    fn does_not_match_prefix_of_component() {
        assert_eq!(linked().classify("srcfoo"), generated("srcfoo"));
        assert_eq!(
            unlinked().classify("/home/u/FFmpegx/a.h"),
            Location::External
        );
    }

    #[test]
    fn reads_roots_from_config_mak() {
        let config = "ARCH=x86\n\
                      SRC_PATH=/home/u/FFmpeg\n\
                      SRC_LINK=/home/u/FFmpeg\n\
                      SRC_PATH:=$(SRC_PATH:.%=..%)\n";

        let tree = SourceTree::from_config(config, Path::new("/elsewhere/FFmpeg")).unwrap();

        assert_eq!(tree.roots, ["/home/u/FFmpeg", "/elsewhere/FFmpeg"]);
        assert!(SourceTree::from_config("SRC_PATH=/a\n", Path::new("/a")).is_err());
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
