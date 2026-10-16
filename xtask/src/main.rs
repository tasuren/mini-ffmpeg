//! mini-ffmpeg の開発用タスクをまとめたコマンド。
//!
//! `cargo xtask <タスク名>` で実行する（`.cargo/config.toml` のエイリアス）。
//!
//! ここにあるのはメンテナが開発時に使う処理だけで、利用者のビルドでは実行されない。
//! FFmpeg の `configure` や `make` を必要とする作業をここに閉じ込め、その結果を
//! コミットしておくことで、通常のビルドを Cargo だけで完結させるのが狙い。

mod ffmpeg_license;
mod ffmpeg_manifest;

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

/// ビルドマニフェスト。`mini-ffmpeg-sys` からの相対パス。
///
/// `ffmpeg-manifest` が書き出し、`mini-ffmpeg-sys` の `build.rs` が読む。
const MANIFEST_FILE: &str = "cinrs_manifest.toml";

const USAGE: &str = "\
使い方: cargo xtask <タスク名>

タスク:
  ffmpeg-manifest  FFmpeg を最小構成でビルドし、ビルドマニフェストと
                   生成された設定ファイルを mini-ffmpeg-sys に書き出す
  ffmpeg-license   ビルドマニフェストの入力ファイルを licensecheck で検査する
                   オプションは cargo xtask ffmpeg-license --help を参照

FFmpeg のバージョンや構成を変えたときに実行する。ビルドマニフェストの翻訳単位を
Rust に変換するのは mini-ffmpeg-sys の build.rs で、タスクとして実行する必要は無い。";

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

    let args: Vec<String> = args.collect();

    if task.as_deref() != Some("ffmpeg-license") && !args.is_empty() {
        eprintln!("error: 余分な引数です: {}\n\n{USAGE}", args.join(" "));
        return ExitCode::FAILURE;
    }

    let result = match task.as_deref() {
        Some("ffmpeg-license") => ffmpeg_license::run(&args),
        Some("ffmpeg-manifest") => ffmpeg_manifest::run(),

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
