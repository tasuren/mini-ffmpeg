//! `libavutil/log.c` を cinrs で変換した結果を、Rust から呼んで確かめる。
//!
//! `log.c` は、コールバックの関数のアドレスを整数（`uintptr_t`）にキャストして、
//! アトミックな変数に保持している。関数のアドレスを整数にした値は、Rust の
//! `static` の初期化子に書けないので、cinrs はこの変数をポインタとして持つ形に
//! 変換する。コールバックの設定と呼び出しが、その形で正しく動くことをここで
//! 確かめる。cinrs 0.2.0 はこの初期化を変換できなかった。
//!
//! `av_log` は可変長引数の関数で、書式の展開まで C の側で行う。Rust から渡した
//! 可変長の引数が、取り込んだ C のコードを通って、C ライブラリの `vsnprintf`
//! まで正しく届くことの確認にもなっている。

use std::{
    ffi::{CStr, VaList, c_char, c_int, c_void},
    ptr,
    sync::Mutex,
};

use mini_ffmpeg_sys::libavutil::log;

// `libavutil/log.h` の `AV_LOG_ERROR` と `AV_LOG_INFO`。
const AV_LOG_ERROR: c_int = 16;
const AV_LOG_INFO: c_int = 32;

/// [`capture`] が受け取った、レベルと整形済みの行。
static CAPTURED: Mutex<Vec<(c_int, String)>> = Mutex::new(Vec::new());

/// `av_log_set_callback` に渡す、Rust で書いたコールバック。
///
/// C の側のコールバックの型は `void (*)(void *, int, const char *, va_list)`。
/// 書式の展開は `av_log_format_line2` に任せ、結果の行だけを記録する。
unsafe extern "C" fn capture(avcl: *mut c_void, level: c_int, format: *const c_char, args: VaList) {
    let mut line = [0 as c_char; 256];
    let mut print_prefix: c_int = 1;

    unsafe {
        log::av_log_format_line2(
            avcl,
            level,
            format,
            args,
            line.as_mut_ptr(),
            line.len() as c_int,
            &mut print_prefix,
        );
    }

    let line = unsafe { CStr::from_ptr(line.as_ptr()) };
    let line = line.to_str().expect("書式も引数も ASCII").to_owned();

    CAPTURED.lock().unwrap().push((level, line));
}

/// コールバックはプロセス全体で 1 つなので、設定を変える検証を 1 つのテストに
/// まとめている。テストを分けると、並列に実行されたときに互いの設定を上書きする。
#[test]
fn av_log_calls_the_configured_callback() {
    unsafe {
        // 既定のコールバック。標準エラー出力に書くので、内容までは確かめない。
        // コールバックを保持する変数の初期値（既定のコールバックのアドレス）を
        // 通る経路で、初期化が壊れていれば、ここで不正なアドレスを呼んで落ちる。
        log::av_log(
            ptr::null_mut(),
            AV_LOG_INFO,
            c"mini-ffmpeg-sys test: default callback %d\n".as_ptr(),
            1 as c_int,
        );

        log::av_log_set_callback(Some(capture));

        // 文字列、整数、浮動小数点数を混ぜて、可変長の引数が順に取り出される
        // ことを確かめる。
        log::av_log(
            ptr::null_mut(),
            AV_LOG_INFO,
            c"captured: %s %d %.2f\n".as_ptr(),
            c"hello".as_ptr(),
            42 as c_int,
            1.5_f64,
        );
        log::av_log(ptr::null_mut(), AV_LOG_ERROR, c"error level\n".as_ptr());

        // `NULL` を設定すると、何も呼ばれなくなる。C の側は整数の 0 との比較で、
        // 変換後はポインタの `NULL` との比較になる。意味が変わっていないこと。
        log::av_log_set_callback(None);
        log::av_log(ptr::null_mut(), AV_LOG_INFO, c"dropped\n".as_ptr());
    }

    assert_eq!(
        *CAPTURED.lock().unwrap(),
        [
            (AV_LOG_INFO, "captured: hello 42 1.50\n".to_owned()),
            (AV_LOG_ERROR, "error level\n".to_owned()),
        ],
    );
}
