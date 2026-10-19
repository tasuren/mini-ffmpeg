# mini-ffmpeg

An experimental, small FFmpeg build translated from C to Rust with
[cinrs](https://github.com/tanakh/cinrs). Normal Cargo builds translate the bundled
FFmpeg source without compiling C or linking an installed FFmpeg library.

The project explores how far cinrs can take FFmpeg, with a longer-term interest
in portable builds and a high-level media pipeline API.

## Status

Early-stage research code; not ready for production use.

- `mini-ffmpeg-sys` exposes low-level FFmpeg APIs; WAV input/output and file/pipe
  protocols are enabled.
- `remux-spike` demonstrates WAV-to-WAV copying without re-encoding.
- A high-level library API and project CLI are not implemented yet.
- The current configuration disables threads, codecs, and filters.

macOS and Linux/glibc builds have been tested on arm64 and x86_64, including
cross-compilation. Windows/MSVC configurations are recorded for investigation,
but cinrs cannot yet translate the required MSVC dialect. See the
[build guide](docs/building.md) for the scope of verification and prerequisites.

## Quick start

Use Rust 1.99 or newer and install your platform's C library headers (on macOS,
these come with Xcode or Command Line Tools). A target-specific configuration
is already included for the tested targets.

From the repository root on a tested native target:

```sh
git submodule update --init
cargo test --workspace
cargo run -p remux-spike -- input.wav output.wav
```

See [Building and cross-compiling](docs/building.md) for platform setup and
cross-compilation examples. Regenerating FFmpeg's configuration is a maintainer
task, not a prerequisite for these commands.

## Project layout

| Crate | Purpose |
| --- | --- |
| `mini-ffmpeg` | Placeholder for the future high-level library and CLI. |
| [`mini-ffmpeg-sys`](mini-ffmpeg-sys/README.md) | Translates FFmpeg and exposes its low-level APIs. |
| [`remux-spike`](remux-spike/README.md) | Executable experiment based on FFmpeg's remux example. |
| [`xtask`](xtask/README.md) | Configuration generation and input license scanning. |

## Documentation

- [Architecture](docs/architecture.md): translation, public APIs, and linking.
- [Build configurations](docs/build-configurations.md): regenerate configurations
  when updating FFmpeg or changing build settings, including Windows/MSVC setup.

## License

Project code is licensed under [LGPL-2.1-or-later](LICENSE). FFmpeg and its bundled
third-party code carry additional notices; see
[mini-ffmpeg-sys](mini-ffmpeg-sys/README.md#licenses-and-provenance).

This software is based in part on the work of the Independent JPEG Group.
