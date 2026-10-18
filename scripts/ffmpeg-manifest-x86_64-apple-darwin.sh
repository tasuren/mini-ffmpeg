#!/bin/sh
# Apple Silicon の Mac で、x86_64-apple-darwin の FFmpeg のビルド構成
# （mini-ffmpeg-sys/FFmpeg_build/x86_64-apple-darwin）を生成する。
#
# `configure` はターゲットのコードを実際に実行して調べるので、構成はターゲットを
# 実行できるマシンで作る。Apple Silicon の Mac は、Rosetta で x86_64 のコードを
# 実行できる。ここでは `cargo xtask ffmpeg-manifest` を、Rosetta の下で動かす。
#
# macOS 標準の /bin/sh（bash 3.2 相当）で動くように書いてある。

set -eu

root=$(cd "$(dirname "$0")/.." && pwd)

# x86_64 の Rust のツールチェーン。別のものを使うときだけ環境変数で上書きする。
toolchain=${RUST_X86_64_TOOLCHAIN:-stable-x86_64-apple-darwin}

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "error: Apple Silicon の Mac の、arm64 のシェルで実行してください。" >&2
    exit 1
fi

if ! arch -x86_64 /usr/bin/true 2>/dev/null; then
    echo "error: Rosetta が使えません。softwareupdate --install-rosetta で入れてください。" >&2
    exit 1
fi

if ! cargo=$(rustup which --toolchain "$toolchain" cargo 2>/dev/null); then
    echo "error: ツールチェーン $toolchain がありません。" >&2
    echo "       rustup toolchain install $toolchain" >&2
    exit 1
fi

# configure が使う `gcc` を、x86_64 向けに固定するラッパーに差し替える。
#
# Rosetta の下で動かしても、Apple の clang は、既定で arm64 向けにコンパイルする。
# そのままだと、config.h は arm64 のものになり、xtask の検査も止まる。`-arch` を
# ラッパーの中で付けるので、`configure_args` は他のターゲットと同じままで、
# マニフェストにも `-arch` は現れない。
shim=$(mktemp -d)
trap 'rm -rf "$shim"' EXIT

printf '#!/bin/sh\nexec /usr/bin/clang -arch x86_64 "$@"\n' > "$shim/gcc"
chmod +x "$shim/gcc"

# PATH は、ラッパー、x86_64 の Rust、システムのコマンドだけにする。Homebrew の
# コマンド（/opt/homebrew/bin）は arm64 専用で、Rosetta の下では動かない。
#
# `rustc -vV` を xtask が PATH から実行するので、x86_64 の `rustc` が先に見つかる
# 必要がある。
#
# ビルドの成果物は専用のディレクトリに置く。arm64 の通常のビルドと同じ場所に置くと、
# xtask のバイナリが互いに上書きされて、そのたびに作り直しになる。
cd "$root"

env \
    PATH="$shim:$(dirname "$cargo"):/usr/bin:/bin:/usr/sbin:/sbin" \
    CARGO_TARGET_DIR="$root/target/rosetta" \
    arch -x86_64 cargo xtask ffmpeg-manifest
