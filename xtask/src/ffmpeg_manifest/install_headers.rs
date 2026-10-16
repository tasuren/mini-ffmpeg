//! `make -n install-headers` が表示するコマンドから、FFmpeg が公開の API として
//! インストールするヘッダを読み取る。
//!
//! FFmpeg の `ffbuild/library.mak` は、ライブラリごとに、次の形でヘッダを
//! インストールする。
//!
//! ```make
//! install-lib$(NAME)-headers: $(addprefix $(SUBDIR),$(HEADERS) $(BUILT_HEADERS))
//!     $(Q)mkdir -p "$(INCINSTDIR)"
//!     $$(INSTALL) -m 644 $$^ "$(INCINSTDIR)"
//! ```
//!
//! `INCINSTDIR` は `<インストール先>/include/lib<名前>` で、利用者は
//! `#include <libavutil/avutil.h>` のように、ライブラリのディレクトリの名前を
//! 付けて読む。ここでは、コマンドが `... -m 644 <ヘッダ>... <ディレクトリ>` の形で
//! 終わることを前提に、ヘッダと、その `#include` での名前を取り出す。
//!
//! 実際にインストールはしない。`-n` はコマンドを表示するだけなので、何が
//! 公開ヘッダかを、インストール先に何も書き込まずに知ることができる。

use std::{collections::BTreeSet, path::Path};

use super::{SOURCE_LINK, compile_command::split_shell_words, manifest::to_slash_path};
use crate::Result;

/// インストールされるヘッダ 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledHeader {
    /// インストール元のファイル。コマンドに書かれていた通りのパス。
    pub source: String,

    /// インストール先のディレクトリの名前。ライブラリの名前（`libavutil` など）。
    pub library: String,
}

impl InstalledHeader {
    /// 利用者が `#include` に書く名前。`libavutil/avutil.h` の形。
    ///
    /// `install` はファイルをディレクトリの直下にコピーするので、インストール元が
    /// どこにあっても、ライブラリの名前とファイルの名前だけで決まる。
    fn include_name(&self) -> String {
        let file_name = self.source.rsplit('/').next().unwrap_or(&self.source);

        format!("{}/{file_name}", self.library)
    }

    /// インストール元の場所を、ビルドマニフェストの `headers` や `generated` と同じ
    /// 形のパスにする。
    fn location(&self, ffmpeg_src: &Path) -> Result<Location> {
        let path = Path::new(&self.source);

        // ツリーのヘッダは、`configure` に渡した絶対パスで書かれる。
        if path.is_absolute() {
            let relative = path.strip_prefix(ffmpeg_src).map_err(|_| {
                format!(
                    "FFmpeg のツリーの外にある不明なヘッダが\
                    インストールされる構成になってて、これは想定外です: {}",
                    self.source,
                )
            })?;

            return Ok(Location::Tree(to_slash_path(relative)));
        }

        // 相対パスは、`make` を実行したビルドディレクトリが起点。そのうち
        // シンボリックリンクの `src/` 以下がツリーのファイルで、残りは生成物。
        Ok(match path.strip_prefix(SOURCE_LINK) {
            Ok(relative) => Location::Tree(to_slash_path(relative)),
            Err(_) => Location::Generated(to_slash_path(path)),
        })
    }
}

/// インストール元のヘッダがある場所。
#[derive(Debug, PartialEq, Eq)]
enum Location {
    /// FFmpeg のツリー。ツリーからの相対パス。
    Tree(String),

    /// `configure` や `make` の生成物。ビルドディレクトリからの相対パス。
    Generated(String),
}

/// コマンド 1 行を読み、ヘッダをインストールするコマンドであれば、その内容を返す。
///
/// それ以外のコマンド（`mkdir` や、pkg-config のファイルのインストールなど）には、
/// 空の `Vec` を返す。
///
/// # Errors
///
/// ヘッダとそれ以外のファイルを、同じコマンドでインストールする構成だった場合。
/// 公開ヘッダだけを取り出せるという前提が外れている。
pub fn parse_line(line: &str) -> Result<Vec<InstalledHeader>> {
    // 引用符が閉じていない行は、コマンドとして解釈できないので対象外にする。
    let Some(words) = split_shell_words(line) else {
        return Ok(Vec::new());
    };

    // `install` のプログラムの名前は環境によって違いうる（`configure` が決める）
    // ので、名前ではなく、FFmpeg の Makefile が必ず付ける `-m 644` で見分ける。
    // この行は、進捗の表示（`printf "INSTALL\t%s\n" ...;`）に続けて書かれるので、
    // 行の途中から探す。
    let Some(mode) = words
        .windows(2)
        .position(|pair| pair[0] == "-m" && pair[1] == "644")
    else {
        return Ok(Vec::new());
    };

    // `-m 644` の後ろは、インストールするファイルが並び、最後がインストール先。
    let [sources @ .., destination] = &words[mode + 2..] else {
        return Ok(Vec::new());
    };

    let is_header = |source: &&String| source.ends_with(".h");

    if !sources.iter().any(|source| is_header(&source)) {
        return Ok(Vec::new());
    }

    if let Some(other) = sources.iter().find(|source| !is_header(source)) {
        return Err(format!(
            "公開ヘッダのみがインストールされることを想定していましたが、\
            現在の FFmpeg はヘッダとヘッダではないファイルが、\
            同じコマンドでインストールされるよう設定されています: {other}"
        )
        .into());
    }

    let library = destination
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("ヘッダのインストール先を読めません: {destination}"))?;

    Ok(sources
        .iter()
        .map(|source| InstalledHeader {
            source: source.clone(),
            library: library.to_owned(),
        })
        .collect())
}

/// インストールされるヘッダのうち、翻訳単位が実際に include したものを選ぶ。
/// 返すのは `#include` に書く名前（`libavutil/avutil.h` の形）で、名前の順。
///
/// `headers` と `generated` は、ビルドマニフェストの同じ名前の項目。
///
/// # インストールされるヘッダをそのまま使わない理由
///
/// インストールされるヘッダの集合は、`configure` の構成に関係なく、ほぼ一定に
/// なっている。そのため、今の構成では使えないヘッダが混ざる。
///
/// - この環境に無いヘッダを要求するもの。`libavcodec/d3d11va.h` は Windows の
///   `<d3d11.h>` を、`libavcodec/vdpau.h` は `<vdpau/vdpau.h>` を include する。
///   読もうとした時点でエラーになる。
/// - 実体がコンパイルされていないもの。`libavcodec/smpte_436m.h` は単独で
///   読めるが、`smpte_436m.c` は今の構成ではコンパイルされない。宣言だけが
///   見えて、呼ぶとリンクで失敗する。
///
/// 翻訳単位が include したヘッダに絞ると、どちらも外れる。include されたヘッダは、
/// 元のビルドで実際に読めたものである。また、実体の `.c` がコンパイルされて
/// いれば、その `.c` が自分のヘッダを include している。
///
/// ここで外れないのは、使われているヘッダの中に、この環境では実体の無い宣言が
/// 混ざる場合。`libavcodec/jni.h` は Android 専用の関数を無条件に宣言している。
/// これは宣言の単位の話なので、ヘッダの単位では扱えない。
///
/// # Errors
///
/// - ヘッダが、`#include` での名前と違う場所にある場合。名前とツリーのパスが
///   一致するという前提で、ビルドマニフェストには名前だけを書き出している。
/// - 選んだ結果が空の場合。コマンドの形についての前提が外れている。
pub fn select_public_headers(
    installed: &[InstalledHeader],
    ffmpeg_src: &Path,
    headers: &[String],
    generated: &[String],
) -> Result<Vec<String>> {
    let mut public = BTreeSet::new();

    for header in installed {
        let include_name = header.include_name();

        let (path, used) = match header.location(ffmpeg_src)? {
            Location::Tree(path) => {
                let used = headers.contains(&path);
                (path, used)
            }
            Location::Generated(path) => {
                let used = generated.contains(&path);
                (path, used)
            }
        };

        // インクルードディレクトリは、ツリーのルートと、生成物のディレクトリの
        // ルート。そこからの相対パスが名前と違うと、`#include` で見つからない。
        // サブディレクトリのヘッダがインストールされる構成になると、ここに当たる。
        if path != include_name {
            return Err(format!(
                "公開ヘッダ {include_name} が、名前と違う場所（{path}）にあります。\n\
                 `#include` での名前と、実際のパスを分けて持つ対応が必要です。"
            )
            .into());
        }

        if used {
            public.insert(include_name);
        }
    }

    if public.is_empty() {
        return Err(
            "make -n install-headers の出力から、翻訳単位が include した公開ヘッダを \
             1 つも読み取れませんでした"
                .into(),
        );
    }

    Ok(public.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(source: &str, library: &str) -> InstalledHeader {
        InstalledHeader {
            source: source.to_owned(),
            library: library.to_owned(),
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// 実際の `make -n install-headers` の 1 行の形。進捗の表示に続けて、同じ行に
    /// `install` のコマンドが書かれる。ツリーのヘッダは絶対パス、生成物は
    /// ビルドディレクトリからの相対パスで並ぶ。
    #[test]
    fn parses_header_install_command() {
        let line = "printf \"INSTALL\\t%s\\n\" work/FFmpeg/libavutil/avutil.h \
                    libavutil/avconfig.h; install -m 644 /work/FFmpeg/libavutil/avutil.h \
                    libavutil/avconfig.h \"/usr/local/include/libavutil\"";

        assert_eq!(
            parse_line(line).unwrap(),
            [
                header("/work/FFmpeg/libavutil/avutil.h", "libavutil"),
                header("libavutil/avconfig.h", "libavutil"),
            ],
        );
    }

    /// `install-headers` は、pkg-config のファイルもインストールする。ディレクトリを
    /// 作るコマンドも表示される。どちらもヘッダではないので、読み飛ばす。
    #[test]
    fn ignores_other_commands() {
        for line in [
            "mkdir -p \"/usr/local/include/libavutil\"",
            "printf \"INSTALL\\t%s\\n\" libavutil/libavutil.pc; install -m 644 \
             libavutil/libavutil.pc \"/usr/local/lib/pkgconfig\"",
            "install -m 644",
            "echo \"unterminated",
        ] {
            assert_eq!(parse_line(line).unwrap(), [], "{line}");
        }
    }

    /// ヘッダとそれ以外が同じコマンドに混ざっていたら、公開ヘッダだけを取り出せて
    /// いない。黙って片方を捨てずに止まること。
    #[test]
    fn rejects_headers_mixed_with_other_files() {
        let line =
            "install -m 644 libavutil/avconfig.h libavutil/libavutil.pc \"/x/include/libavutil\"";

        assert!(parse_line(line).is_err());
    }

    /// 翻訳単位が include したヘッダだけが残ること。ツリーのヘッダは `headers` と、
    /// 生成物は `generated` と突き合わせる。
    ///
    /// `d3d11va.h` は、インストールされるが、今の構成ではどの翻訳単位も include
    /// しないヘッダの例。Windows のヘッダを要求するので、残すと変換できない。
    #[test]
    fn selects_only_headers_that_units_include() {
        let installed = [
            header("/work/FFmpeg/libavcodec/avcodec.h", "libavcodec"),
            header("/work/FFmpeg/libavcodec/d3d11va.h", "libavcodec"),
            header("src/libavutil/avutil.h", "libavutil"),
            header("libavutil/avconfig.h", "libavutil"),
            header("libavutil/unused_generated.h", "libavutil"),
        ];

        let public = select_public_headers(
            &installed,
            Path::new("/work/FFmpeg"),
            &strings(&[
                "libavcodec/avcodec.h",
                "libavcodec/internal.h",
                "libavutil/avutil.h",
            ]),
            &strings(&["config.h", "libavutil/avconfig.h"]),
        )
        .unwrap();

        assert_eq!(
            public,
            [
                "libavcodec/avcodec.h",
                "libavutil/avconfig.h",
                "libavutil/avutil.h"
            ],
        );
    }

    /// サブディレクトリのヘッダがインストールされると、`install` がディレクトリの
    /// 直下にコピーするので、`#include` での名前とツリーのパスが食い違う。
    /// ビルドマニフェストには名前しか書き出さないので、食い違ったら止まること。
    #[test]
    fn rejects_header_whose_path_differs_from_include_name() {
        let installed = [header("/work/FFmpeg/libavcodec/aac/aac.h", "libavcodec")];
        let headers = strings(&["libavcodec/aac/aac.h"]);

        assert!(
            select_public_headers(&installed, Path::new("/work/FFmpeg"), &headers, &[]).is_err()
        );
    }

    #[test]
    fn rejects_header_outside_ffmpeg_tree() {
        let installed = [header("/opt/include/libavutil/avutil.h", "libavutil")];

        assert!(select_public_headers(&installed, Path::new("/work/FFmpeg"), &[], &[]).is_err());
    }

    /// 1 つも選べないのは、コマンドの読み取りの前提が外れたとき。空の一覧を
    /// 黙って書き出さないこと。
    #[test]
    fn rejects_empty_selection() {
        let installed = [header("/work/FFmpeg/libavcodec/d3d11va.h", "libavcodec")];

        assert!(select_public_headers(&installed, Path::new("/work/FFmpeg"), &[], &[]).is_err());
        assert!(select_public_headers(&[], Path::new("/work/FFmpeg"), &[], &[]).is_err());
    }
}
