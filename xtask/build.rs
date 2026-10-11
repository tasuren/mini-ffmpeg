//! xtask 自身のコンパイル先のターゲットを、xtask のコードから読めるようにする。
//!
//! `ffmpeg-units` は、実行したマシンのプラットフォームのヘッダを読んで C を Rust に
//! 変換するので、生成物はそのマシンのターゲット向けになる。生成物の置き場所と
//! 変換の指定に、ターゲットトリプル（`aarch64-apple-darwin` など）が要る。
//!
//! 標準ライブラリから実行時に分かるのは `std::env::consts` のアーキテクチャと OS
//! までで、トリプルは分からない。Cargo がビルドスクリプトに渡す `TARGET` が、
//! xtask のコンパイル先のトリプルそのものなので、それを環境変数として埋め込む。

use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target = env::var("TARGET").expect("Cargo がビルドスクリプトに TARGET を設定する");
    println!("cargo:rustc-env=XTASK_HOST_TARGET={target}");
}
