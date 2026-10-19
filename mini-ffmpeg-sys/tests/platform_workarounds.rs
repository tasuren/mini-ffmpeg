//! Regression tests for the build script's platform declaration policy.

#[path = "../build/platform_workarounds/mod.rs"]
mod platform_workarounds;

use cinrs_core::{
    Dialect, FilePreprocessing, Header, Options, Standard, TargetModel, include::HeaderKind,
};
use platform_workarounds::WorkaroundSet;

fn macos() -> WorkaroundSet {
    WorkaroundSet::for_target("aarch64-apple-darwin")
}

fn options() -> Options {
    Options::with_dialect(Standard::C17, Dialect::Iso)
        .for_target(TargetModel::from_triple("aarch64-apple-darwin").unwrap())
}

/// Model an SDK header without depending on the host's installed SDK version.
fn header(source: &str) -> FilePreprocessing {
    let mut opts = options();
    macos().apply(&mut opts);
    let mut preprocessed = cinrs_core::preprocess_source("math.h".into(), source.into(), &opts);
    let file = preprocessed.map.files()[0].id();
    preprocessed.headers.push(Header {
        name: "math.h".into(),
        kind: HeaderKind::System,
        file,
        included_from: file,
    });
    preprocessed
}

#[test]
fn accepts_and_removes_known_sdk_prototypes() {
    let source = "extern _Float16 __fabsf16(_Float16);\n\
        extern _Float16 __hypotf16(_Float16, _Float16);\n\
        extern _Float16 __fmaf16(_Float16, _Float16, _Float16)\n\
        __attribute__((availability(macos,introduced=15.0)));\n\
        extern float sqrtf(float);";
    macos().validate(&header(source)).unwrap();
    let mut opts = options();
    opts.complex = false;
    macos().apply(&mut opts);
    let generated = cinrs_core::translate_source("math.h".into(), source.into(), &opts);
    assert!(!generated.diagnostics.has_errors());
    let output = macos()
        .exclude_declarations(generated.items.unwrap())
        .unwrap()
        .to_string();

    assert!(!output.contains("__fabsf16"));
    assert!(!output.contains("__hypotf16"));
    assert!(!output.contains("__fmaf16"));
    assert!(output.contains("sqrtf"));
}

/// Matching a function name alone must not permit ABI changes or half fields,
/// callbacks, locals, definitions, and extra declarators sharing a prototype.
#[test]
fn rejects_unknown_or_changed_uses() {
    for source in [
        "extern _Float16 new_half(_Float16);",
        "extern float __sqrtf16(_Float16);",
        "extern _Float16 __sqrtf16(_Float16, _Float16);",
        "extern _Float16 __sqrtf16(_Float16 value);",
        "extern _Float16 __sqrtf16(_Float16), other_half(_Float16);",
        "struct S { _Float16 value; };",
        "typedef _Float16 (*Callback)(_Float16);",
        "void local(void) { _Float16 value; }",
        "_Float16 __sqrtf16(_Float16 x) { return x; }",
        "extern _Float16 __sqrtf16(_Float16) __attribute__((unused(sizeof(_Float16))));",
    ] {
        assert!(
            macos().validate(&header(source)).is_err(),
            "accepted {source}"
        );
    }
}

/// Preprocessing also exposes calls through macros. Reject them even in unused
/// inline bodies; deleting their foreign declaration cannot make them correct.
#[test]
fn rejects_references_to_excluded_functions() {
    let source = "extern _Float16 __sqrtf16(_Float16);\n\
        #define ROOT __sqrtf16\n\
        static inline float use_half(float x) { return ROOT(x); }";
    assert!(macos().validate(&header(source)).is_err());
}

#[test]
fn ignores_inactive_uses_and_preserves_macro_expanded_types() {
    macos()
        .validate(&header("#if 0\n_Float16 ignored;\n#endif"))
        .unwrap();
    assert!(
        macos()
            .validate(&header("#define HALF _Float16\nHALF exposed;"))
            .is_err()
    );
}

/// The replacement itself can affect #ifdef branches. Validate with the same
/// macro definitions as translation, recovering _Float16 from expansion origins.
#[test]
fn checks_the_translation_branch_and_nested_macro_origins() {
    let source = "#ifdef _Float16\n#define HALF _Float16\n#define TYPE HALF\nstruct Exposed { TYPE value; };\n#endif";
    let preprocessed = header(source);
    assert!(!preprocessed.diagnostics.has_errors());
    assert!(macos().validate(&preprocessed).is_err());

    macos()
        .validate(&header(
            "#define HALF _Float16\nextern HALF __sqrtf16(HALF);",
        ))
        .unwrap();
}

/// An application declaration must not impersonate an approved SDK prototype.
#[test]
fn rejects_non_system_prototypes() {
    let mut source = header("extern _Float16 __sqrtf16(_Float16);");
    source.headers[0].kind = HeaderKind::User;
    assert!(macos().validate(&source).is_err());
}

#[test]
fn removes_attributes_and_renamed_foreign_symbols_only() {
    let tokens = "mod unit { unsafe extern \"C\" {\n\
        #[link_name = \"__sqrtf16\"] pub fn renamed(x: f32) -> f32;\n\
        #[link_name = \"sqrtf\"] pub fn sqrtf(x: f32) -> f32;\n\
        pub static count: i32; } }"
        .parse()
        .unwrap();
    let filtered = macos().exclude_declarations(tokens).unwrap().to_string();
    assert!(!filtered.contains("renamed"));
    assert!(!filtered.contains("__sqrtf16"));
    assert!(filtered.contains("link_name = \"sqrtf\""));
    assert!(filtered.contains("count"));
}

/// Reachable implementation declarations indicate actual dependencies. Public
/// inline references must likewise fail rather than survive declaration removal.
#[test]
fn rejects_remaining_generated_dependencies() {
    let declaration: proc_macro2::TokenStream =
        "unsafe extern \"C\" { pub fn __sqrtf16(x: f32) -> f32; }"
            .parse()
            .unwrap();
    assert!(macos().check_references(declaration.clone()).is_err());
    let caller: proc_macro2::TokenStream = "unsafe fn caller(x: f32) -> f32 { __sqrtf16(x) }"
        .parse()
        .unwrap();
    let filtered = macos()
        .exclude_declarations(declaration.into_iter().chain(caller).collect())
        .unwrap();
    assert!(macos().check_references(filtered).is_err());
}

#[test]
fn does_not_apply_workarounds_to_x86_64_linux() {
    let linux = WorkaroundSet::for_target("x86_64-unknown-linux-gnu");
    let mut opts = options();
    assert!(!linux.needs_validation());
    assert!(macos().needs_validation());
    let before = opts.macros.len();
    linux.apply(&mut opts);
    assert_eq!(opts.macros.len(), before);
    linux.validate(&header("_Float16 value;")).unwrap();
    let tokens: proc_macro2::TokenStream =
        "unsafe extern \"C\" { fn __sqrtf16(); }".parse().unwrap();
    assert_eq!(
        linux
            .exclude_declarations(tokens.clone())
            .unwrap()
            .to_string(),
        tokens.to_string()
    );
    linux.check_references(tokens).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn generated_public_api_contains_no_half_math_symbols() {
    let public_api = include_str!(concat!(env!("OUT_DIR"), "/public_api.rs"));
    macos()
        .check_references(public_api.parse().unwrap())
        .unwrap();
    assert!(public_api.contains("avformat_open_input"));
}

fn linux() -> WorkaroundSet {
    WorkaroundSet::for_target("aarch64-unknown-linux-gnu")
}

fn vector_header(source: &str) -> FilePreprocessing {
    let mut opts = Options::with_dialect(Standard::C17, Dialect::Iso)
        .for_target(TargetModel::from_triple("aarch64-unknown-linux-gnu").unwrap());
    linux().apply(&mut opts);
    let mut preprocessed =
        cinrs_core::preprocess_source("math-vector.h".into(), source.into(), &opts);
    let file = preprocessed.map.files()[0].id();
    preprocessed.headers.push(Header {
        name: "bits/math-vector.h".into(),
        kind: HeaderKind::System,
        file,
        included_from: file,
    });
    preprocessed
}

const VECTOR_TYPES: &str = "typedef __Float32x4_t __f32x4_t;
    typedef __Float64x2_t __f64x2_t;
    typedef __SVFloat32_t __sv_f32_t;
    typedef __SVFloat64_t __sv_f64_t;
    typedef __SVBool_t __sv_bool_t;";

#[test]
fn accepts_original_neon_and_sve_signatures() {
    let source = format!(
        "{VECTOR_TYPES}
        __attribute__((__aarch64_vector_pcs__)) __f32x4_t _ZGVnN4vv_atan2f(__f32x4_t, __f32x4_t);
        __attribute__((__aarch64_vector_pcs__)) __f64x2_t _ZGVnN2v_erf(__f64x2_t);
        __sv_f32_t _ZGVsMxv_erff(__sv_f32_t, __sv_bool_t);
        __sv_f64_t _ZGVsMxv_erf(__sv_f64_t, __sv_bool_t);
        __sv_f64_t _ZGVsMxvv_pow(__sv_f64_t, __sv_f64_t, __sv_bool_t);"
    );
    linux().validate(&vector_header(&source)).unwrap();
}

/// Builtin replacements may only feed the reviewed typedefs. Alias use in a
/// record, callback, scalar API, or inline caller would change the actual ABI.
#[test]
fn rejects_new_vector_uses_and_signature_changes() {
    for declaration in [
        "typedef __SVFloat32_t Other;",
        "struct S { __f32x4_t value; };",
        "extern __sv_f32_t av_vector(__sv_f32_t);",
        "typedef __sv_f32_t (*Callback)(__sv_f32_t);",
        "__sv_f32_t _ZGVsMxv_unknown(__sv_f32_t, __sv_bool_t);",
        "__sv_f32_t _ZGVsMxv_cosf(__sv_f32_t);",
        "__sv_f32_t _ZGVsMxv_cosf(__sv_f32_t, int);",
        "__sv_f64_t _ZGVsMxv_cosf(__sv_f64_t, __sv_bool_t);",
        "__sv_f32_t _ZGVsMxv_cosf(__sv_f32_t, __sv_bool_t, __sv_bool_t);",
        "__f32x4_t _ZGVnN4v_cosf(__f32x4_t);",
        "__attribute__((__aarch64_vector_pcs__)) __f32x4_t _ZGVnN4v_cosf(__f64x2_t);",
        "void caller(void) { _ZGVsMxv_cosf(0, 0); }",
    ] {
        let source = format!("{VECTOR_TYPES}\n{declaration}");
        assert!(
            linux().validate(&vector_header(&source)).is_err(),
            "accepted {declaration}"
        );
    }
}

#[test]
fn rejects_vector_declarations_from_application_headers() {
    let mut source = vector_header(VECTOR_TYPES);
    source.headers[0].kind = HeaderKind::User;
    assert!(linux().validate(&source).is_err());
}

/// Scalar stand-ins must not remain available as Rust type aliases, even when
/// the function declarations themselves have already been removed.
#[test]
fn removes_vector_aliases_attributes_and_renamed_functions() {
    let tokens = "mod unit {
        #[doc = \"temporary\"] pub type __f32x4_t = f32;
        pub type __sv_f32_t = f32;
        pub type __sv_bool_t = i32;
        pub type Kept = f32;
        unsafe extern \"C\" {
            #[link_name = \"_ZGVsMxv_cosf\"] pub fn renamed(x: f32, mask: i32) -> f32;
            pub fn _ZGVnN4vv_atan2f(x: f32, y: f32) -> f32;
            pub fn cosf(x: f32) -> f32;
        }
    }"
    .parse()
    .unwrap();
    let filtered = linux().exclude_declarations(tokens).unwrap();
    linux().check_references(filtered.clone()).unwrap();
    let output = filtered.to_string();
    for removed in [
        "__f32x4_t",
        "__sv_f32_t",
        "__sv_bool_t",
        "temporary",
        "renamed",
        "_ZGV",
    ] {
        assert!(!output.contains(removed), "retained {removed}");
    }
    assert!(output.contains("Kept"));
    assert!(output.contains("cosf"));
}

#[test]
fn rejects_retained_vector_dependencies() {
    for source in [
        "unsafe extern \"C\" { fn _ZGVsMxv_cosf(x: f32, mask: i32) -> f32; }",
        "struct Exposed { value: __f32x4_t }",
        "unsafe fn caller() { _ZGVnN4v_cosf(0.0); }",
    ] {
        assert!(linux().check_references(source.parse().unwrap()).is_err());
    }
}

#[test]
fn vector_rules_are_limited_to_aarch64_glibc() {
    assert!(linux().needs_validation());
    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-musl",
        "aarch64-pc-windows-msvc",
    ] {
        let rules = WorkaroundSet::for_target(target);
        assert!(!rules.needs_validation(), "enabled for {target}");
        let mut opts = options();
        let count = opts.macros.len();
        rules.apply(&mut opts);
        assert_eq!(opts.macros.len(), count);
    }
}

#[cfg(all(target_arch = "aarch64", target_os = "linux", target_env = "gnu"))]
#[test]
fn generated_public_api_contains_no_vector_stand_ins() {
    let source = include_str!(concat!(env!("OUT_DIR"), "/public_api.rs"));
    linux().check_references(source.parse().unwrap()).unwrap();
    assert!(!source.contains("_ZGV"));
    for alias in [
        "__f32x4_t",
        "__f64x2_t",
        "__sv_f32_t",
        "__sv_f64_t",
        "__sv_bool_t",
    ] {
        assert!(!source.contains(alias));
    }
    assert!(source.contains("avformat_open_input"));
}

/// Preserve the original builtin even when replacement selects an #ifdef branch
/// and the type reaches it through several layers of macro expansion.
#[test]
fn checks_vector_macro_origins_and_active_branches() {
    let source = "#ifdef __Float32x4_t\n#define INNER __Float32x4_t\n#define OUTER INNER\nstruct S { OUTER value; };\n#endif";
    assert!(linux().validate(&vector_header(source)).is_err());
    linux()
        .validate(&vector_header(
            "#define V __Float32x4_t\ntypedef V __f32x4_t;\n#if 0\n__SVFloat32_t ignored;\n#endif",
        ))
        .unwrap();
}
