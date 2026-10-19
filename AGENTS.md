# AGENTS.md

## Project overview

mini-ffmpeg is an experimental project that translates FFmpeg's C source to Rust
with cinrs. Normal Cargo builds perform translation in build scripts; they do not
compile C sources or link an installed FFmpeg library.

`mini-ffmpeg-sys` translates implementation units and a separate public API unit
with cinrs. The public API exposes a shared set of Rust types and declarations
for callers, linking to the implementation units through C symbols.

FFmpeg configuration generation is a separate maintainer task that runs
`configure` and `make` through xtask. Consumers normally use committed configurations.

## Repository layout

- `mini-ffmpeg/`: Placeholder for the future high-level library and CLI.
- `mini-ffmpeg-sys/`: FFmpeg translation, low-level public APIs, and validation.
- `remux-spike/`: Executable experiment translating FFmpeg's remux example.
- `xtask/`: Build configuration generation and input license scanning.
- `mini-ffmpeg-sys/FFmpeg/`: Unmodified upstream source, tracked as a Git submodule.
- `mini-ffmpeg-sys/FFmpeg_build/<target-triple>/`: Generated manifests and matching
  configuration files for each target.

## Working conventions

- Write comments, documentation, and diagnostic messages in English.
- Keep upstream FFmpeg sources unmodified. Change project-owned translation or
  configuration code instead of patching the submodule's files.
- Do not hand-edit generated build configurations. Change the generator and
  regenerate the affected target's complete configuration with xtask.
- Keep detailed procedures in the linked documentation rather than duplicating
  them here.

## Validation and references

Initialize the source submodule with `git submodule update --init` when needed.

The baseline code validation is `cargo test --workspace`; use focused tests for
the affected crate as appropriate. Documentation-only edits generally need link
and example checks rather than a full build.

Report which targets and checks were actually exercised. Distinguish translation,
compilation, linking, and execution; a committed configuration alone does not
establish target support. Consult the build guide for target header requirements.

- [Project introduction](README.md)
- [Building and cross-compiling](docs/building.md)
- [Architecture and public API translation](docs/architecture.md)
- [Generating build configurations](docs/build-configurations.md)
- [Maintainer commands and license scanning](xtask/README.md)
