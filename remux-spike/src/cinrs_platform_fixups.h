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
 * どの項目も cinrs 0.2.0 と macOS 27 の SDK の組み合わせで必要だったもの。
 * `_Float16` の件を除いて、0.2.0 より後の cinrs では直っている。FFmpeg 本体
 * （`mini-ffmpeg-sys`）の変換は新しい cinrs で行っていて、このファイルを
 * 使わない。このクレートが使う cinrs を上げたら、項目を 1 つずつ外して
 * `cargo test -p remux-spike` が通ることを確かめ、消していける。
 */

#ifndef REMUX_SPIKE_CINRS_PLATFORM_FIXUPS_H
#define REMUX_SPIKE_CINRS_PLATFORM_FIXUPS_H

#if defined(__APPLE__)

/*
 * Apple clang は、arm64 で `__aarch64__` と `__arm64__` の両方を定義する。
 * cinrs が定義するのは `__aarch64__` だけ。
 *
 * SDK のヘッダは `__arm64__` のほうでアーキテクチャを判定している。未定義だと
 * `<machine/_types.h>` が `#error architecture not supported` になり、
 * `__int32_t` などの基本の型が定義されないまま、後続の宣言がすべてエラーになる。
 */
#if defined(__aarch64__) && !defined(__arm64__)
#define __arm64__ 1
#endif

/*
 * SDK の `<math.h>` は、半精度の関数（`__fabsf16` など）を `_Float16` で
 * 無条件に宣言している。cinrs は `_Float16` を扱えず、エラーにする。
 *
 * FFmpeg はこれらの関数を呼ばないので、宣言を読み飛ばせれば十分。型だけを
 * `float` に置き換えて、宣言を通している。宣言の型は実際の関数と食い違うので、
 * これらの関数を呼んではいけない。
 */
#define _Float16 float

#include <sys/cdefs.h>

/*
 * SDK のヘッダには、関数の定義を持つものがある（`<sys/_types/_fd_def.h>` の
 * `__darwin_check_fd_set` など）。これらは `__header_inline` 付きで定義され、
 * GCC として振る舞うコンパイラには `extern __inline __attribute__((gnu_inline))`
 * に展開される。C コンパイラはこの形の定義からシンボルを出力しない。
 *
 * cinrs は、これを外部リンケージを持つ通常の定義として扱う。そのため
 * `#pragma cinrs export` を付けた翻訳単位が 2 つ以上このヘッダを読むと、同じ
 * シンボルを重複して定義することになり、「symbol is already defined」で
 * 失敗する。`static` にして、翻訳単位ごとの内部の関数にしている。
 */
#undef __header_inline
#define __header_inline static __inline

/*
 * SDK のヘッダは、関数の宣言に `__asm("_fputs")` のようなラベルを付けて、
 * リンクするシンボルの名前を指定している（`__DARWIN_ALIAS` など）。ラベルの
 * 文字列は、Mach-O の規約で付く先頭の `_` を含んだ、シンボルの名前そのもの。
 *
 * cinrs は、このラベルを Rust の `#[link_name = "_fputs"]` に変換する。Rust は
 * `link_name` にも先頭の `_` を付けるので、シンボルが `__fputs` になり、
 * リンクで「Undefined symbols」になる。
 *
 * arm64 では、ラベルに付く接尾辞（`$UNIX2003`、`$INODE64`、`$1050`、
 * `$NOCANCEL`）がすべて空で、ラベルを外しても同じシンボルを指す。そこで、
 * 接尾辞が空になるマクロに限って、ラベルを付けないようにしている。
 *
 * x86_64 では `$INODE64` などの接尾辞が実際に付くので、同じ方法は使えない。
 * 外すと古い ABI の関数にリンクされ、構造体のレイアウトが食い違う。
 * `__DARWIN_EXTSN` 系は arm64 でも接尾辞（`$DARWIN_EXTSN`）が付くので、
 * 触っていない。
 */
#if defined(__arm64__)

#undef __DARWIN_ALIAS
#define __DARWIN_ALIAS(sym)
#undef __DARWIN_ALIAS_C
#define __DARWIN_ALIAS_C(sym)
#undef __DARWIN_ALIAS_I
#define __DARWIN_ALIAS_I(sym)
#undef __DARWIN_NOCANCEL
#define __DARWIN_NOCANCEL(sym)
#undef __DARWIN_INODE64
#define __DARWIN_INODE64(sym)
#undef __DARWIN_1050
#define __DARWIN_1050(sym)
#undef __DARWIN_1050ALIAS
#define __DARWIN_1050ALIAS(sym)
#undef __DARWIN_1050ALIAS_C
#define __DARWIN_1050ALIAS_C(sym)
#undef __DARWIN_1050ALIAS_I
#define __DARWIN_1050ALIAS_I(sym)
#undef __DARWIN_1050INODE64
#define __DARWIN_1050INODE64(sym)

#endif /* __arm64__ */

#endif /* __APPLE__ */

#endif /* REMUX_SPIKE_CINRS_PLATFORM_FIXUPS_H */
