//! Passes the C translation target to the cinrs macro.
//!
//! During macro expansion, cinrs determines `sizeof`, struct layouts, and `#if`
//! values from the target data model. A proc-macro cannot query rustc for the
//! target, so the crate passes it through an environment variable.
//!
//! Without it, cinrs translates using the host data model. If this differs from
//! the target when cross-compiling, assertions in the expanded code cause
//! compilation to fail.
//!
//! This script also writes the macro invocation that translates `remux.c`.
//! FFmpeg's generated files (such as `config.h`) live in a target-specific
//! directory (`mini-ffmpeg-sys/FFmpeg_build/<target>`), and the macro's
//! `#pragma cinrs include_path` only accepts string literals. The paths are
//! therefore assembled here, where the target is known.

// The macro keeps its output private, but must obey the same source validation
// as mini-ffmpeg-sys before parsing any substituted platform declarations.
#[allow(dead_code)]
#[path = "../mini-ffmpeg-sys/build/platform_workarounds/mod.rs"]
mod platform_workarounds;

use cinrs_core::{CommandLineMacro, Dialect, Options, Standard, TargetModel, include::System};
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Track the directory so adding a platform module also reruns the build.
    println!("cargo:rerun-if-changed=../mini-ffmpeg-sys/build/platform_workarounds");

    let target = env::var("TARGET").expect("Cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=CINRS_TARGET={target}");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));

    let mut options = Options::with_dialect(Standard::C17, Dialect::Iso)
        .for_target(TargetModel::from_triple(&target).expect("cinrs supports the build target"));
    options.system_include = System::First;
    options.include_paths = vec![
        PathBuf::from(format!("../mini-ffmpeg-sys/FFmpeg_build/{target}")),
        PathBuf::from("../mini-ffmpeg-sys/FFmpeg"),
    ];

    let workarounds = platform_workarounds::WorkaroundSet::for_target(&target);
    workarounds.apply(&mut options);

    if workarounds.needs_validation() {
        let source = cinrs_core::preprocess_source(
            "remux.c".into(),
            "#define main remux_main\n#include \"doc/examples/remux.c\"\n".into(),
            &options,
        );
        workarounds
            .validate(&source)
            .unwrap_or_else(|error| panic!("{error}"));

        for header in source.headers {
            if !matches!(header.kind, cinrs_core::include::HeaderKind::Bundled) {
                println!("cargo:rerun-if-changed={}", header.name);
            }
        }
    }

    // Emit exactly the replacements used for validation, avoiding a second list
    // in a C header that could drift away from the sys crate's policy.
    let defines = options
        .macros
        .iter()
        .map(|definition| match definition {
            CommandLineMacro::Define(value) => {
                let (name, replacement) = value.split_once('=').unwrap_or((value, "1"));
                format!("#define {name} {replacement}\n")
            }
            CommandLineMacro::Undefine(_) => unreachable!("workarounds only define macros"),
        })
        .collect::<String>();

    fs::write(out_dir.join("remux_c.rs"), remux_c(&target, &defines))
        .expect("can write to OUT_DIR");
}

/// Builds the macro invocation that translates `remux.c`.
/// See `src/lib.rs` for an explanation of the settings.
///
/// Include paths are relative to this crate. Replacements come from the same
/// policy used to validate the C source before invoking the macro.
fn remux_c(target: &str, defines: &str) -> String {
    format!(
        r#"cinrs::c17! {{
    #pragma cinrs system_include first

    {defines}

    #pragma cinrs include_path "../mini-ffmpeg-sys/FFmpeg_build/{target}"
    #pragma cinrs include_path "../mini-ffmpeg-sys/FFmpeg"

    #define main remux_main

    #include "doc/examples/remux.c"
}}
"#
    )
}
