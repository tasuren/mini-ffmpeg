#!/bin/sh
# Generate the FFmpeg build configuration for x86_64-apple-darwin on an Apple
# Silicon Mac (mini-ffmpeg-sys/FFmpeg_build/x86_64-apple-darwin).
#
# `configure` probes the target by running its code, so generate the configuration
# on a machine that can run the target. Apple Silicon Macs can run x86_64 code
# through Rosetta. This script runs `cargo xtask ffmpeg-manifest` under Rosetta.
#
# Compatible with the default macOS /bin/sh (equivalent to bash 3.2).

set -eu

root=$(cd "$(dirname "$0")/.." && pwd)

# The x86_64 Rust toolchain. Override the environment variable to use another one.
toolchain=${RUST_X86_64_TOOLCHAIN:-stable-x86_64-apple-darwin}

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "error: run this script in an arm64 shell on an Apple Silicon Mac." >&2
    exit 1
fi

if ! arch -x86_64 /usr/bin/true 2>/dev/null; then
    echo "error: Rosetta is unavailable. Install it with softwareupdate --install-rosetta." >&2
    exit 1
fi

if ! cargo=$(rustup which --toolchain "$toolchain" cargo 2>/dev/null); then
    echo "error: toolchain $toolchain is not installed." >&2
    echo "       rustup toolchain install $toolchain" >&2
    exit 1
fi

# Replace the `gcc` used by configure with a wrapper that forces x86_64 output.
#
# Apple's clang defaults to arm64 even under Rosetta. Without the wrapper,
# config.h would describe arm64, and xtask's validation would fail. Adding `-arch`
# in the wrapper keeps `configure_args` the same as for other targets and leaves
# `-arch` out of the manifest.
shim=$(mktemp -d)
trap 'rm -rf "$shim"' EXIT

printf '#!/bin/sh\nexec /usr/bin/clang -arch x86_64 "$@"\n' > "$shim/gcc"
chmod +x "$shim/gcc"

# Limit PATH to the wrapper, x86_64 Rust, and system commands. Homebrew commands
# (/opt/homebrew/bin) are arm64-only and cannot run under Rosetta.
#
# xtask runs `rustc -vV` using PATH, so it must find the x86_64 `rustc` first.
#
# Use a dedicated directory for build artifacts. Sharing the normal arm64 build
# directory would overwrite the xtask binary and require rebuilding it each time.
cd "$root"

env \
    PATH="$shim:$(dirname "$cargo"):/usr/bin:/bin:/usr/sbin:/sbin" \
    CARGO_TARGET_DIR="$root/target/rosetta" \
    arch -x86_64 cargo xtask ffmpeg-manifest
