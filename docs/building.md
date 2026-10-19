# Building and cross-compiling

These instructions build the Rust crates using the committed FFmpeg
configurations. To change those configurations, see
[Generating build configurations](build-configurations.md).

Run commands from the repository root.

## Prerequisites

- Rust 1.99 or newer, including the standard library for the build target.
- The FFmpeg source submodule: `git submodule update --init`.
- C library headers for the target: the macOS SDK or a matching Linux/glibc
  development environment/sysroot.
- The linker and runtime libraries required by your Rust target. Cross-compiling
  may require a cross linker; running tests also requires a compatible machine
  or runner.

The build script translates C with cinrs. Normal crate builds do not compile C
sources or run FFmpeg's `configure` or `make`, and do not require an installed
FFmpeg library. A compiler driver may still provide the linker used by Rust.

On native macOS and Linux builds, cinrs discovers platform headers. On macOS,
install Xcode or Command Line Tools and select the intended developer directory.
On Linux, install the development headers for the target's glibc.

## Verified configurations

The following records describe successful checks, not a guarantee for every SDK,
libc version, or deployment environment. The project remains experimental.

| Target | Verification |
| --- | --- |
| `aarch64-apple-darwin` | Native workspace tests, including WAV copying. |
| `x86_64-apple-darwin` | Cross-build on Apple Silicon; sys tests have also been run under Rosetta. |
| `aarch64-unknown-linux-gnu` | Workspace tests and WAV copying in Ubuntu 26.04/glibc 2.43 on Docker arm64; sys tests also confirmed in Parallels Ubuntu arm64. |
| `x86_64-unknown-linux-gnu` | Sys tests cross-compiled on Ubuntu arm64 and executed with QEMU. |
| `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc` | Maintainer-generated FFmpeg configurations only; crate builds are blocked by cinrs's lack of support for the required MSVC dialect. |

The cross-compilation examples below were confirmed by the maintainer. Adapt
paths and toolchain availability to your installation. There are currently no
recorded musl or Windows/GNU configurations.

## Native build

```sh
git submodule update --init # if you have never done it
cargo test --workspace
```

For the low-level crate alone:

```sh
cargo test -p mini-ffmpeg-sys
```

## Cross-compilation rules

`mini-ffmpeg-sys` selects `FFmpeg_build/<TARGET>/` and verifies that the manifest's
triple matches Cargo's target. The directory contains a complete configuration,
including `config.h`; renaming another target's directory does not retarget it.

Set `CINRS_SYSTEM_INCLUDE_PATH` to the **target's** platform header directories
in either of these cases:

- Cargo's host and target differ: explicit paths prevent discovery of host headers.
- The target is Windows/MSVC, even for a native build: cinrs does not automatically
  discover MSVC or Windows SDK headers.

Without this setting, `build.rs` rejects these builds. Multiple directories use
the host's path-list separator (`:` on Unix, `;` on Windows).

The Windows/MSVC requirement applies to crate translation with cinrs. It is
separate from [configuration generation](build-configurations.md#windowsmsvc),
where `cl.exe` uses the Visual Studio developer environment's header settings.
Setting the variable does not resolve cinrs's current MSVC dialect limitations.

The script checks that the directories exist, but cannot verify that they belong
to the intended target. Header layouts, the configuration, and the target ABI
must agree. Providing this variable does not enable unsupported C dialects or
make an untested target supported.

Building for another architecture and executing its tests are separate steps.
`cargo build` needs a target standard library and linker; `cargo test` also needs
a way to run the resulting executables.

### Apple Silicon to x86_64 macOS

Install the x86_64 macOS Rust standard library with your toolchain manager, for
example `rustup target add x86_64-apple-darwin`. Use the active macOS SDK:

```sh
CINRS_SYSTEM_INCLUDE_PATH="$(xcrun --sdk macosx --show-sdk-path)/usr/include" \
  cargo build -p mini-ffmpeg-sys --target x86_64-apple-darwin
```

This resolves the SDK path without assuming an Xcode installation directory.
The macOS SDK contains declarations for both architectures; cinrs uses the
explicit Rust target to select the relevant preprocessing branches.
Running x86_64 test executables on Apple Silicon additionally requires Rosetta.

### Ubuntu arm64 to x86_64 Linux/glibc

The maintainer's setup uses the x86_64 GNU cross toolchain, target headers and
runtime under `/usr/x86_64-linux-gnu`, and the `qemu-x86_64` executable. Install
the x86_64 Linux/glibc Rust standard library too, for example with
`rustup target add x86_64-unknown-linux-gnu`.

```sh
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="qemu-x86_64 -L /usr/x86_64-linux-gnu" \
CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
CINRS_SYSTEM_INCLUDE_PATH="/usr/x86_64-linux-gnu/include/" \
  cargo test -p mini-ffmpeg-sys --target x86_64-unknown-linux-gnu
```

The header path supplies cinrs's input, the linker setting selects Rust's target
linker, and the runner executes tests with the target runtime prefix. `CC_...` is
retained from the confirmed environment; it is not a replacement for Cargo's
linker setting, and the sys crate translates its C sources with cinrs.

This example verifies the sys crate. It does not claim that every workspace
crate or arbitrary cross-target combination has been tested.
