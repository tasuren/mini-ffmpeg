# Scripts

Maintainer helpers for generating target-specific FFmpeg configurations. Normal
crate builds do not invoke these scripts.

| Script | Configuration | Requirements |
| --- | --- | --- |
| `ffmpeg-manifest-x86_64-apple-darwin.sh` | `x86_64-apple-darwin` | Apple Silicon Mac, Rosetta, matching x86_64 Rust toolchain. |
| `ffmpeg-manifest-linux-docker.sh arm64` | `aarch64-unknown-linux-gnu` | Docker able to run Linux arm64. |
| `ffmpeg-manifest-linux-docker.sh amd64` | `x86_64-unknown-linux-gnu` | Docker able to run Linux amd64. |

See [Generating build configurations](../docs/build-configurations.md) for the
workflow, environment overrides and architecture checks. Docker uses emulation
when the container architecture differs from its host; otherwise it runs that
architecture directly.

There is no Windows helper script. The documented
[Windows/MSVC procedure](../docs/build-configurations.md#windowsmsvc) uses a
Visual Studio tools prompt and an MSYS2 shell.
