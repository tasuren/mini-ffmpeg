# scripts

メンテナが手元で使うスクリプト。利用者のビルドでは使われません。

どちらも `cargo xtask ffmpeg-manifest` を、普段とは別のターゲット向けに動かすためのものです。
`configure` はターゲットのコードを実際に実行して調べるので、ビルド構成
（`mini-ffmpeg-sys/FFmpeg_build/<ターゲットトリプル>/`）は、ターゲットを実行できる環境で
生成します。ホストと同じターゲットなら、スクリプトを使わず `cargo xtask ffmpeg-manifest`
を直接実行します。

| スクリプト | 生成するターゲット | 必要なもの |
| --- | --- | --- |
| `ffmpeg-manifest-x86_64-apple-darwin.sh` | `x86_64-apple-darwin` | Apple Silicon の Mac、Rosetta、`x86_64-apple-darwin` の Rust ツールチェーン |
| `ffmpeg-manifest-linux-docker.sh amd64\|arm64` | `x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu` | Docker |

どちらも、エミュレーションで生成します。本物のマシンで生成した場合と完全に同じ内容かは
確かめていません。ホストと違うアーキテクチャでは処理が遅い可能性があります。

Windows 用のスクリプトはありません。実機で確かめてから追加します。
