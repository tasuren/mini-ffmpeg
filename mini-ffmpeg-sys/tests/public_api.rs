//! Check that the API translated from public headers is available at the crate root.
//!
//! Other tests call implementation translation unit modules (`libavutil::version`,
//! for example) directly. Their types are distinct per translation unit and are not
//! intended for users. Call functions as users would, such as
//! `mini_ffmpeg_sys::avformat_open_input`, to verify that declarations link to
//! implementation translation unit definitions.

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

/// Check that a declared function can be called. `av_version_info` is defined
/// in the implementation translation unit `libavutil/version.c`.
#[test]
fn calls_function_declared_by_public_header() {
    let version = unsafe { CStr::from_ptr(av_version_info()) };

    assert!(version.to_str().unwrap().starts_with('n'), "{version:?}");
}

/// Check that an enumerator can be passed to a function. Implementation translation
/// units emit enumerators only where used; the public API has all public header enumerators.
#[test]
fn passes_enumerator_to_function() {
    let name = unsafe { CStr::from_ptr(av_get_media_type_string(AVMEDIA_TYPE_AUDIO)) };

    assert_eq!(name.to_str().unwrap(), "audio");
}

/// Check that a struct pointer returned by one function can be passed directly to another.
///
/// In implementation translation units, the same C struct becomes a different Rust
/// type in each unit; passing `avformat.c`'s `AVFormatContext` to a `demux.c` function
/// requires a cast. The public API shares one type, so compiling without a cast
/// itself verifies this property.
#[test]
fn shares_one_struct_type_between_functions() {
    unsafe {
        let context: *mut AVFormatContext = avformat_alloc_context();
        assert!(!context.is_null());

        // Fields are readable too. Immediately after allocation, there are no streams.
        assert_eq!((*context).nb_streams, 0);

        avformat_free_context(context);
    }
}

/// Check that the public API alone can open a file and read stream information.
///
/// The demuxer, file protocol, and packet reading run through implementation
/// translation unit definitions, with results accessible as public API structs.
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

/// Check that the public API file defines no symbols.
///
/// Definitions would duplicate implementation translation unit definitions and fail
/// to link downstream. The build script checks this too, but inspect the translated
/// code as well.
#[test]
fn public_api_defines_no_symbols() {
    let public_api = include_str!(concat!(env!("OUT_DIR"), "/public_api.rs"));

    assert!(!public_api.contains("no_mangle"));
}

/// Assemble a silent 16-bit PCM WAV containing only `fmt ` and `data` chunks.
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
