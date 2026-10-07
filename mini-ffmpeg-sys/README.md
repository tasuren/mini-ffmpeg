# mini-ffmpeg-sys

FFmpeg のライブラリ API を小規模のビルド構成で提供するクレート。
cinrs を使ってライブラリの C コードを取り込み Rust のみでビルド可能。

特に関連するファイル:

- `cinrs_manifest.toml`: ツールにより生成された取り込み情報
- `FFmpeg`: 上流 FFmpeg のサブモジュール
- `FFmpeg_generated`: `make`/`configure` が生成したファイル群
- `LICENSE.ffmpeg`: 使用している FFmpeg の構成におけるライセンス全文

`src/` 下のうち先頭に `// @generated` があるものはツールにより生成されました。
例えば、`libavutil.rs` や `libavutil/time.rs` などがそれにあたります。
詳細は後述の「生成物」を参照してください。

## ライセンス

本クレートのコードは GNU Lesser General Public License version 2.1
or later（SPDX: `LGPL-2.1-or-later`）で提供しています。
ライセンスの全文はリポジトリルートの `LICENSE` を参照してください。

## FFmpeg 由来のファイル

FFmpeg の取り込み元は、`FFmpeg/` サブモジュールです。使用するリビジョンは、
このリポジトリが記録しているサブモジュールのコミットを参照してください。

[LICENSE.ffmpeg](LICENSE.ffmpeg) は、そのリビジョンの
`COPYING.LGPLv2.1` を変更せずコピーした LGPL 2.1 の全文です。
FFmpeg 全体には異なるライセンスのファイルも含まれるため、サブモジュール内の
[LICENSE.md](FFmpeg/LICENSE.md) と各ファイルの表示も参照してください。

### 生成物

`FFmpeg_generated/` 配下の内容は、上記 FFmpeg の `configure` と `make` による
生成物を、手を加えずにコピーしたものです。設定ヘッダー、バージョンヘッダー、
有効な部品の登録配列を含みます。FFmpeg 由来の構成ファイルとして、
本クレートの LGPL-2.1-or-later の配布に含めています。

構成と生成環境は [cinrs_manifest.toml](cinrs_manifest.toml) に記録しています。
現在の構成では GPL、nonfree、version3 オプションを有効にしていません。
`config.h` のコンパイラ識別文字列は、生成時の環境を示す情報です。

リポジトリのルートで次の順に実行すると、構成ファイル・取り込み対象の一覧と、
cinrs を使った Rust ファイルを再生成できます。最初のコマンドには FFmpeg の
サブモジュール、C コンパイラ、`make` が必要です。

```sh
cargo xtask ffmpeg-manifest
cargo xtask ffmpeg-units
```

生成処理と `configure` の引数は
[xtask/src/ffmpeg_manifest.rs](../xtask/src/ffmpeg_manifest.rs) にあります。
`FFmpeg_generated/` は再生成時に置き換えられるため、説明や手書きのファイルは
その外に置いています。

### 変更したファイル

以下は、FFmpeg のソースをコピーして変更したファイルです。
コピー元のリビジョン、変更日、変更内容、および著作権・ライセンスの表示は、
各ファイルの先頭コメントを参照してください。

- [src/libavutil/log.c](src/libavutil/log.c)

### クレートへの同梱

`Cargo.toml` の `include` で、`LICENSE.ffmpeg`、この README、FFmpeg のソース、
生成物と変更した `log.c` を配布対象に指定しています。
配布対象はリポジトリのルートから次のコマンドで確認できます。

```sh
cargo package -p mini-ffmpeg-sys --list --allow-dirty
```
