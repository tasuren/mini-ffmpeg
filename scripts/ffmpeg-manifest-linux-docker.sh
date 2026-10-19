#!/bin/sh
# Generate the FFmpeg build configuration for Linux (glibc) using Docker.
#
#   scripts/ffmpeg-manifest-linux-docker.sh amd64   # x86_64-unknown-linux-gnu
#   scripts/ffmpeg-manifest-linux-docker.sh arm64   # aarch64-unknown-linux-gnu
#
# `configure` probes the target by running its code, so generate the configuration
# on a machine that can run the target. Docker's `--platform` can run containers
# for architectures other than the host's through emulation, regardless of the
# host OS. Matching the host architecture avoids emulation and runs faster.
#
# The host triple of `rustc` inside the container determines the generated triple.
# Using a musl-based image such as Alpine changes the triple.
#
# Compatible with the default macOS /bin/sh.

set -eu

root=$(cd "$(dirname "$0")/.." && pwd)

# Official Rust image (Debian), including a C compiler, make, and bash.
# `config.h` depends on details such as the glibc version, so use the same image
# to reproduce it. Override the environment variable to use another image.
image=${FFMPEG_MANIFEST_IMAGE:-rust:1-trixie}

case "${1:-}" in
    amd64 | arm64) platform=linux/$1 ;;
    *)
        echo "Usage: $0 amd64|arm64" >&2
        exit 1
        ;;
esac

# - Run as the host user; running as root would make root own the generated files.
#   Place the home and Cargo directories under writable /tmp for this reason.
# - Disable git's ownership check. The version (ffversion.h) comes from
#   `git describe`; if the check fails, it will not have a value such as `n9.0.2`.
# - Keep build artifacts inside the container; a mounted directory would be slow.
exec docker run --rm \
    --platform "$platform" \
    --user "$(id -u):$(id -g)" \
    --mount "type=bind,source=$root,target=/work" \
    --workdir /work \
    --env HOME=/tmp/home \
    --env CARGO_HOME=/tmp/cargo-home \
    --env CARGO_TARGET_DIR=/tmp/cargo-target \
    --env GIT_CONFIG_COUNT=1 \
    --env GIT_CONFIG_KEY_0=safe.directory \
    --env GIT_CONFIG_VALUE_0='*' \
    "$image" \
    cargo xtask ffmpeg-manifest
