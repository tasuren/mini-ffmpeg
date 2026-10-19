# remux-spike

An executable experiment that translates FFmpeg's `doc/examples/remux.c` through
a cinrs macro and links it to `mini-ffmpeg-sys`. It copies packets between
containers without re-encoding; the current FFmpeg configuration supports WAV.
This is a validation example, not the planned mini-ffmpeg CLI.

From the repository root, on a tested native target:

```sh
cargo run -p remux-spike -- input.wav output.wav
cargo test -p remux-spike
```

Use the same [build prerequisites](../docs/building.md) as the sys crate. The
integration test creates a WAV file, remuxes it, and checks the audio data,
format information and container metadata. Workspace tests including this
example have passed on macOS arm64 and Ubuntu/glibc arm64.

The sample is MIT-licensed FFmpeg source. It is included from the unmodified
FFmpeg submodule; see its
[original license notice](../mini-ffmpeg-sys/FFmpeg/doc/examples/remux.c).
