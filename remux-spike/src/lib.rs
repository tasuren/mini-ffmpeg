//! 取り込んだ FFmpeg が実際に動くかを確かめるための、仮のクレート。
//!
//! FFmpeg のツリーにあるサンプルの `doc/examples/remux.c` を、cinrs でそのまま
//! 取り込んでいる。このサンプルは、入力のコンテナからパケットを読み、再エンコード
//! せずに出力のコンテナへ書く。`ffmpeg -i in -c copy out` と同じ処理に当たる。
//!
//! `mini-ffmpeg-sys` とは別のクレートにしているのは、試行錯誤のたびに FFmpeg 本体の
//! マクロを展開し直さずに済ませるため。ここを変えても、再ビルドされるのは
//! このクレートだけ。

use std::{
    ffi::{CString, c_char, c_int},
    path::Path,
    ptr,
};

// `remux.c` は FFmpeg の関数を C のシンボルの名前で呼ぶので、Rust のコードからは
// `mini-ffmpeg-sys` を一度も参照しない。Rust は参照の無い依存クレートをリンク
// しないので、リンクだけを目的に宣言している。
extern crate mini_ffmpeg_sys as _;

mod remux_c {
    // `mini-ffmpeg-sys` の翻訳単位との違い。
    //
    // - `#pragma cinrs export` を付けない。ここの関数は Rust から呼ぶだけで、
    //   他の翻訳単位から参照されない。
    // - `HAVE_AV_CONFIG_H` などを定義せず、`config.h` も読まない。`remux.c` は
    //   FFmpeg を使う側のプログラムで、公開のヘッダだけを include する。
    // - `main` を別の名前に変える。Rust のプログラムの入口と衝突させず、普通の
    //   関数として呼べるようにするため。
    cinrs::c17! {
        #pragma cinrs system_include first

        #include "../../mini-ffmpeg-sys/src/cinrs_platform_fixups.h"

        #pragma cinrs include_path "../mini-ffmpeg-sys/FFmpeg_generated"
        #pragma cinrs include_path "../mini-ffmpeg-sys/FFmpeg"

        #define main remux_main

        #include "doc/examples/remux.c"
    }
}

/// `remux.c` の `main` を呼び、`input` のストリームを再エンコードせずに `output` へ
/// 書き出す。戻り値は `main` の終了コードで、成功なら 0。
///
/// 出力のフォーマットは、`remux.c` が `output` の拡張子から決める。
///
/// # Panics
///
/// パスに NUL 文字が含まれている場合。
pub fn remux(input: &Path, output: &Path) -> i32 {
    let arguments = [
        // `argv[0]`。`remux.c` は使い方の表示にしか使わない。
        CString::new("remux").unwrap(),
        path_to_c_string(input),
        path_to_c_string(output),
    ];

    // C の `main` は `char **argv` を受け取り、`argv[argc]` は `NULL` と決まって
    // いる。`remux.c` は引数を書き換えないが、型に合わせて可変のポインタにする。
    let mut argv: Vec<*mut c_char> = arguments
        .iter()
        .map(|argument| argument.as_ptr().cast_mut())
        .chain([ptr::null_mut()])
        .collect();

    unsafe { remux_c::remux_main(arguments.len() as c_int, argv.as_mut_ptr()) }
}

fn path_to_c_string(path: &Path) -> CString {
    CString::new(path.as_os_str().as_encoded_bytes()).expect("パスに NUL 文字が無い")
}
