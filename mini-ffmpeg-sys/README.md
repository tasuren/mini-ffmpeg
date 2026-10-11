# mini-ffmpeg-sys

FFmpeg のライブラリ API を小規模のビルド構成で提供するクレート。
ビルド時に FFmpeg の C コードを cinrs で Rust に変換するので、C コンパイラ無しで
ビルド可能。

ビルドには、FFmpeg のサブモジュールと、プラットフォームのヘッダ（macOS では
Command Line Tools）が必要です。現在対応しているターゲットは
`aarch64-apple-darwin` のみで、クロスコンパイルには対応していません。

特に関連するファイル:

- `build.rs`、`build/`: FFmpeg の C コードを Rust に変換するビルドスクリプト
- `cinrs_manifest.toml`: ツールにより生成されたビルドマニフェスト
- `FFmpeg`: 上流 FFmpeg のサブモジュール
- `FFmpeg_generated`: `make`/`configure` が生成したファイル群
- `LICENSE.ffmpeg`: 使用している FFmpeg の構成におけるライセンス全文

## ビルドの流れ

ビルドマニフェスト（`cinrs_manifest.toml`）は、FFmpeg を一度ビルドして読み取った、
ビルドの内容の宣言です。どのファイルを、どの指定でコンパイルするかを含みます。
ツールが生成し、手では編集しません。

ビルドスクリプトがビルドマニフェストにある FFmpeg の `.c` を一つずつ Rust に
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

リポジトリのルートで次のコマンドを実行すると、構成ファイルとビルドマニフェストを
再生成できます。FFmpeg のサブモジュール、C コンパイラ、`make` が必要です。
FFmpeg のバージョンや構成を変えるときだけ実行します。

```sh
cargo xtask ffmpeg-manifest
```

生成処理と `configure` の引数は
[xtask/src/ffmpeg_manifest.rs](../xtask/src/ffmpeg_manifest.rs) にあります。
`FFmpeg_generated/` は再生成時に置き換えられるため、説明や手書きのファイルは
その外に置いています。

### LGPL 以外のライセンスのファイル

FFmpeg には、LGPL 以外のライセンスの表示を持つファイルがあります。現在の構成で
使うファイルでは、次のものが該当します。条件の原文は各ファイルにあります。
サブモジュール内の [LICENSE.md](FFmpeg/LICENSE.md) も参照してください。

| ライセンス | ファイル |
| --- | --- |
| IJG（libjpeg 由来） | `libavcodec/jfdctfst.c`、`libavcodec/jfdctint_template.c`、`libavcodec/jrevdct.c` |
| BSD-3-Clause（LGPL の表示と併記） | `libavutil/fixed_dsp.c`、`libavutil/fixed_dsp.h`、`libavutil/uuid.c` |
| BSL-1.0（LGPL の表示と併記。一部の関数が対象） | `libavutil/mathematics.c`、`libavutil/libm.h` |
| ISC | `libavcodec/faandct.c` |
| MIT | `libavutil/avsscanf.c` |
| Zlib | `libavutil/adler32.c` |

それ以外のファイルの表示は LGPL 2.1 or later のみです。`FFmpeg_generated/` の
生成物には、ライセンスの表示がありません。

#### Independent JPEG Group のクレジット

libjpeg 由来の 3 つのファイルについて、次のクレジットを記します。

> This software is based in part on the work of the Independent JPEG Group.

- 3 つのファイルは、FFmpeg のツリーにあるものをそのまま使っています。本クレートで
  加えた追加、削除、変更はありません。FFmpeg が libjpeg のファイルに加えた変更は、
  FFmpeg のリポジトリの履歴を参照してください。
- 著作権の表示、無保証の表示、配布の条件は、各ファイルの先頭にあります。
- 本クレートを使ったプログラムを実行ファイルだけの形で配布する場合は、付属の
  文書に上記のクレジットを入れる必要があります。

#### 実行ファイルを配布する場合

本クレートを使ったプログラムをビルドすると、FFmpeg のコードがその実行ファイルに
含まれます。その実行ファイルを配布する場合、配布する人は各ライセンスの条件を
満たす必要があります。次の一覧は、気づくための手掛かりとして挙げるもので、
網羅的なものでも、法的な助言でもありません。**条件は原文で確認してください。**

- LGPL 2.1: 受け取った人がライブラリの部分を差し替えられるようにすることなどを
  求めています。Rust のクレートは既定で静的にリンクされる点に注意してください。
  FFmpeg による説明が <https://ffmpeg.org/legal.html> にあります。
- BSD-3-Clause、MIT、ISC: 著作権の表示と許諾の文面を、原文のまま配布物に含める
  ことを求めています。BSD-3-Clause は、実行ファイルの形での配布について、付属の
  文書などに載せることを明記しています。
- IJG: 上記のクレジットを、付属の文書に入れることを求めています。
- BSL-1.0、Zlib: 表示を保つことを求めているのは、ソースの配布についてです。

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
