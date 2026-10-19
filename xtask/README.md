# xtask

Maintainer commands for generating FFmpeg build configurations and inspecting
their input licenses. They do not run as part of normal consumer builds.
Run them from the repository root:

```sh
cargo xtask --help
```

## Configuration generation

```sh
cargo xtask ffmpeg-manifest
```

This runs FFmpeg's `configure` and `make` and replaces the configuration for the
selected Rust toolchain's host triple under `mini-ffmpeg-sys/FFmpeg_build/`.
Use it when updating FFmpeg, changing build settings, or investigating a target.
See [Generating build configurations](../docs/build-configurations.md) for
prerequisites, architecture selection, Docker/Rosetta helpers and Windows/MSVC
instructions.

## License scanning

Scan the files recorded by one build manifest:

```sh
cargo xtask ffmpeg-license --target aarch64-apple-darwin
cargo xtask ffmpeg-license --target aarch64-apple-darwin --docker
cargo xtask ffmpeg-license --target aarch64-apple-darwin --native --output target/license-report
```

`--target` is required because each scan describes only that configuration.
Linux defaults to `licensecheck` from PATH (install it separately, for example
with `apt install licensecheck` on Debian). Other operating systems default to
a cached Debian trixie Docker image. `--docker` and `--native` explicitly select
a mode without automatic fallback; the first Docker run requires network access
to obtain the image and packages.

The scan reads the full contents of recorded translation units, include-only
sources, headers and generated files. It requires neither a C compiler nor an
FFmpeg rebuild. It excludes system headers and cinrs/Cargo dependencies, and does
not evaluate preprocessor branches or reachability in the final binary.

By default, output is saved under `target/xtask/ffmpeg-license/<target>/`:

| File | Contents |
| --- | --- |
| `report.md` | Summary and per-file detection results. |
| `licensecheck.tsv` | Detected license names and copyright notices. |
| `versions.txt` | Tool versions. |
| `manifest.toml` | Copy of the scanned build manifest. |

`--output` is relative to the working directory. Use
`cargo xtask ffmpeg-license --help` for the command's complete usage text.

These are detection results. `UNKNOWN` does not mean that no license applies;
IJG notices can also be reported as unknown. The tool's `and/or` output is not a
confirmed SPDX expression or a determination that a license choice is available.
See the sys crate's [license and provenance notes](../mini-ffmpeg-sys/README.md#licenses-and-provenance).
