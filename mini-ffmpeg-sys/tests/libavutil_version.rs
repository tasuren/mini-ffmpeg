//! `libavutil/version.c` を cinrs で取り込んだ結果を、Rust から呼んで確かめる。
//!
//! 結合テストにしているのは、リンクまで通ることを確かめるため。ライブラリの
//! クレートは `cargo build` だけではリンクされず、未解決のシンボルがあっても
//! 通ってしまう。

use std::ffi::CStr;

use mini_ffmpeg_sys::libavutil::version;

/// `cinrs_manifest.toml` の `[ffmpeg]` にある `version` の値を取り出す。
///
/// 期待値をテストに直接書くと、FFmpeg のバージョンを上げるたびに書き換えが
/// 必要になる。ツールが同じビルドから読み取った値と突き合わせる。
fn manifest_ffmpeg_version() -> &'static str {
    include_str!("../cinrs_manifest.toml")
        .lines()
        .find_map(|line| line.strip_prefix("version = "))
        .expect("cinrs_manifest.toml に version がある")
        .trim_matches('"')
}

/// `FFMPEG_VERSION` は `make` が生成した `libavutil/ffversion.h` にある。
/// この値が返るなら、`FFmpeg_generated` をインクルードディレクトリとして
/// 読めていて、C の関数を Rust から呼べている。
#[test]
fn av_version_info_returns_ffversion_h_value() {
    let version = unsafe { CStr::from_ptr(version::av_version_info()) };

    assert_eq!(version.to_str().unwrap(), manifest_ffmpeg_version());
}

/// `FFMPEG_LICENSE` は `configure` が生成した `config.h` にある。GPL の部分を
/// 入れない方針なので、取り込んだ構成が LGPL であることもここで確かめる。
///
/// C の側は、`"libavutil license: " FFMPEG_LICENSE` という連結した文字列
/// リテラルの途中を指すポインタを返している。
#[test]
fn avutil_license_returns_config_h_value() {
    let license = unsafe { CStr::from_ptr(version::avutil_license()) };

    assert_eq!(license.to_str().unwrap(), "LGPL version 2.1 or later");
}

/// `LIBAVUTIL_VERSION_INT` は `major << 16 | minor << 8 | micro` で、micro は
/// 100 から始まる。値そのものは FFmpeg のバージョンで変わるので、構造だけを見る。
/// FFmpeg のツリーのヘッダ（`libavutil/version.h`）を読めていることの確認。
#[test]
fn avutil_version_has_expected_layout() {
    let version = unsafe { version::avutil_version() };

    assert!(version >> 16 > 0, "major: {version:#x}");
    assert!(version & 0xff >= 100, "micro: {version:#x}");
}
