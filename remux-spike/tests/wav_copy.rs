//! Checks that FFmpeg translated by cinrs can copy WAV to WAV.
//!
//! Earlier tests called individual functions. This test exercises the full
//! sequence: opening a file, parsing a container, reading packets, writing them
//! to another file, and finally seeking back to the beginning to write the size.
//! It checks that the demuxer, muxer, file protocol, and packet management work
//! together in the translated code.
//!
//! The output has been manually confirmed to match the original C build byte
//! for byte by comparing it against the same `remux.c` built with a C compiler.
//! That comparison requires a C compiler and a C build of FFmpeg, so it is not
//! part of this test.

use std::{fs, path::PathBuf};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const BYTES_PER_SAMPLE: u16 = 2;

/// Generates one second of 16-bit stereo PCM, with different sine-wave
/// frequencies for the left and right channels.
///
/// Identical channel contents would hide swapped channels or a channel being
/// ignored.
fn sine_samples() -> Vec<u8> {
    let mut samples = Vec::new();

    for index in 0..SAMPLE_RATE {
        let time = f64::from(index) / f64::from(SAMPLE_RATE);

        for frequency in [440.0, 660.0] {
            let value = (12_000.0 * (std::f64::consts::TAU * frequency * time).sin()) as i16;
            samples.extend_from_slice(&value.to_le_bytes());
        }
    }

    samples
}

/// Builds a minimal WAV file containing only the `fmt ` and `data` chunks.
fn wav_file(samples: &[u8]) -> Vec<u8> {
    let block_align = CHANNELS * BYTES_PER_SAMPLE;
    let byte_rate = SAMPLE_RATE * u32::from(block_align);

    let mut format = Vec::new();
    format.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    format.extend_from_slice(&CHANNELS.to_le_bytes());
    format.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    format.extend_from_slice(&byte_rate.to_le_bytes());
    format.extend_from_slice(&block_align.to_le_bytes());
    format.extend_from_slice(&(BYTES_PER_SAMPLE * 8).to_le_bytes());

    let mut body = b"WAVE".to_vec();

    for (id, data) in [(b"fmt ", format.as_slice()), (b"data", samples)] {
        body.extend_from_slice(id);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
    }

    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&body);

    file
}

/// Splits a RIFF file into chunk IDs and contents. Also checks that the size
/// field in the header matches the actual file length.
fn riff_chunks(file: &[u8]) -> Vec<([u8; 4], &[u8])> {
    assert_eq!(&file[..4], b"RIFF");
    assert_eq!(&file[8..12], b"WAVE");

    // The WAV muxer seeks back to write this size after writing the entire file.
    // If seeking fails, the placeholder value remains here.
    let riff_size = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    assert_eq!(riff_size, file.len() - 8);

    let mut chunks = Vec::new();
    let mut position = 12;

    while position + 8 <= file.len() {
        let id: [u8; 4] = file[position..position + 4].try_into().unwrap();
        let size = u32::from_le_bytes(file[position + 4..position + 8].try_into().unwrap());
        let size = size as usize;

        chunks.push((id, &file[position + 8..position + 8 + size]));

        // Chunks are aligned to even byte boundaries.
        position += 8 + size + (size & 1);
    }

    chunks
}

fn find_chunk<'a>(chunks: &[([u8; 4], &'a [u8])], id: &[u8; 4]) -> &'a [u8] {
    chunks
        .iter()
        .find(|(chunk_id, _)| chunk_id == id)
        .unwrap_or_else(|| panic!("expected a {} chunk", String::from_utf8_lossy(id)))
        .1
}

#[test]
fn copies_wav_to_wav_without_changing_audio() {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let input_path = directory.join("wav_copy_input.wav");
    let output_path = directory.join("wav_copy_output.wav");

    let samples = sine_samples();
    let input = wav_file(&samples);
    fs::write(&input_path, &input).unwrap();

    // Leftover output from a previous run could pass validation even if this
    // run writes nothing.
    let _ = fs::remove_file(&output_path);

    assert_eq!(remux_spike::remux(&input_path, &output_path), 0);

    let output = fs::read(&output_path).unwrap();
    let input_chunks = riff_chunks(&input);
    let output_chunks = riff_chunks(&output);

    // A copy without re-encoding preserves both the audio data and format
    // description byte for byte.
    assert_eq!(find_chunk(&output_chunks, b"data"), samples);
    assert_eq!(
        find_chunk(&output_chunks, b"fmt "),
        find_chunk(&input_chunks, b"fmt "),
    );

    // The muxer adds a `LIST` chunk containing the writing library's name and
    // version, so the entire output file differs from the input. `Lavf` here
    // identifies the translated libavformat as the library that wrote it.
    let list = find_chunk(&output_chunks, b"LIST");
    assert!(list.windows(4).any(|window| window == b"Lavf"), "{list:?}");
}
