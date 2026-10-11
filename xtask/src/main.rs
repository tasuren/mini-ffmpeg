//! mini-ffmpeg の開発用タスクをまとめたコマンド。
//!
//! `cargo xtask <タスク名>` で実行する（`.cargo/config.toml` のエイリアス）。
//!
//! ここにあるのは開発時に使う処理だけで、通常のビルドでは実行されない。
//! FFmpeg の `configure` や `make` を必要とする作業と、C から Rust への変換を
//! ここに閉じ込め、通常のビルドを Cargo だけで完結させるのが狙い。
//!
//! `ffmpeg-manifest` の結果はコミットしてある。`ffmpeg-units` の結果（変換後の
//! Rust のコード）は、今はコミットしていないので、クローンした直後に一度
//! 実行する必要がある。理由は `ffmpeg_units` の説明にある。

mod ffmpeg_manifest;
mod ffmpeg_units;

use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
};

/// タスク全体で使う `Result`。
///
/// 開発用のコマンドであり、エラーは表示して終了するだけなので、型を分けずに
/// 文字列ベースで扱う。
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// 取り込む対象の一覧。`mini-ffmpeg-sys` からの相対パス。
///
/// `ffmpeg-manifest` が書き出し、`ffmpeg-units` が読む。
const MANIFEST_FILE: &str = "cinrs_manifest.toml";

/// FFmpeg のツリー（サブモジュール）。`mini-ffmpeg-sys` からの相対パス。
const FFMPEG_DIR: &str = "FFmpeg";

/// xtask を実行しているマシンのターゲットトリプル。`build.rs` が埋め込む。
const HOST_TARGET: &str = env!("XTASK_HOST_TARGET");

const USAGE: &str = "\
使い方: cargo xtask <タスク名>

タスク:
  ffmpeg-manifest  FFmpeg を最小構成でビルドし、取り込む対象の一覧と
                   生成された設定ファイルを mini-ffmpeg-sys に書き出す
  ffmpeg-units     一覧の翻訳単位を cinrs で Rust のコードに変換し、
                   mini-ffmpeg-sys/targets/<ターゲット> に書き出す

FFmpeg のバージョンや構成を変えたときは、この順に両方を実行する。";

/// ワークスペースのルート。
fn workspace_root() -> Result<&'static Path> {
    // xtask クレートのディレクトリの 1 つ上が、ワークスペースのルート。
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "ワークスペースのルートが見つかりません".into())
}

/// `mini-ffmpeg-sys` クレートのディレクトリ。タスクの結果は、この中に書き出す。
fn sys_crate_dir() -> Result<PathBuf> {
    Ok(workspace_root()?.join("mini-ffmpeg-sys"))
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let task = args.next();

    // 引数を取るタスクはまだ無い。余分な引数を黙って捨てると、指定したつもりの
    // 内容が効いていないことに気づけないので、エラーにする。
    if let Some(extra) = args.next() {
        eprintln!("error: 余分な引数です: {extra}\n\n{USAGE}");
        return ExitCode::FAILURE;
    }

    let result = match task.as_deref() {
        Some("ffmpeg-manifest") => ffmpeg_manifest::run(),
        Some("ffmpeg-units") => ffmpeg_units::run(),

        Some("help" | "--help" | "-h") | None => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }

        Some(other) => Err(format!("未知のタスクです: {other}\n\n{USAGE}").into()),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
