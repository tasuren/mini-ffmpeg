//! Call cinrs-translated `libavutil/log.c` from Rust to verify it.
//!
//! `log.c` casts the callback function address to an integer (`uintptr_t`) and
//! stores it in an atomic variable. A function address cast to an integer cannot
//! appear in a Rust `static` initializer, so cinrs represents this variable as a
//! pointer. Verify that setting and invoking callbacks works with this
//! representation. cinrs 0.2.0 could not translate this initialization.
//!
//! `av_log` is variadic and performs formatting in C. This also checks that variadic
//! arguments passed from Rust travel correctly through the translated C code to
//! the C library's `vsnprintf`.

use std::{
    ffi::{CStr, VaList, c_char, c_int, c_void},
    ptr,
    sync::Mutex,
};

use mini_ffmpeg_sys::libavutil::log;

// `AV_LOG_ERROR` and `AV_LOG_INFO` from `libavutil/log.h`.
const AV_LOG_ERROR: c_int = 16;
const AV_LOG_INFO: c_int = 32;

/// Levels and formatted lines received by [`capture`].
static CAPTURED: Mutex<Vec<(c_int, String)>> = Mutex::new(Vec::new());

/// Rust callback passed to `av_log_set_callback`.
///
/// The C callback type is `void (*)(void *, int, const char *, va_list)`.
/// Delegate formatting to `av_log_format_line2` and record only the resulting line.
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
    let line = line
        .to_str()
        .expect("format and arguments are ASCII")
        .to_owned();

    CAPTURED.lock().unwrap().push((level, line));
}

/// The callback is process-global, so keep changes to its configuration in one test.
/// Separate tests could overwrite each other's configuration when run in parallel.
#[test]
fn av_log_calls_the_configured_callback() {
    unsafe {
        // The default callback writes to stderr, so do not inspect its output. This path
        // uses the initial value of the callback variable (the default callback's address).
        // Broken initialization would call an invalid address here and crash.
        log::av_log(
            ptr::null_mut(),
            AV_LOG_INFO,
            c"mini-ffmpeg-sys test: default callback %d\n".as_ptr(),
            1 as c_int,
        );

        log::av_log_set_callback(Some(capture));

        // Mix strings, integers, and floating-point values to verify that variadic
        // arguments are consumed in order.
        log::av_log(
            ptr::null_mut(),
            AV_LOG_INFO,
            c"captured: %s %d %.2f\n".as_ptr(),
            c"hello".as_ptr(),
            42 as c_int,
            1.5_f64,
        );
        log::av_log(ptr::null_mut(), AV_LOG_ERROR, c"error level\n".as_ptr());

        // Setting `NULL` disables callbacks. The C comparison against integer zero
        // becomes a pointer comparison against `NULL` after translation. Check that
        // the meaning is preserved.
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
