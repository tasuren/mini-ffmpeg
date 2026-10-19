//! Determine the target CPU architecture of the compiler used by `configure`.
//!
//! With `--disable-asm`, `configure` sets `ARCH=c` and all `ARCH_*` values in
//! `config.h` to 0, so the build results do not reveal the architecture. It may also
//! differ from xtask's architecture, for example when using x64 `cl.exe` on ARM64
//! Windows.
//!
//! Preprocess a small source that branches on compiler-predefined macros using
//! FFmpeg's `%.i: %.c` Makefile rule. The rule runs
//! `$(CC) $(CCFLAGS) $(CC_E) $<`, preserving the compiler and flags chosen by
//! `configure`.

use crate::Result;

/// The source file to preprocess, placed in the build directory.
pub const PROBE_SOURCE: &str = "xtask-target-arch.c";

/// The output file from preprocessing [`PROBE_SOURCE`], used as a `make` target.
pub const PROBE_OUTPUT: &str = "xtask-target-arch.i";

/// Use Rust's `CARGO_CFG_TARGET_ARCH` names so the `triple` module can compare them
/// with the architecture in the target triple.
///
/// Put the names in string literals to prevent macro expansion. Bare identifiers
/// such as `x86_64` could match predefined macros in some compilers.
///
/// MSVC for ARM64EC also defines `_M_X64`, so exclude `_M_ARM64EC` first.
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

/// Extract the architecture name from the preprocessed [`PROBE`].
pub fn parse_preprocessed(output: &str) -> Result<String> {
    let arch = output
        .lines()
        .find(|line| line.contains("xtask_target_arch"))
        .and_then(|line| line.split('"').nth(1))
        .ok_or("preprocessed output does not contain xtask_target_arch")?;

    if arch.is_empty() {
        return Err("cannot identify the compiler target architecture".into());
    }

    Ok(arch.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `cl.exe -P -EP` omits line directives, whereas gcc and clang with `-E` emit them.
    /// Read either form.
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
