//! `libavutil/log.c` の取り込み。手書き。
//!
//! この翻訳単位は xtask の `EXCLUDED_UNITS` に載っていて、`cargo xtask
//! ffmpeg-units` は取り込みのファイルを生成しない。FFmpeg のツリーにある
//! `log.c` は cinrs で変換できないので、変更を加えたコピーを隣の `log.c` に
//! 置き、そちらを取り込んでいる。何を変えたかは、`log.c` の先頭に書いてある。
//!
//! 生成される取り込みとの違いは、最後の `#include` だけ。FFmpeg のツリーの
//! `"libavutil/log.c"` ではなく、このファイルの隣の `"log.c"` を読む。cinrs は
//! `.rs` ファイルの場所を最初に探すので、インクルードディレクトリを足さなくても
//! 見つかる。それより上の部分は、生成される取り込みと揃えておく必要がある。
//! 取り込みの形が変わったときは、生成されたファイル（`time.rs` など）を見て、
//! ここも合わせる。

cinrs::c17! {
    #pragma cinrs export
    #pragma cinrs system_include first

    #include "../cinrs_platform_fixups.h"

    #pragma cinrs include_path "FFmpeg_generated"
    #pragma cinrs include_path "FFmpeg"
    #pragma cinrs include_path "FFmpeg/compat/dispatch_semaphore"
    #pragma cinrs include_path "FFmpeg/compat/stdbit"

    #define _ISOC11_SOURCE 1
    #define _FILE_OFFSET_BITS 64
    #define _LARGEFILE_SOURCE 1
    #define PIC 1
    #define HAVE_AV_CONFIG_H 1

    #include "../cinrs_ffmpeg_fixups.h"

    #include "log.c"
}
