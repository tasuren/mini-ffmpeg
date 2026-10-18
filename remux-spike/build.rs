//! cinrs のマクロに、C のコードを変換する対象のターゲットを伝える。
//!
//! cinrs は、`sizeof` や構造体のレイアウト、`#if` の値を、マクロの展開時に
//! ターゲットのデータモデルから決める。ところが proc-macro は rustc にターゲットを
//! 問い合わせられないので、クレートの側から環境変数で渡す。
//!
//! 渡さない場合、cinrs はホストのデータモデルで変換する。クロスコンパイルで
//! ターゲットと食い違うと、展開されたコードの assert でコンパイルが失敗する。
//!
//! もう 1 つの仕事は、`remux.c` を取り込むマクロの呼び出しを書き出すこと。
//! FFmpeg の生成物（`config.h` など）はターゲットごとのディレクトリ
//! （`mini-ffmpeg-sys/FFmpeg_build/<ターゲット>`）にあり、マクロの
//! `#pragma cinrs include_path` は文字列のリテラルしか受け取れないので、
//! ターゲットを知っているここでパスを組み立てる。

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target = env::var("TARGET").expect("Cargo がビルドスクリプトに TARGET を設定する");
    println!("cargo:rustc-env=CINRS_TARGET={target}");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo が OUT_DIR を設定する"));

    fs::write(out_dir.join("remux_c.rs"), remux_c(&target)).expect("OUT_DIR に書き出せる");
}

/// `remux.c` を取り込むマクロの呼び出し。指定の意味は `src/lib.rs` の説明にある。
///
/// `include_path` の相対パスは、このクレートのルートが起点になる。
/// `cinrs_platform_fixups.h` は、マクロを書いた `.rs` ファイルの隣を最初に探す
/// 規則なので、書き出したファイルが `OUT_DIR` にあっても見つかるよう、`src` も
/// 探索パスに足している。
fn remux_c(target: &str) -> String {
    format!(
        r#"cinrs::c17! {{
    #pragma cinrs system_include first

    #pragma cinrs include_path "src"
    #include "cinrs_platform_fixups.h"

    #pragma cinrs include_path "../mini-ffmpeg-sys/FFmpeg_build/{target}"
    #pragma cinrs include_path "../mini-ffmpeg-sys/FFmpeg"

    #define main remux_main

    #include "doc/examples/remux.c"
}}
"#
    )
}
