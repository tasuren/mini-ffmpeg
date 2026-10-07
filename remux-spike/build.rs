//! cinrs に、C のコードを変換する対象のターゲットを伝える。
//!
//! 理由は `mini-ffmpeg-sys` の `build.rs` と同じ。cinrs のマクロを使うクレートは、
//! それぞれが自分で渡す必要がある。

use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target = env::var("TARGET").expect("Cargo がビルドスクリプトに TARGET を設定する");
    println!("cargo:rustc-env=CINRS_TARGET={target}");
}
