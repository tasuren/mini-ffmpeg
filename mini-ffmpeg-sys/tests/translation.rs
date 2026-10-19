//! Inspect Rust code translated by the build script as text.
//!
//! Other tests call translated functions to check behavior. These tests inspect
//! properties not observable at runtime, verifying the settings in
//! `build/translate.rs`. They catch changes in option names or defaults when
//! cinrs is updated.

/// Translated code from `libavutil/version.c`.
///
/// This unit only defines four functions and one variable, with effectively no
/// dependencies on header contents. It is useful for checking that unused items
/// are omitted.
const VERSION_UNIT: &str = include_str!(concat!(env!("OUT_DIR"), "/libavutil/version.rs"));

/// Check that items unused by the translation unit are omitted.
///
/// Headers included by `version.c` contain `av_clip_c` (a `static inline` function),
/// `strcat` (a function declaration), and `AVPixelFormat` (an enum), but `version.c`
/// uses none of them.
///
/// Without this filtering, building, linking, and other tests still pass. Only build
/// time changes: output grows roughly fourfold and builds take over a minute longer.
/// Check the contents rather than runtime behavior for that reason.
#[test]
fn unreachable_header_items_are_not_generated() {
    for unused in ["av_clip_c", "strcat", "AVPixelFormat"] {
        assert!(!VERSION_UNIT.contains(unused), "{unused} was emitted");
    }
}

/// Check that defined functions are emitted as symbols with their C names.
///
/// This also checks that filtering does not remove required definitions.
#[test]
fn defined_functions_are_exported() {
    assert!(VERSION_UNIT.contains("no_mangle"));

    for defined in [
        "av_version_info",
        "avutil_version",
        "avutil_configuration",
        "avutil_license",
    ] {
        assert!(VERSION_UNIT.contains(defined), "{defined} was not emitted");
    }
}

/// Check that generated files do not contain absolute build-machine paths.
///
/// cinrs uses supplied paths unchanged in `__FILE__`. `mem.c` uses `__FILE__` in
/// `av_assert0`, so an absolute path would appear here and remain in users'
/// executables.
#[test]
fn source_paths_are_relative() {
    let mem_unit = include_str!(concat!(env!("OUT_DIR"), "/libavutil/mem.rs"));

    assert!(
        mem_unit.contains("FFmpeg/libavutil/"),
        "missing `__FILE__` string"
    );
    assert!(!mem_unit.contains(env!("CARGO_MANIFEST_DIR")));
}
