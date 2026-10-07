/*
 * cinrs で FFmpeg のコードを読むための回避策。
 *
 * FFmpeg のソースには手を加えずに、cinrs が扱えない箇所を避けるためのもの。
 * どの項目も、FFmpeg のヘッダを先に読んでから、そこで定義されるマクロを
 * 定義し直している。そのため、インクルードディレクトリの指定と、ビルドの
 * マクロ定義（`HAVE_AV_CONFIG_H` など）より後で include する。
 *
 * プラットフォームのヘッダを読むための回避策は、`cinrs_platform_fixups.h` に
 * ある。あちらはプラットフォームのヘッダより前に読む必要があるので、ファイルを
 * 分けている。
 *
 * ここに置けるのは、全翻訳単位に入れても害が無い回避策だけ。FFmpeg のソース
 * そのものを書き換えないと通らない翻訳単位は、xtask の `EXCLUDED_UNITS` に
 * 載せて、手書きの取り込みにする。
 *
 * どの項目も cinrs 0.2.0 で必要だったもので、cinrs の側で直れば消せる。消す
 * ときは、その項目を外しても `cargo test -p mini-ffmpeg-sys` が通ることを
 * 確かめる。
 */

#ifndef MINI_FFMPEG_SYS_CINRS_FFMPEG_FIXUPS_H
#define MINI_FFMPEG_SYS_CINRS_FFMPEG_FIXUPS_H

/*
 * FFmpeg は、バージョンの文字列（`63.1.102`）を、トークンの連結で組み立てる。
 *
 *     #define AV_VERSION_DOT(a, b, c) a ##.## b ##.## c
 *
 * `63.1` に `.` を連結した `63.1.` は、C の前処理数（pp-number）として正しい
 * トークンだが、cinrs は「pasting '63.1' and '.' does not give a valid token」
 * として拒否する。`libavformat/mux.c` が `LIBAVFORMAT_IDENT`（"Lavf63.1.102"）を
 * 使う箇所で当たる。
 *
 * このマクロの結果は、文字列化して使われる。連結せずに並べるだけでも、
 * 文字列化した結果は同じになるので、そう定義し直している。元のビルドの
 * オブジェクトファイルに入る文字列と一致することを確かめてある。
 */
#include "libavutil/version.h"

#undef AV_VERSION_DOT
#define AV_VERSION_DOT(a, b, c) a.b.c

#if defined(__APPLE__)

/*
 * `libavutil/timer.h` は、高精度の時刻を読む方法を `config.h` の内容で選ぶ。
 * macOS では `HAVE_MACH_ABSOLUTE_TIME` が 1 なので、`mach_absolute_time` を
 * 使う分岐が選ばれ、`<mach/mach_time.h>` が include される。
 *
 * このヘッダは、関数 1 つのために Mach の型のヘッダを約 30 個まとめて読む。
 * その中に、cinrs が扱えない書き方が 2 つある。
 *
 * - `<mach_debug/ipc_info.h>`: 列挙子の値 `~0u` が `int` の範囲を超える。clang
 *   なら基底の型を指定した列挙型として読まれるが、cinrs は GCC として振る舞う
 *   ので、通常の列挙型に展開される。
 * - `<mach/arm/processor_info.h>`: 構造体に `__attribute__((packed, aligned(4)))`
 *   が付いている。この組み合わせを表す方法が Rust に無い。
 *
 * どちらも FFmpeg が使う型ではなく、読み込まれるだけで止まる。
 *
 * `timer.h` には、`clock_gettime` で時刻を読む分岐が元からある。
 * `HAVE_MACH_ABSOLUTE_TIME` を 0 に定義し直して、そちらを選ばせている。
 *
 * `clock_gettime` が macOS に入ったのは 10.12 (Sierra) から。それより古い
 * macOS でも動くように、`mach_absolute_time` を使う分岐が残っているのだと
 * 考えられる。今の macOS を対象にする限り、`clock_gettime` の側で困らない。
 *
 * `timer.h` を使う翻訳単位は、今の構成では `libavutil/random_seed.c` だけ。
 * そこでも macOS では `arc4random_buf` が先に使われるので、この時刻を使う
 * コードは、乱数の取得に失敗したときの予備の経路でしか通らない。
 */
#include "config.h"

#undef HAVE_MACH_ABSOLUTE_TIME
#define HAVE_MACH_ABSOLUTE_TIME 0

#endif /* __APPLE__ */

#endif /* MINI_FFMPEG_SYS_CINRS_FFMPEG_FIXUPS_H */
