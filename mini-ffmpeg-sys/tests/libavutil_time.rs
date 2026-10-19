//! Call cinrs-translated `libavutil/time.c` from Rust to verify it.
//!
//! `time.c` uses platform headers (`<sys/time.h>`, `<unistd.h>`) and calls
//! `gettimeofday`, `clock_gettime`, and `nanosleep`. Verify that platform headers
//! were read and their declarations link correctly to C library functions.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mini_ffmpeg_sys::libavutil::time;

/// `av_gettime` returns microseconds since the UNIX epoch. Compare against Rust's
/// clock to verify the unit and epoch.
///
/// macOS SDK headers attach symbol labels such as `__asm("_nanosleep")` to
/// `nanosleep` and other declarations. An incorrect cinrs translation of those
/// symbol names causes this test to fail linking. cinrs 0.2.0 translated them
/// with an extra leading `_`.
#[test]
fn av_gettime_returns_microseconds_since_unix_epoch() {
    let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let microseconds = unsafe { time::av_gettime() };
    let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();

    let time = Duration::from_micros(microseconds.try_into().unwrap());

    // `av_gettime` truncates to microseconds, so allow one microsecond only at the lower bound.
    assert!(
        time + Duration::from_micros(1) >= before,
        "{time:?} < {before:?}"
    );
    assert!(time <= after, "{time:?} > {after:?}");
}

/// `HAVE_CLOCK_GETTIME` is enabled in `config.h`, so relative time uses a monotonic
/// clock. Verify that `#if` branches follow `config.h`.
#[test]
fn av_gettime_relative_is_monotonic() {
    assert_eq!(unsafe { time::av_gettime_relative_is_monotonic() }, 1);

    let first = unsafe { time::av_gettime_relative() };
    let second = unsafe { time::av_gettime_relative() };

    assert!(second >= first);
}

/// Check that `av_usleep` actually waits for the requested duration. A mismatched
/// layout for `struct timespec` passed to `nanosleep` would cause an immediate
/// return or an incorrect sleep duration.
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
