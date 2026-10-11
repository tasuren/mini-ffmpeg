//! cinrs で Rust に変換した FFmpeg の libav\* を置くクレート。
//!
//! FFmpeg の `.c` 1 つを 1 つの翻訳単位として、Rust のファイル 1 つに変換し、
//! ライブラリ名のモジュールの下に置いている。`libavutil/time.c` は
//! `libavutil::time` になる。
//!
//! 変換は `cargo xtask ffmpeg-units` が、このクレートのビルドより前に行う。
//! 変換が済んでいれば、ビルドに要るのは rustc だけで、cinrs も、FFmpeg の
//! ソースも、プラットフォームのヘッダも要らない。変換の指定の説明は、xtask の
//! `ffmpeg_units` にある。
//!
//! 変換はプラットフォームのヘッダを読んで行うので、結果はターゲットごとに違う。
//! 生成物は `targets/<ターゲット>/rust/` に分けて置き、このファイルの宣言で、
//! ビルド先に合うものを選んでいる。
//!
//! 生成物は、今はリポジトリに入れていない（`.gitignore` で除外している）。
//! 変換後のコードには、FFmpeg のコードに加えて、プラットフォームのヘッダの
//! 宣言やインライン関数も含まれる。それを再配布するときのライセンスの扱いが
//! 決まっていないため。クローンした直後は、先に `cargo xtask ffmpeg-units` を
//! 実行する必要がある。生成物が無ければ、`build.rs` がそう伝えて止まる。
//!
//! 翻訳単位どうしは、Rust のモジュールのパスではなく、C のシンボルの名前で
//! 互いを参照する。そのため、同じヘッダを読む翻訳単位が複数あっても、構造体などの
//! 型は翻訳単位ごとに別々の Rust の型になる。
//!
//! 各モジュールにあるのは、その翻訳単位が定義するものと、そこから使われるもの
//! だけ。ヘッダにある宣言や型でも、その翻訳単位が使っていなければ出力されない。
//!
//! クレートのディレクトリには、Rust のコードのほかに次のものがある。
//!
//! - `FFmpeg/`: FFmpeg のツリー。Git のサブモジュール。
//! - `FFmpeg_generated/`: FFmpeg の `configure` と `make` が生成したファイル。
//! - `cinrs_manifest.toml`: 取り込む対象の一覧。
//!
//! 後ろの 2 つは `cargo xtask ffmpeg-manifest` の生成物で、手では編集しない。
//! 3 つとも変換の入力で、このクレートのビルドには使わない。

// ターゲットを足したときは、ここに宣言を足す。`cargo xtask ffmpeg-units` が、
// 実行したマシンのターゲットの宣言があることを確かめる。
//
// 生成物は cinrs の側で整形してあり、rustfmt の整形とは一致しない。`cargo fmt` が
// 生成物を書き換えないように、`rustfmt::skip` を付けている。
#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
#[path = "../targets/aarch64-apple-darwin/rust/mod.rs"]
#[rustfmt::skip]
mod generated;

// 生成物が無いターゲットでは、理由を 1 つのエラーで伝える。空のモジュールを
// 置いているのは、下の再エクスポートが別のエラーを重ねて出さないようにするため。
#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
mod generated {
    compile_error!(
        "mini-ffmpeg-sys には、このターゲット向けに変換した FFmpeg のコードがありません。\
         対応しているターゲットは aarch64-apple-darwin です。"
    );
}

pub use generated::*;
