//! A temporary crate for checking that the translated FFmpeg actually works.
//!
//! A cinrs macro translates `doc/examples/remux.c` from the FFmpeg source tree
//! without modification. This example reads packets from the input container
//! and writes them to the output container without re-encoding, equivalent to
//! `ffmpeg -i in -c copy out`.
//!
//! The FFmpeg implementation (`mini-ffmpeg-sys`) is translated by a build script
//! that calls cinrs directly; this crate uses a cinrs macro instead. The two
//! link through C symbol names, so they work together despite using different
//! translation methods.
//!
//! Because the macro performs the translation, building this crate requires
//! the FFmpeg source tree and platform headers.

use std::{
    ffi::{CString, c_char, c_int},
    path::Path,
    ptr,
};

// `remux.c` calls FFmpeg functions by their C symbol names, so the Rust code
// never references `mini-ffmpeg-sys`. Rust does not link unreferenced dependency
// crates, so this declaration exists solely to ensure it is linked.
extern crate mini_ffmpeg_sys as _;

mod remux_c {
    // The build script writes the macro invocation to `OUT_DIR`. FFmpeg's
    // generated files live in target-specific directories, so writing their
    // paths into the macro requires knowing the target. The generated invocation
    // reproduces the settings corresponding to C compiler options, then includes
    // the entire `.c` file. The FFmpeg source is not modified.
    //
    // - Omit `#pragma cinrs export`: these functions are only called from Rust,
    //   not referenced by other translation units.
    // - Use `#pragma cinrs system_include first` to search platform headers before
    //   cinrs's bundled headers, as when translating the FFmpeg implementation.
    // - Platform replacements are generated from the sys crate's shared policy.
    //   The build script validates their uses before the macro translates C.
    // - Do not define `HAVE_AV_CONFIG_H` or similar macros: `remux.c` is a
    //   consumer of FFmpeg and only includes public headers.
    // - Rename `main` so it can be called as an ordinary function without
    //   conflicting with the Rust program's entry point.
    include!(concat!(env!("OUT_DIR"), "/remux_c.rs"));
}

/// Calls `main` from `remux.c` to write streams from `input` to `output` without
/// re-encoding. Returns the exit code from `main`, with 0 indicating success.
///
/// `remux.c` determines the output format from the extension of `output`.
///
/// # Panics
///
/// Panics if either path contains a NUL byte.
pub fn remux(input: &Path, output: &Path) -> i32 {
    let arguments = [
        // `argv[0]`: `remux.c` only uses it in the usage message.
        CString::new("remux").unwrap(),
        path_to_c_string(input),
        path_to_c_string(output),
    ];

    // C's `main` takes `char **argv`, with `argv[argc]` required to be `NULL`.
    // `remux.c` does not modify the arguments, but use mutable pointers to match
    // the parameter type.
    let mut argv: Vec<*mut c_char> = arguments
        .iter()
        .map(|argument| argument.as_ptr().cast_mut())
        .chain([ptr::null_mut()])
        .collect();

    unsafe { remux_c::remux_main(arguments.len() as c_int, argv.as_mut_ptr()) }
}

fn path_to_c_string(path: &Path) -> CString {
    CString::new(path.as_os_str().as_encoded_bytes()).expect("path contains no NUL bytes")
}
