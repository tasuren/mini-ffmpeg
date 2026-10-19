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

    let target = env::var("TARGET").expect("Cargo がビルドスクリプトに TARGET を設定する");
    println!("cargo:rustc-env=CINRS_TARGET={target}");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo が OUT_DIR を設定する"));

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
        .expect("OUT_DIR に書き出せる");
}

/// `remux.c` を取り込むマクロの呼び出し。指定の意味は `src/lib.rs` の説明にある。
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
