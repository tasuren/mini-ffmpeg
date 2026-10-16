//! 公開ヘッダから変換した API を、クレートのルートのパスで使えることを確かめる。
//!
//! 他のテストは、実装の翻訳単位のモジュール（`libavutil::version` など）を直接
//! 呼んでいる。そちらは翻訳単位ごとに型が別物で、利用者に見せるものではない。
//! ここでは、利用者と同じく `mini_ffmpeg_sys::avformat_open_input` のように呼び、
//! 宣言がリンクのときに実装の翻訳単位の定義に繋がることを確かめる。

use std::{
    env,
    ffi::{CStr, CString},
    fs, process, ptr,
};

use mini_ffmpeg_sys::{
    AVFormatContext, AVMEDIA_TYPE_AUDIO, av_get_media_type_string, av_version_info,
    avformat_alloc_context, avformat_close_input, avformat_find_stream_info, avformat_free_context,
    avformat_open_input,
};

/// 宣言だけの関数を呼べること。`av_version_info` の実体は、実装の翻訳単位の
/// `libavutil/version.c` にある。
#[test]
fn calls_function_declared_by_public_header() {
    let version = unsafe { CStr::from_ptr(av_version_info()) };

    assert!(version.to_str().unwrap().starts_with('n'), "{version:?}");
}

/// 列挙子を、関数の引数に渡せること。列挙子は、実装の翻訳単位からは、それを
/// 使う翻訳単位にしか出力されない。公開 API には、公開ヘッダのすべての列挙子がある。
#[test]
fn passes_enumerator_to_function() {
    let name = unsafe { CStr::from_ptr(av_get_media_type_string(AVMEDIA_TYPE_AUDIO)) };

    assert_eq!(name.to_str().unwrap(), "audio");
}

/// ある関数が返した構造体へのポインタを、別の関数にそのまま渡せること。
///
/// 実装の翻訳単位では、同じ C の構造体でも翻訳単位ごとに別の Rust の型になり、
/// `avformat.c` の `AVFormatContext` を `demux.c` の関数に渡すには変換が要る。
/// 公開 API では型が 1 つなので、変換無しでコンパイルが通ること自体が確認になる。
#[test]
fn shares_one_struct_type_between_functions() {
    unsafe {
        let context: *mut AVFormatContext = avformat_alloc_context();
        assert!(!context.is_null());

        // フィールドも読める。確保した直後は、ストリームが 1 つも無い。
        assert_eq!((*context).nb_streams, 0);

        avformat_free_context(context);
    }
}

/// 公開 API だけで、ファイルを開いてストリームの情報を読めること。
///
/// demuxer、ファイルのプロトコル、パケットの読み取りが、実装の翻訳単位の
/// 定義を通って動き、その結果を公開 API の構造体として読める。
#[test]
fn opens_wav_file_through_public_api() {
    let path = env::temp_dir().join(format!("mini-ffmpeg-sys-public-api-{}.wav", process::id()));
    fs::write(&path, silent_wav(48_000, 2, 4_800)).unwrap();

    let url = CString::new(path.to_str().unwrap()).unwrap();

    unsafe {
        let mut context: *mut AVFormatContext = ptr::null_mut();

        let status = avformat_open_input(&mut context, url.as_ptr(), ptr::null(), ptr::null_mut());
        assert_eq!(status, 0, "avformat_open_input");
        assert!(avformat_find_stream_info(context, ptr::null_mut()) >= 0);

        assert_eq!((*context).nb_streams, 1);

        let parameters = (**(*context).streams).codecpar;
        assert_eq!((*parameters).codec_type, AVMEDIA_TYPE_AUDIO);
        assert_eq!((*parameters).sample_rate, 48_000);
        assert_eq!((*parameters).ch_layout.nb_channels, 2);

        avformat_close_input(&mut context);
        assert!(context.is_null());
    }

    fs::remove_file(&path).unwrap();
}

/// 公開 API のファイルが、シンボルを定義していないこと。
///
/// 定義していると、実装の翻訳単位の定義と重なり、利用する側のリンクで失敗する。
/// ビルドスクリプトも同じことを確かめているが、変換後のコードの側からも見ておく。
#[test]
fn public_api_defines_no_symbols() {
    let public_api = include_str!(concat!(env!("OUT_DIR"), "/public_api.rs"));

    assert!(!public_api.contains("no_mangle"));
}

/// 無音の 16 ビット PCM の WAV（`fmt ` と `data` のチャンクだけ）を組み立てる。
fn silent_wav(sample_rate: u32, channels: u16, frames: u32) -> Vec<u8> {
    const BYTES_PER_SAMPLE: u16 = 2;

    let block_align = channels * BYTES_PER_SAMPLE;
    let data_size = frames * u32::from(block_align);

    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&(BYTES_PER_SAMPLE * 8).to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.resize(wav.len() + data_size as usize, 0);

    wav
}
