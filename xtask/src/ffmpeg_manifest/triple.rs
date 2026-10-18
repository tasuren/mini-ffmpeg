//! 構成の置き場所に使う、ターゲットトリプルの決定。
//!
//! 構成は `FFmpeg_build/<トリプル>/` に置かれ、`mini-ffmpeg-sys` の `build.rs` が
//! Cargo の `TARGET` と文字列で一致するディレクトリを探す。そのため、ここで決める
//! 名前は、Rust のトリプルでなければならない。
//!
//! `config.guess` や `std::env::consts` から組み立てる方法は採らない。前者は
//! `x86_64-apple-darwin23.5.0` のように OS のバージョンを含み、後者は
//! `aarch64-apple-darwin` や `x86_64-pc-windows-msvc` のようなトリプルを復元する
//! のに必要なベンダーと環境（`gnu`、`musl`、`msvc`）を持たない。Rust のトリプルを
//! 確実に得られるのは、`rustc` 自身が名乗るホストのトリプルである。
//!
//! ただし、`rustc` のホストは xtask を動かしているマシンのもので、`configure` が
//! 使うコンパイラのターゲットとは別に決まる。両者のアーキテクチャが食い違うと、
//! 別のアーキテクチャの構成を、このトリプルの名前で保存してしまう。そこで、
//! [`target_arch`](super::target_arch) が調べたコンパイラのターゲットと突き合わせる。

use std::{env, process::Command};

use crate::Result;

/// `rustc` のホストのトリプルを返す。
///
/// Cargo と同じく環境変数 `RUSTC` があればそれを使う。
pub fn host_triple() -> Result<String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());

    let output = Command::new(&rustc)
        .arg("-vV")
        .output()
        .map_err(|error| format!("{} を実行できません: {error}", rustc.to_string_lossy()))?;

    if !output.status.success() {
        return Err(format!("rustc -vV が失敗しました（{}）", output.status).into());
    }

    parse_host(&String::from_utf8(output.stdout)?)
}

/// `rustc -vV` の出力から、`host:` の行の値を取り出す。
fn parse_host(output: &str) -> Result<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_owned())
        .ok_or_else(|| "rustc -vV の出力に host の行がありません".into())
}

/// トリプルの CPU アーキテクチャが、コンパイラのターゲットのアーキテクチャと
/// 一致することを確かめる。`compiler_arch` は [`target_arch`](super::target_arch) の
/// 名前（`CARGO_CFG_TARGET_ARCH` の値）。
pub fn check_arch_matches(triple: &str, compiler_arch: &str) -> Result<()> {
    if arch_of(triple) == compiler_arch {
        return Ok(());
    }

    Err(format!(
        "rustc のホスト（{triple}）と、configure が使うコンパイラのターゲット\
         （{compiler_arch}）のアーキテクチャが違います。\n\
         構成は {triple} の名前で保存されるので、食い違ったままでは生成できません。\
         コンパイラと同じアーキテクチャの Rust のツールチェーンで実行してください。"
    )
    .into())
}

/// トリプルの先頭の要素を、`CARGO_CFG_TARGET_ARCH` の名前に直す。
///
/// トリプルの先頭は `armv7`、`i686`、`riscv64gc` のように細かいので、
/// [`target_arch::PROBE`](super::target_arch::PROBE) が返す名前に寄せる。
fn arch_of(triple: &str) -> &str {
    let first = triple.split('-').next().unwrap_or_default();

    match first {
        "i386" | "i486" | "i586" | "i686" => "x86",
        arch if arch.starts_with("riscv64") => "riscv64",
        // `arm64ec` は別のアーキテクチャなので、先に除く。
        arch if arch.starts_with("arm64ec") => arch,
        arch if arch.starts_with("arm") || arch.starts_with("thumb") => "arm",
        arch => arch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_host_line() {
        let output = "rustc 1.99.0 (abcdef 2026-01-01)\nbinary: rustc\nhost: aarch64-apple-darwin\nrelease: 1.99.0\n";

        assert_eq!(parse_host(output).unwrap(), "aarch64-apple-darwin");
        assert!(parse_host("rustc 1.99.0\n").is_err());
    }

    #[test]
    fn maps_triple_arch_to_probe_names() {
        for (triple, arch) in [
            ("aarch64-apple-darwin", "aarch64"),
            ("x86_64-pc-windows-msvc", "x86_64"),
            ("i686-pc-windows-msvc", "x86"),
            ("armv7-unknown-linux-gnueabihf", "arm"),
            ("thumbv7em-none-eabihf", "arm"),
            ("riscv64gc-unknown-linux-gnu", "riscv64"),
        ] {
            assert_eq!(arch_of(triple), arch, "{triple}");
        }
    }

    /// ARM64 の Windows で x64 の `cl.exe` を使うと、`rustc` のホストと
    /// コンパイラのターゲットが食い違う。別のアーキテクチャの構成を、トリプルの
    /// 名前で保存しないこと。
    #[test]
    fn rejects_arch_mismatch() {
        assert!(check_arch_matches("aarch64-pc-windows-msvc", "aarch64").is_ok());
        assert!(check_arch_matches("aarch64-pc-windows-msvc", "x86_64").is_err());
        assert!(check_arch_matches("arm64ec-pc-windows-msvc", "aarch64").is_err());
    }
}
