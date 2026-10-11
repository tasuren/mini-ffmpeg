# mini-ffmpeg-sys

FFmpeg のライブラリ API を小規模のビルド構成で提供するクレート。
FFmpeg の C コードを cinrs で Rust に変換したものを読み、Rust のみでビルドします。

変換した Rust のコードは、現在リポジトリに含めていません。クローンした直後は、
リポジトリのルートで次のコマンドを一度実行してください。FFmpeg のサブモジュールと
プラットフォームのヘッダ（macOS では Command Line Tools）が必要です。

```sh
cargo xtask ffmpeg-units
```

変換が済んでいれば、このクレートのビルドに cinrs、FFmpeg のソース、プラットフォームの
ヘッダは必要ありません。

特に関連するファイル:

- `targets/<ターゲット>/rust`: ツールが FFmpeg の C コードを変換した Rust のコード（Git の管理外）
- `targets/<ターゲット>/generator.toml`: 変換に使った cinrs の記録（Git の管理外）
- `cinrs_manifest.toml`: ツールにより生成された取り込み情報
- `FFmpeg`: 上流 FFmpeg のサブモジュール
- `FFmpeg_generated`: `make`/`configure` が生成したファイル群
- `LICENSE.ffmpeg`: 使用している FFmpeg の構成におけるライセンス全文

変換の結果はターゲットごとに異なるため、ターゲットごとのディレクトリに分けています。
現在対応しているのは `aarch64-apple-darwin` のみです。
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

`targets/<ターゲット>/rust/` 配下の内容は、上記 FFmpeg の C コードを cinrs で機械的に
Rust へ変換したものです。FFmpeg の `.c` 一つが `.rs` 一つに対応します
（`libavutil/time.c` は `rust/libavutil/time.rs`）。変換に使った cinrs のリポジトリと
コミットは、同じディレクトリの `generator.toml` に記録されます。

変換した Rust のコードには、FFmpeg のコードのほかに、その `.c` が読んだ
プラットフォームのヘッダの宣言、型、インライン関数も含まれます。これを再配布する
場合のライセンスの扱いが決まっていないため、リポジトリにもクレートの配布にも
含めていません（`.gitignore` で除外しています）。

`FFmpeg_generated/` 配下の内容は、上記 FFmpeg の `configure` と `make` による
生成物を、手を加えずにコピーしたものです。設定ヘッダー、バージョンヘッダー、
有効な部品の登録配列を含みます。C コードを変換するときの入力で、クレートの
配布には含めていません。

構成と生成環境は [cinrs_manifest.toml](cinrs_manifest.toml) に記録しています。
現在の構成では GPL、nonfree、version3 オプションを有効にしていません。
`config.h` のコンパイラ識別文字列は、生成時の環境を示す情報です。

リポジトリのルートで次の順に実行すると、構成ファイル・取り込み対象の一覧と、
変換した Rust ファイルを生成できます。どちらのコマンドにも FFmpeg の
サブモジュールが必要です。最初のコマンドには C コンパイラと `make` が、二つ目の
コマンドにはプラットフォームのヘッダが必要です。最初のコマンドの結果はコミット
してあるので、FFmpeg のバージョンや構成を変えるときだけ実行します。

```sh
cargo xtask ffmpeg-manifest
cargo xtask ffmpeg-units
```

二つ目のコマンドは、実行したマシンのターゲット向けの Rust ファイルを生成します。

生成処理と `configure` の引数は
[xtask/src/ffmpeg_manifest.rs](../xtask/src/ffmpeg_manifest.rs) に、変換の指定は
[xtask/src/ffmpeg_units](../xtask/src/ffmpeg_units) にあります。
`FFmpeg_generated/` と `targets/<ターゲット>/rust/` は再生成時に置き換えられるため、
説明や手書きのファイルはその外に置いています。

### クレートへの同梱

このクレートは、まだ公開できる状態ではありません（`publish = false`）。
変換した Rust のコードを配布に含めるか、FFmpeg の C のソースを含めて利用者の
ビルドで変換するかを、ライセンスの扱いと合わせて検討しています。

また、FFmpeg には LGPL 以外のライセンスのファイルがあります。現在の構成では、
libjpeg 由来の `libavcodec/jfdctfst.c`、`libavcodec/jfdctint_template.c`、
`libavcodec/jrevdct.c`（IJG のライセンス）、`libavcodec/faandct.c`、
`libavutil/avsscanf.c`、`libavutil/adler32.c` が該当します。配布の形を決めるときに、
各ファイルの先頭の条件と、サブモジュール内の [LICENSE.md](FFmpeg/LICENSE.md) を
確認する必要があります。
