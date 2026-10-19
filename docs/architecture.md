# Architecture

mini-ffmpeg translates the bundled FFmpeg C source to Rust at Cargo build time.
It does not load an installed FFmpeg library or invoke the FFmpeg command-line
program. The current experiment uses FFmpeg `n9.0.2` with a small WAV-oriented
configuration.

## Recorded configurations

A maintainer first runs FFmpeg's native build tools through
`cargo xtask ffmpeg-manifest`. The task records:

- Compile commands: translation units, include directories and macro definitions.
- Dependency files: included sources, headers and configure/make output.
- Header installation commands: public headers used by the selected units.

The manifest and required generated files are committed together under
`mini-ffmpeg-sys/FFmpeg_build/<target>/`. This lets normal Cargo builds translate
the same configuration without running `configure` or `make`. See
[Generating build configurations](build-configurations.md).

## Two translation paths

[`mini-ffmpeg-sys/build.rs`](../mini-ffmpeg-sys/build.rs) calls the pinned
`cinrs-core` translator for two kinds of translation units:

| Kind | Input | Output |
| --- | --- | --- |
| Implementation | One FFmpeg `.c` file per unit | Function/variable definitions exported as C symbols. |
| Public API | One unit including all recorded public headers | Shared Rust types, enumerators and foreign declarations, re-exported at the crate root. |

Implementation units use cinrs' `reachable_only` to omit unused header items.
Each has its own Rust types, and units link through C symbol names. The public API
unit keeps declarations and types together without defining `HAVE_AV_CONFIG_H`,
as an FFmpeg consumer would. It does not export definitions as C symbols.

Without a separate public API unit, callers would have to cast between distinct
Rust types when passing values across implementation units, even when those types
originate from the same C declaration. The public API provides one shared set of
types for callers, while its function declarations link to the implementation
units through C symbols.

The build script checks for duplicate implementation symbols and public FFmpeg
function/variable declarations lacking implementations, with explicit exceptions
for the two Android-only JNI functions. These checks concern symbol presence;
they do not prove correctness of every ABI or translated operation.

## Platform headers and workarounds

Translation reads the build machine's platform headers for the intended target.
This is why the SDK or libc development headers are prerequisites, and why
[cross-builds require explicit header paths](building.md#cross-compilation-rules).

The public header translation also encounters system declarations unrelated to
FFmpeg. Target-specific
[`WorkaroundSet` policies](../mini-ffmpeg-sys/build/platform_workarounds/model.rs)
allow parsing known unsupported declarations, validate their original types
through macro origins, and exclude the affected functions and aliases from the
public API. Unexpected uses or retained references stop the build. Current
policies cover macOS half-precision math and AArch64 glibc vector math; scalar
stand-ins do not implement their real ABI.

## Generated Rust and rebuilding

The build script writes translated units, `public_api.rs`, and module declarations
in Cargo's `OUT_DIR`. [`src/lib.rs`](../mini-ffmpeg-sys/src/lib.rs) includes the
module declarations and exposes the public API. Implementation modules remain
available but are hidden from documentation.

Generated Rust is not checked in or bundled as pretranslated source. It includes
material read from local platform headers and is regenerated on the machine
building the crate. The build script tracks translation inputs and header-path
settings so Cargo reruns it when those inputs change. Translation units are
processed in parallel.

## Remux experiment

[`remux-spike`](../remux-spike/README.md) translates FFmpeg's
`doc/examples/remux.c` with a cinrs macro. Its build script shares the platform
validation policy with the sys crate, and its C calls link to the sys crate's
exported symbols. It provides an end-to-end check of packet copying without
re-encoding; it is not the planned project CLI or high-level API.
