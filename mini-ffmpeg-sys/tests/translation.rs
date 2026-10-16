//! ビルドスクリプトが変換した Rust のコードの中身を、文字列として確かめる。
//!
//! 他のテストは、変換後の関数を呼んで動作を見る。ここで見るのは、動作には
//! 現れない性質で、変換の指定（`build/translate.rs`）が効いているかどうか。
//! cinrs を更新して、項目の名前や既定値が変わったときに気づけるようにしている。

/// `libavutil/version.c` を変換したコード。
///
/// この翻訳単位は、関数 4 個と変数 1 個を定義するだけで、ヘッダの中身には実質
/// 何も依存していない。使われない項目が出力されていないかを見るのに向いている。
const VERSION_UNIT: &str = include_str!(concat!(env!("OUT_DIR"), "/libavutil/version.rs"));

/// 翻訳単位が使わない項目が、出力されていないこと。
///
/// `version.c` が include するヘッダには、`av_clip_c`（`static inline` 関数）、
/// `strcat`（宣言だけの関数）、`AVPixelFormat`（列挙型）があるが、`version.c` は
/// どれも使っていない。
///
/// この絞り込みが外れても、ビルドもリンクも通り、他のテストも通る。変わるのは
/// ビルドの時間だけで、出力が約 4 倍に増え、1 分以上長くなる。そのため、
/// 動作ではなく中身で確かめている。
#[test]
fn unreachable_header_items_are_not_generated() {
    for unused in ["av_clip_c", "strcat", "AVPixelFormat"] {
        assert!(!VERSION_UNIT.contains(unused), "{unused} が出力されている");
    }
}

/// 翻訳単位が定義する関数が、C の名前のシンボルとして出力されていること。
///
/// 絞り込みが行き過ぎて、必要な定義まで消えていないことの確認でもある。
#[test]
fn defined_functions_are_exported() {
    assert!(VERSION_UNIT.contains("no_mangle"));

    for defined in [
        "av_version_info",
        "avutil_version",
        "avutil_configuration",
        "avutil_license",
    ] {
        assert!(
            VERSION_UNIT.contains(defined),
            "{defined} が出力されていない"
        );
    }
}

/// 生成物に、ビルドしたマシンの絶対パスが入っていないこと。
///
/// cinrs は、渡されたパスをそのまま `__FILE__` の文字列にする。`mem.c` は
/// `av_assert0` で `__FILE__` を使うので、絶対パスで渡していれば、ここに現れる。
/// 絶対パスが入ると、利用者の実行ファイルに、ビルドしたマシンのパスが残る。
#[test]
fn source_paths_are_relative() {
    let mem_unit = include_str!(concat!(env!("OUT_DIR"), "/libavutil/mem.rs"));

    assert!(
        mem_unit.contains("FFmpeg/libavutil/"),
        "`__FILE__` の文字列が無い"
    );
    assert!(!mem_unit.contains(env!("CARGO_MANIFEST_DIR")));
}
