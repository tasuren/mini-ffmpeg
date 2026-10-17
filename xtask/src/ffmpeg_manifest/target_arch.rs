//! `configure` が使うコンパイラの、ターゲットの CPU アーキテクチャの判定。
//!
//! `--disable-asm` の構成では、`configure` は `ARCH=c` とし、`config.h` の
//! `ARCH_*` をすべて 0 にする。そのため、ビルドの結果からはアーキテクチャが
//! 分からない。また、xtask 自身のアーキテクチャとも一致するとは限らない。
//! ARM64 の Windows で、x64 の `cl.exe` を使う場合などがある。
//!
//! そこで、コンパイラが事前に定義するマクロで分岐する小さなソースを、FFmpeg の
//! Makefile の `%.i: %.c` の規則でプリプロセスする。この規則は
//! `$(CC) $(CCFLAGS) $(CC_E) $<` なので、`configure` が決めたコンパイラとフラグが
//! そのまま使われる。

use crate::Result;

/// プリプロセスするソースのファイル名。ビルドディレクトリに置く。
pub const PROBE_SOURCE: &str = "xtask-target-arch.c";

/// [`PROBE_SOURCE`] をプリプロセスした結果のファイル名。`make` のターゲット。
pub const PROBE_OUTPUT: &str = "xtask-target-arch.i";

/// 名前は Rust の `CARGO_CFG_TARGET_ARCH` の値に揃える。ビルドスクリプトが、
/// ビルドマニフェストの値とそのまま比べるため。
///
/// 名前を文字列リテラルに入れているのは、マクロとして展開されないようにするため。
/// `x86_64` のような識別子のままだと、コンパイラによっては定義済みのマクロに
/// 当たりうる。
///
/// ARM64EC の MSVC は `_M_X64` も定義するので、`_M_ARM64EC` を先に除く。
pub const PROBE: &str = r#"#if defined(_M_ARM64EC)
const char *xtask_target_arch = "";
#elif defined(__x86_64__) || defined(_M_X64)
const char *xtask_target_arch = "x86_64";
#elif defined(__aarch64__) || defined(_M_ARM64)
const char *xtask_target_arch = "aarch64";
#elif defined(__i386__) || defined(_M_IX86)
const char *xtask_target_arch = "x86";
#elif defined(__arm__) || defined(_M_ARM)
const char *xtask_target_arch = "arm";
#elif defined(__riscv) && __riscv_xlen == 64
const char *xtask_target_arch = "riscv64";
#else
const char *xtask_target_arch = "";
#endif
"#;

/// [`PROBE`] をプリプロセスした結果から、アーキテクチャの名前を取り出す。
pub fn parse_preprocessed(output: &str) -> Result<String> {
    let arch = output
        .lines()
        .find(|line| line.contains("xtask_target_arch"))
        .and_then(|line| line.split('"').nth(1))
        .ok_or("プリプロセスの結果に xtask_target_arch がありません")?;

    if arch.is_empty() {
        return Err(
            "コンパイラのターゲットのアーキテクチャが、判定できるものではありません".into(),
        );
    }

    Ok(arch.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cl.exe -P -EP` は行番号の指示を出さないが、gcc や clang の `-E` は出す。
    /// どちらの形からも読めること。
    #[test]
    fn reads_arch_from_preprocessed_output() {
        let msvc = "\n\nconst char *xtask_target_arch = \"x86_64\";\n\n";
        let gcc = "# 1 \"xtask-target-arch.c\"\n# 1 \"<built-in>\" 1\n\
                   \n\nconst char *xtask_target_arch = \"aarch64\";\n";

        assert_eq!(parse_preprocessed(msvc).unwrap(), "x86_64");
        assert_eq!(parse_preprocessed(gcc).unwrap(), "aarch64");
    }

    #[test]
    fn rejects_unknown_arch() {
        assert!(parse_preprocessed("const char *xtask_target_arch = \"\";").is_err());
        assert!(parse_preprocessed("").is_err());
    }
}
