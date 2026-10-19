# Generating build configurations

This is a maintainer task for FFmpeg updates, build-setting changes, and
investigating new targets. Normal crate users use the configurations already
committed under `mini-ffmpeg-sys/FFmpeg_build/`; see the
[build guide](building.md) instead.

Run the commands below from the repository root.

## What the task records

```sh
cargo xtask ffmpeg-manifest
```

The task runs FFmpeg's `configure` and `make`, inspects compile commands,
dependency files and installed headers, then writes:

```text
mini-ffmpeg-sys/FFmpeg_build/<rustc-host-triple>/
├── cinrs_manifest.toml
├── config.h
└── ...other generated files
```

The output contains both the manifest and the matching generated headers and
registration tables. Keep the entire configuration together. The task replaces
its target's output directory; manually added files there are removed. Other
target directories are left unchanged. Intermediate files and logs live in
`target/xtask/ffmpeg-build/`.

Change `CONFIGURE_ARGS` in
[`xtask/src/ffmpeg_manifest.rs`](../xtask/src/ffmpeg_manifest.rs) to change the
build settings. Do not hand-edit generated configuration files. After an FFmpeg
update, initialize the intended submodule revision before generating each
configuration.

## Rust toolchain and compiler architecture

The directory name comes from the **`host:` line reported by `rustc -vV`**,
using `RUSTC` if it is set. It is not inferred from the hardware alone or from a
Cargo `--target` argument.

The task also preprocesses an architecture probe with the C compiler and flags
selected by `configure`. Its target CPU architecture must match the Rust host
triple. A mismatch stops generation before output is saved.

For another architecture, select both a matching Rust toolchain and C compiler,
and run in an environment able to execute that architecture. A rustup
`+<toolchain>` selection changes the Rust toolchain; it does not by itself
retarget the C compiler. The architecture check does not establish that every
OS, libc, or ABI setting is correct.

This is different from cross-compiling the Rust crates against an existing
configuration. That process uses Cargo's `--target` and target headers as
explained in the [build guide](building.md#cross-compilation-rules).

## macOS and Linux

For native generation, install Rust, a C compiler with matching platform
headers/libraries, Bash, Make, and the other tools needed by FFmpeg's configure
script. FFmpeg's `configure` itself is supplied by the submodule; it is not a
separate package. Then run:

```sh
git submodule update --init # if you have never done it
cargo xtask ffmpeg-manifest
```

The current configuration disables assembly and inline assembly, so it does not
require an assembler for FFmpeg's optimized assembly implementations.

### x86_64 macOS on Apple Silicon

The repository provides a helper that selects x86_64 Rust, runs through Rosetta,
and wraps Apple's compiler to force x86_64 output:

```sh
scripts/ffmpeg-manifest-x86_64-apple-darwin.sh
```

It requires Rosetta and the `stable-x86_64-apple-darwin` Rust toolchain. Set
`RUST_X86_64_TOOLCHAIN` to select another installed toolchain. Merely running
x86_64 Rust is insufficient: Apple's compiler can still default to arm64, which
is why the helper explicitly selects the C compiler architecture too.

### Linux/glibc through Docker

```sh
scripts/ffmpeg-manifest-linux-docker.sh arm64
scripts/ffmpeg-manifest-linux-docker.sh amd64
```

These generate `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-gnu`
configurations respectively. The helper defaults to `rust:1-trixie`; override
`FFMPEG_MANIFEST_IMAGE` to use a different image. The image must provide Rust,
the build tools, and the intended libc development environment. Docker must be
able to run the selected platform, using emulation if needed.

The generating environment affects `config.h` and the manifest. Use the same
image/toolchain when reproducing a configuration; switching distributions or
libc versions is an environment change worth reviewing.

## Windows/MSVC

The following procedure was used by the maintainer to generate the recorded
x64 and ARM64 MSVC configurations. Generating these files uses the native C
toolchain and succeeds independently of cinrs: **the Rust crates still cannot
be built with this MSVC configuration because the required dialect is not yet
supported by cinrs**.

Prerequisites are Visual Studio C++ Build Tools and Windows SDK for the selected
architecture, Rust with a matching MSVC toolchain, and MSYS2 with Bash, Make and
the other shell utilities required by FFmpeg.

1. Open the **x64 Native Tools Command Prompt** or **ARM64 Native Tools Command
   Prompt**, matching the intended configuration. Change to the repository root.
   This initializes MSVC's compiler, SDK, include and library environment.
2. Start the MSYS2 shell from that prompt, preserving the Windows environment
   and current directory (adjust the installation path if necessary):

   ```bat
   C:\msys64\msys2_shell.cmd -msys -use-full-path -here
   ```

3. In the MSYS2 shell, find the selected MSVC compiler and put its directory
   before MSYS2's tools:

   ```sh
   command -v cl.exe
   msvc_bin="$(dirname "$(command -v cl.exe)")"
   export PATH="$msvc_bin:$PATH"
   ```

   This makes MSVC's tools, including its linker, take priority over tools with
   conflicting names in MSYS2. The shell supplies Bash and Make; `cl.exe` and
   the Windows SDK supply the compiler and platform headers/libraries.
4. Confirm the Rust toolchain's host architecture and generate the configuration:

   ```sh
   rustc -vV
   cargo xtask ffmpeg-manifest
   ```

   If the default Rust toolchain has a different architecture, select an
   installed matching toolchain. For example, with x64 Native Tools:

   ```sh
   rustup run stable-x86_64-pc-windows-msvc rustc -vV
   cargo +stable-x86_64-pc-windows-msvc xtask ffmpeg-manifest
   ```

   Use the corresponding `aarch64-pc-windows-msvc` toolchain with ARM64 Native
   Tools. The machine must be able to execute the chosen Rust toolchain.

When xtask runs as an MSVC Windows build, it adds `--toolchain=msvc` to FFmpeg's
configure arguments. The recorded configuration still follows the `host:`
triple of the selected `rustc`, so verify it before interpreting the output.

These generation steps use `cl.exe` and the developer environment's header
settings. Translating the Rust crates with cinrs separately requires
`CINRS_SYSTEM_INCLUDE_PATH`, even for native Windows/MSVC builds; see the
[build guide](building.md#cross-compilation-rules). Supplying header paths does
not resolve the current MSVC dialect limitations.

This procedure does not describe a Windows/GNU configuration. MSYS2's MinGW-w64
environments use Windows runtimes such as UCRT/MSVCRT, not Linux's glibc; see
[MSYS2 environments](https://www.msys2.org/docs/environments/). Those toolchains
have not been used to generate a configuration in this repository. For
FFmpeg's general MSVC guidance, see the
[vendor documentation](../mini-ffmpeg-sys/FFmpeg/doc/platform.texi).

## Review and verify

Review the full diff under the target's `FFmpeg_build/` directory: the target
triple, configure arguments, defines, include directories, source/header lists,
and generated files all matter. Do not treat configuration generation as proof
that cinrs can translate that target.

For a target on which crate translation works, run its sys tests with the
appropriate target headers and linker/runner, then workspace tests and the
remux example where applicable. The examples in the [build guide](building.md)
record the configurations that have actually been exercised.

The input [license scan](../xtask/README.md#license-scanning) can be run against
the new configuration without rebuilding FFmpeg.
