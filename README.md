# Mini FFmpeg (**WIP**)

このリポジトリでは、FFmpeg のライブラリ（`libavcodec`等）を [cinrs] で Rust
コードに取り込み、Rust のツールチェーンのみでビルドできる FFmpeg の軽量版を
実験的に開発しています。

cinrs は C のコードを Rust のマクロで書くと、コンパイル時に Rust コードへ
変換してくれるライブラリです。C コードのコンパイルに C コンパイラを使いません。

## 目的

このプロジェクトは今の所、全てにおいて完璧な「ミニ」な FFmpeg を作ることを
意図していません。
将来的にそれを目指す可能性はあるものの、現状は「cinrs で FFmpeg をどこまで
取り込めるか」を試し、cinrs の使い道を研究する意図が強いです。

ただし、ミニと名付けてるのは、次のような点を意識しているためです。
完全にこれらを諦めているわけではありません。

- **ポータビリティ** — Cargo さえあればすぐ使えるライブラリや CLI が欲しい
- **統合されたパイプライン** — CLI 並の高水準 API が欲しい

## 現在の状況

CLI もライブラリの API もまだありません。
できているのは FFmpeg を取り込む土台と小さいサンプルのみです。

- FFmpeg バージョン: `n9.0.2`
- ビルド構成: WAV demuxer/muxer, file/pipe プロトコル対応
- Cファイルの数: 178個 （サブモジュールから177個、1個は変更を加えたコピー）

この構成で `mini-ffmpeg-sys` クレートを作成し、`remux-spike` にある
コードで WAV から WAV への再エンコード無しコピーが Rust で動作します。
この `remux-spike` は FFmpeg の公式サンプル `doc/examples/remux.c` を cinrs
で取り込んだもので、出力は C コンパイラでビルドしたオリジナルと一致します。

ただし、以下の前提・制限があります。

- 動作を確かめたのは macOS (arm64) のみ
- スレッドを無効にした構成
- コーデックもフィルタも含んでいない
- ライブラリを取り込んだ `mini-ffmpeg-sys` のビルドに 1 分前後掛かる

## 設計概要

- FFmpeg の `.c` 一つにつき一つの cinrs マクロ呼び出し
- `make` のコンパイルコマンドから対象の `.c`・インクルードパス・マクロ定義を抽出
- `make` が出力する `.d` から、include されるファイルを抽出
- 抽出した情報を元に cinrs マクロの呼び出しコードを生成
- `configure`/`make` は FFmpeg 更新時かビルド設定変更時だけ再実行すれば良い

| クレート          | 役割                                                                                |
| ----------------- | ----------------------------------------------------------------------------------- |
| `mini-ffmpeg`     | CLI とライブラリ。まだ未実装で中身がありません。                                    |
| `mini-ffmpeg-sys` | cinrs でライブラリを取り込むクレート。このフォルダにFFmpeg のサブモジュールがある。 |
| `remux-spike`     | 取り込んだ FFmpeg が実際に動くかを確かめるための、仮のクレート。                    |
| `xtask`           | ビルドマニフェストとRustの取り込みコードを生成する開発用のツール。                  |

## 動作方法

Rust 1.99 以降と、プラットフォームのヘッダ（macOS では Command Line Tools）が
必要です。FFmpeg のツリーはサブモジュールなので、クローンのあとに取得します。

```sh
git submodule update --init
cargo test --workspace
cargo run -p remux-spike -- input.wav output.wav
```

ビルドの時間の大半は、cinrs が C を Rust に変換する時間です。Cargo は
proc-macro を既定では最適化せずにコンパイルするので、ワークスペースのルートの
`Cargo.toml` に次の設定を足すと速くなります。

```toml
[profile.dev.build-override]
opt-level = 3
```

## FFmpeg 入力のライセンス検査

`cinrs_manifest.toml` に列挙された C ソース、ヘッダ、`FFmpeg_generated/` の
ファイルを、`licensecheck` で全文検査できます。

```sh
cargo xtask ffmpeg-license
cargo xtask ffmpeg-license --docker
cargo xtask ffmpeg-license --native --output target/license-report
```

Linux では PATH にある `licensecheck` を直接実行します。Debian なら
`apt install licensecheck` で準備できます。macOS と Windows は Docker が既定で、
Debian trixie の検査用イメージを初回にビルドします。その後は Docker の
ビルドキャッシュを使います。`--docker` と `--native` で実行方法を指定できます。
この検査に C コンパイラや FFmpeg の再ビルドは必要ありません。

既定の出力先は `target/xtask/ffmpeg-license/` です。
`report.md` に検出値ごとの集計・全ファイル一覧とツールのバージョン、
`licensecheck.tsv` にファイルごとの検出値と著作権表示を保存します。
`versions.txt` と、検査に使ったビルドマニフェストのコピー `manifest.toml` も残します。
`--output` の相対パスは実行したディレクトリが基準です。

結果は自動検出値です。IJG の表示も `UNKNOWN` になる場合があり、複数検出時の
`and/or` は適用ライセンスを確定するものではありません。システムヘッダや
cinrs / Cargo の依存は、この検査の対象に含まれません。

## ライセンス

本プロジェクトのコードは GNU Lesser General Public License version 2.1
or later（SPDX: `LGPL-2.1-or-later`）で提供します。
そのライセンス文書は [LICENSE](LICENSE) から確認ができます。

`mini-ffmpeg-sys` には FFmpeg のソースと、ビルドシステムの生成物が含まれます。
由来・適用ライセンスについては
[mini-ffmpeg-sys の README](mini-ffmpeg-sys/README.md) を参照してください。
由来にあたる FFmpeg の LGPL 2.1 の全文は [LICENSE.ffmpeg][LICENSE-FFmpeg]
にあります。

FFmpeg には libjpeg に由来するファイルが含まれます。
This software is based in part on the work of the Independent JPEG Group.

`remux-spike` は MIT ライセンスの FFmpeg サンプルコードを取り込む。

[cinrs]: https://github.com/tanakh/cinrs
[LICENSE-FFmpeg]: mini-ffmpeg-sys/LICENSE.ffmpeg
