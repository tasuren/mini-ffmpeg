//! cinrs で Rust に取り込んだ FFmpeg の libav\* を置くクレート。
//!
//! FFmpeg の `.c` 1 つを 1 つの翻訳単位として、ライブラリ名のモジュールの下に
//! 取り込む。`libavutil/time.c` は `libavutil::time` になる。
//!
//! 翻訳単位のファイル（`libavutil/time.rs` など）と、ライブラリごとのモジュールの
//! 宣言（`libavutil.rs` など）は、`cargo xtask ffmpeg-units` の生成物で、先頭に
//! `// @generated` の印がある。取り込みの形の説明は、xtask の `ffmpeg_units.rs`
//! にある。印が無いファイルは手書きで、次のものがある。
//!
//! - このファイル。
//! - `libavutil/log.rs` と `libavutil/log.c`: cinrs でそのまま変換できない
//!   翻訳単位の、手書きの取り込みと、変更を加えたソースのコピー。
//! - `cinrs_platform_fixups.h` と `cinrs_ffmpeg_fixups.h`: 全翻訳単位が読む、
//!   cinrs のための回避策。
//!
//! 翻訳単位どうしは、Rust のモジュールのパスではなく、C のシンボルの名前で
//! 互いを参照する（`#pragma cinrs export`）。そのため、同じヘッダを読む翻訳
//! 単位が複数あっても、構造体などの型は翻訳単位ごとに別々の Rust の型になる。
//!
//! クレートのディレクトリには、Rust のコードのほかに次のものがある。
//!
//! - `FFmpeg/`: FFmpeg のツリー。Git のサブモジュール。
//! - `FFmpeg_generated/`: FFmpeg の `configure` と `make` が生成したファイル。
//! - `cinrs_manifest.toml`: 取り込む対象の一覧。
//!
//! 後ろの 2 つは `cargo xtask ffmpeg-manifest` の生成物で、手では編集しない。

// ライブラリを増減したときは、ここを手で合わせる。`cargo xtask ffmpeg-units` が、
// 一覧にあるライブラリがすべて宣言されていることを確かめる。
pub mod libavcodec;
pub mod libavformat;
pub mod libavutil;
