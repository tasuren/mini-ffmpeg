//! cinrs で取り込んだ FFmpeg で、WAV から WAV へのコピーが実際に動くことを確かめる。
//!
//! ここまでのテストは、関数を 1 つずつ呼ぶものだった。このテストは、ファイルを
//! 開き、コンテナを解釈し、パケットを読み、別のファイルへ書き、最後に先頭へ戻って
//! サイズを書き込む、という一連の処理を通す。demuxer、muxer、ファイルの
//! プロトコル、パケットの管理が、変換後のコードで噛み合って動くことの確認になる。
//!
//! 出力が元の C のビルドと 1 バイトも違わないことは、同じ `remux.c` を C
//! コンパイラでビルドしたものと突き合わせて、手で確かめてある。その比較には
//! C コンパイラと FFmpeg の C のビルドが要るので、テストには入れていない。

use std::{fs, path::PathBuf};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
const BYTES_PER_SAMPLE: u16 = 2;

/// 1 秒分の、16 ビット、ステレオの PCM。左右で違う周波数の正弦波にしている。
///
/// 左右が同じ内容だと、チャンネルが入れ替わったり片方だけ読まれたりしても
/// 気づけない。
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

/// 最小の構成の WAV（`fmt ` と `data` のチャンクだけ）を組み立てる。
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

/// RIFF のファイルを、チャンクの ID と中身の組に分ける。先頭のサイズの欄が
/// ファイルの実際の長さと合っていることも確かめる。
fn riff_chunks(file: &[u8]) -> Vec<([u8; 4], &[u8])> {
    assert_eq!(&file[..4], b"RIFF");
    assert_eq!(&file[8..12], b"WAVE");

    // WAV の muxer は、最後まで書いてから先頭へ戻って、このサイズを書き込む。
    // シークが効いていなければ、ここが仮の値のまま残る。
    let riff_size = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
    assert_eq!(riff_size, file.len() - 8);

    let mut chunks = Vec::new();
    let mut position = 12;

    while position + 8 <= file.len() {
        let id: [u8; 4] = file[position..position + 4].try_into().unwrap();
        let size = u32::from_le_bytes(file[position + 4..position + 8].try_into().unwrap());
        let size = size as usize;

        chunks.push((id, &file[position + 8..position + 8 + size]));

        // チャンクは偶数バイトの境界に揃えられる。
        position += 8 + size + (size & 1);
    }

    chunks
}

fn find_chunk<'a>(chunks: &[([u8; 4], &'a [u8])], id: &[u8; 4]) -> &'a [u8] {
    chunks
        .iter()
        .find(|(chunk_id, _)| chunk_id == id)
        .unwrap_or_else(|| panic!("{} のチャンクがある", String::from_utf8_lossy(id)))
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

    // 前回の実行の出力が残っていると、今回書けていなくても検証が通ってしまう。
    let _ = fs::remove_file(&output_path);

    assert_eq!(remux_spike::remux(&input_path, &output_path), 0);

    let output = fs::read(&output_path).unwrap();
    let input_chunks = riff_chunks(&input);
    let output_chunks = riff_chunks(&output);

    // 再エンコードしないコピーなので、音声のデータもフォーマットの記述も、
    // 入力と 1 バイトも変わらない。
    assert_eq!(find_chunk(&output_chunks, b"data"), samples);
    assert_eq!(
        find_chunk(&output_chunks, b"fmt "),
        find_chunk(&input_chunks, b"fmt "),
    );

    // muxer は、書き出したライブラリの名前とバージョンを `LIST` のチャンクに
    // 入れる。ファイル全体が入力と同じにならないのは、このチャンクが増えるため。
    // ここに `Lavf` があれば、出力を書いたのは取り込んだ libavformat である。
    let list = find_chunk(&output_chunks, b"LIST");
    assert!(list.windows(4).any(|window| window == b"Lavf"), "{list:?}");
}
