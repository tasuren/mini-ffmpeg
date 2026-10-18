#!/bin/sh
# Docker で、Linux（glibc）の FFmpeg のビルド構成を生成する。
#
#   scripts/ffmpeg-manifest-linux-docker.sh amd64   # x86_64-unknown-linux-gnu
#   scripts/ffmpeg-manifest-linux-docker.sh arm64   # aarch64-unknown-linux-gnu
#
# `configure` はターゲットのコードを実際に実行して調べるので、構成はターゲットを
# 実行できるマシンで作る。Docker は `--platform` で、ホストと違うアーキテクチャの
# コンテナを（エミュレーションで）動かせる。ホストの OS は問わない。
# ホストと同じアーキテクチャなら、エミュレーションは働かず、速い。
#
# 生成されるトリプルは、コンテナの中の `rustc` のホストで決まる。musl の Alpine
# などを使うと、トリプルが変わる。
#
# macOS 標準の /bin/sh でも動くように書いてある。

set -eu

root=$(cd "$(dirname "$0")/.." && pwd)

# Rust の公式イメージ（Debian）。C コンパイラ、make、bash が入っている。
# `config.h` は glibc のバージョンなどに依存するので、再現するときは同じイメージを
# 使う。別のものを使うときだけ環境変数で上書きする。
image=${FFMPEG_MANIFEST_IMAGE:-rust:1-trixie}

case "${1:-}" in
    amd64 | arm64) platform=linux/$1 ;;
    *)
        echo "使い方: $0 amd64|arm64" >&2
        exit 1
        ;;
esac

# - ホストのユーザーで実行する。root で動かすと、生成物の持ち主が root になる。
#   そのため、ホームや Cargo の置き場所は、書き込める /tmp に変える。
# - git の所有者の検査を無効にする。バージョン（ffversion.h）は `git describe` から
#   決まるが、検査に掛かると、`n9.0.2` のようにならない。
# - ビルドの成果物は、コンテナの中に置く。マウントしたディレクトリに置くと遅い。
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
