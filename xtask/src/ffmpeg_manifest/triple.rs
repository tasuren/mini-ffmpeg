//! Determine the target triple used to locate a build configuration.
//!
//! Configurations live in `FFmpeg_build/<triple>/`. `mini-ffmpeg-sys`'s `build.rs`
//! looks for a directory whose name exactly matches Cargo's `TARGET`, so the name
//! chosen here must be a Rust target triple.
//!
//! Neither `config.guess` nor `std::env::consts` is suitable. The former includes
//! OS versions, as in `x86_64-apple-darwin23.5.0`; the latter lacks the vendor and
//! environment (`gnu`, `musl`, `msvc`) needed to reconstruct triples such as
//! `aarch64-apple-darwin` or `x86_64-pc-windows-msvc`. The host triple reported by
//! `rustc` itself reliably provides a Rust triple.
//!
//! However, `rustc`'s host describes the machine running xtask and is independent of
//! the target of the compiler used by `configure`. If their architectures differ,
//! a configuration for another architecture would be saved under this triple.
//! Compare it against the compiler target detected by [`target_arch`](super::target_arch).

use std::{env, process::Command};

use crate::Result;

/// Return `rustc`'s host triple.
///
/// Like Cargo, use the `RUSTC` environment variable if set.
pub fn host_triple() -> Result<String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());

    let output = Command::new(&rustc)
        .arg("-vV")
        .output()
        .map_err(|error| format!("cannot run {}: {error}", rustc.to_string_lossy()))?;

    if !output.status.success() {
        return Err(format!("rustc -vV failed ({})", output.status).into());
    }

    parse_host(&String::from_utf8(output.stdout)?)
}

/// Extract the value of the `host:` line in `rustc -vV` output.
fn parse_host(output: &str) -> Result<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|host| host.trim().to_owned())
        .ok_or_else(|| "rustc -vV output has no host line".into())
}

/// Check that the triple's CPU architecture matches the compiler target architecture.
/// `compiler_arch` uses the names from [`target_arch`](super::target_arch)
/// (the values of `CARGO_CFG_TARGET_ARCH`).
pub fn check_arch_matches(triple: &str, compiler_arch: &str) -> Result<()> {
    if arch_of(triple) == compiler_arch {
        return Ok(());
    }

    Err(format!(
        "the rustc host ({triple}) and the target of the compiler used by configure \
         ({compiler_arch}) have different architectures.\n\
         The configuration would be saved as {triple}, so it cannot be generated with this mismatch. \
         Run this task with a Rust toolchain for the same architecture as the compiler."
    )
    .into())
}

/// Convert the first component of a triple to a `CARGO_CFG_TARGET_ARCH` name.
///
/// Triple prefixes such as `armv7`, `i686`, and `riscv64gc` are more specific, so
/// normalize them to the names returned by [`target_arch::PROBE`](super::target_arch::PROBE).
fn arch_of(triple: &str) -> &str {
    let first = triple.split('-').next().unwrap_or_default();

    match first {
        "i386" | "i486" | "i586" | "i686" => "x86",
        arch if arch.starts_with("riscv64") => "riscv64",
        // Exclude `arm64ec` first because it is a different architecture.
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

    /// On ARM64 Windows, using x64 `cl.exe` makes the compiler target differ from
    /// `rustc`'s host. Do not save a configuration for another architecture under
    /// the host triple.
    #[test]
    fn rejects_arch_mismatch() {
        assert!(check_arch_matches("aarch64-pc-windows-msvc", "aarch64").is_ok());
        assert!(check_arch_matches("aarch64-pc-windows-msvc", "x86_64").is_err());
        assert!(check_arch_matches("arm64ec-pc-windows-msvc", "aarch64").is_err());
    }
}
