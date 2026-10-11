# mini-ffmpeg-sys

FFmpeg のライブラリ API を小規模のビルド構成で提供するクレート。
ビルド時に FFmpeg の C コードを cinrs で Rust に変換するので、C コンパイラ無しで
ビルド可能。

ビルドには、FFmpeg のサブモジュールと、プラットフォームのヘッダ（macOS では
Command Line Tools）が必要です。現在対応しているターゲットは
`aarch64-apple-darwin` のみで、クロスコンパイルには対応していません。

特に関連するファイル:

- `build.rs`、`build/`: FFmpeg の C コードを Rust に変換するビルドスクリプト
- `cinrs_manifest.toml`: ツールにより生成された取り込み情報
- `FFmpeg`: 上流 FFmpeg のサブモジュール
- `FFmpeg_generated`: `make`/`configure` が生成したファイル群
- `LICENSE.ffmpeg`: 使用している FFmpeg の構成におけるライセンス全文

## ビルドの流れ

ビルドスクリプトが `cinrs_manifest.toml` にある FFmpeg の `.c` を一つずつ Rust に
変換し、Cargo の出力用のディレクトリに書き出します。`src/lib.rs` がそれを
読み込みます（`libavutil/time.c` は `libavutil::time` になります）。

変換は、入力（FFmpeg のソース、`FFmpeg_generated/`、プラットフォームのヘッダ）が
変わったときだけ行われます。Apple M3 Pro（11 コア）での目安は、変換に約 6 秒、
変換後のコードのコンパイルに約 6 秒です。

変換済みの Rust のコードは、リポジトリにもクレートの配布にも含めていません。
変換後のコードには、その `.c` が読んだプラットフォームのヘッダの宣言や
インライン関数も含まれるためです。FFmpeg のソースを無編集で配布し、ビルドする
マシンで変換します。

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

リポジトリのルートで次のコマンドを実行すると、構成ファイルと取り込み対象の一覧を
再生成できます。FFmpeg のサブモジュール、C コンパイラ、`make` が必要です。
FFmpeg のバージョンや構成を変えるときだけ実行します。

```sh
cargo xtask ffmpeg-manifest
```

生成処理と `configure` の引数は
[xtask/src/ffmpeg_manifest.rs](../xtask/src/ffmpeg_manifest.rs) にあります。
`FFmpeg_generated/` は再生成時に置き換えられるため、説明や手書きのファイルは
その外に置いています。

### クレートへの同梱

`Cargo.toml` の `include` で、`LICENSE.ffmpeg`、この README、FFmpeg のソース、
生成物、ビルドスクリプトを配布対象に指定しています。
配布対象はリポジトリのルートから次のコマンドで確認できます。

```sh
cargo package -p mini-ffmpeg-sys --list --allow-dirty
```

このクレートは、まだ公開できる状態ではありません（`publish = false`）。
ビルドスクリプトが使う cinrs は、未公開のコミットを Git の依存として指定して
います。crates.io は Git の依存を受け付けないので、必要な変更を含む cinrs が
公開されるまで、このクレートも公開できません。
