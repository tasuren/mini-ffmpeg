//! `libavutil/time.c` を cinrs で取り込んだ結果を、Rust から呼んで確かめる。
//!
//! `time.c` はプラットフォームのヘッダ（`<sys/time.h>`、`<unistd.h>`）を使い、
//! `gettimeofday` や `clock_gettime`、`nanosleep` を呼ぶ。プラットフォームの
//! ヘッダを読めていて、その宣言から C ライブラリの関数へ正しくリンクできて
//! いることを、ここで確かめる。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mini_ffmpeg_sys::libavutil::time;

/// `av_gettime` は、UNIX エポックからのマイクロ秒を返す。Rust の時計と
/// 突き合わせて、単位と基準が合っていることを確かめる。
///
/// SDK のヘッダは `gettimeofday` などの宣言にシンボル名のラベルを付けている。
/// これを cinrs が誤ったシンボル名に変換すると、このテストはリンクで失敗する
/// （`cinrs_platform_fixups.h` を参照）。
#[test]
fn av_gettime_returns_microseconds_since_unix_epoch() {
    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let microseconds = unsafe { time::av_gettime() };
    let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();

    let time = Duration::from_micros(microseconds.try_into().unwrap());

    // `av_gettime` はマイクロ秒に切り捨てるので、下限だけ 1 マイクロ秒の余裕を持たせる。
    assert!(
        time + Duration::from_micros(1) >= before,
        "{time:?} < {before:?}"
    );
    assert!(time <= after, "{time:?} > {after:?}");
}

/// `config.h` で `HAVE_CLOCK_GETTIME` が有効なので、相対時刻は単調な時計から
/// 取られる。`#if` の分岐が `config.h` の通りに選ばれていることの確認。
#[test]
fn av_gettime_relative_is_monotonic() {
    assert_eq!(unsafe { time::av_gettime_relative_is_monotonic() }, 1);

    let first = unsafe { time::av_gettime_relative() };
    let second = unsafe { time::av_gettime_relative() };

    assert!(second >= first);
}

/// `av_usleep` が、指定した時間だけ実際に待つこと。`nanosleep` に渡す
/// `struct timespec` のレイアウトが食い違っていると、待たずに戻るか、
/// 見当違いの時間だけ待つことになる。
#[test]
fn av_usleep_sleeps_for_requested_duration() {
    let requested = Duration::from_millis(20);
    let started = Instant::now();

    let result = unsafe { time::av_usleep(requested.as_micros().try_into().unwrap()) };

    let elapsed = started.elapsed();

    assert_eq!(result, 0);
    assert!(elapsed >= requested, "{elapsed:?}");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}
