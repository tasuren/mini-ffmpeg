//! ビルド先のターゲット向けに変換した FFmpeg のコードがあることを確かめる。
//!
//! 変換後のコード（`targets/<ターゲット>/rust/`）はリポジトリに入れていないので、
//! クローンした直後には無い。そのままビルドすると、rustc が「`mod.rs` を
//! 読めない」というエラーを出すだけで、何をすれば良いかが分からない。先にここで
//! 止めて、生成のコマンドを伝える。

use std::{env, path::Path, process};

fn main() {
    // 指定しないと、パッケージ内のファイルが変わるたびに再実行される。
    println!("cargo:rerun-if-changed=build.rs");

    let target = env::var("TARGET").expect("Cargo がビルドスクリプトに TARGET を設定する");
    let root = Path::new("targets").join(&target).join("rust/mod.rs");

    // 生成し直したときや、生成物を消したときに、この検査をやり直させる。
    println!("cargo:rerun-if-changed={}", root.display());

    if root.is_file() {
        return;
    }

    println!(
        "cargo::error={target} 向けに変換した FFmpeg のコードがありません\
         （{} が見つかりません）。リポジトリのルートで cargo xtask ffmpeg-units を\
         実行してください。",
        root.display(),
    );

    process::exit(1);
}
