/*
 * cinrs のマクロでプラットフォームのヘッダを読むための回避策。
 *
 * プラットフォームのヘッダは、そのプラットフォームの C コンパイラ（macOS なら
 * Apple clang）に読まれる前提で書かれている。cinrs は GCC として振る舞うので、
 * ヘッダが clang に期待している定義や挙動と食い違う箇所が出る。ここでは、その
 * 食い違いをプリプロセッサのレベルで埋める。
 *
 * `#pragma cinrs system_include first` を使う翻訳単位は、プラットフォームの
 * ヘッダを include する前に、このファイルを include する。
 *
 * FFmpeg 本体（`mini-ffmpeg-sys`）の変換も、同じ回避策を、ビルドスクリプトが
 * マクロの定義として渡している（`mini-ffmpeg-sys/build/translate.rs` の
 * `platform_workarounds`）。項目を増減するときは、両方を合わせる。
 *
 * cinrs の側で直れば消せる。消すときは、その項目を外しても
 * `cargo test -p remux-spike` が通ることを確かめる。
 */

#ifndef REMUX_SPIKE_CINRS_PLATFORM_FIXUPS_H
#define REMUX_SPIKE_CINRS_PLATFORM_FIXUPS_H

#if defined(__APPLE__)

/*
 * SDK の `<math.h>` は、半精度の関数（`__fabsf16` など）を `_Float16` で
 * 無条件に宣言している。cinrs は `_Float16` を扱えず、宣言を読んだだけで
 * エラーにする。
 *
 * FFmpeg はこれらの関数を呼ばないので、宣言を読み飛ばせれば十分。型だけを
 * `float` に置き換えて、宣言を通している。宣言の型は実際の関数と食い違うので、
 * これらの関数を呼んではいけない。
 */
#define _Float16 float

#endif /* __APPLE__ */

#endif /* REMUX_SPIKE_CINRS_PLATFORM_FIXUPS_H */
