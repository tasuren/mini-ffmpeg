# mini-ffmpeg-sys

Low-level FFmpeg library APIs from a small configuration, translated from C to
Rust with cinrs at build time. Normal Cargo builds do not compile C sources or
link an installed FFmpeg library.

This is experimental code. Successful builds and tests do not establish the
correctness of every translated operation or ABI. The current configuration
supports WAV input/output and file/pipe protocols, with threads, codecs and
filters disabled.

## Building

Initialize the FFmpeg submodule and install the target's platform headers, then
run from the repository root:

```sh
git submodule update --init # if you have never done it
cargo test -p mini-ffmpeg-sys
```

See [Building and cross-compiling](../docs/building.md) for prerequisites,
verified targets, and macOS/Linux cross-compilation examples. macOS and
Linux/glibc have successful build/test records for arm64 and x86_64. Windows/MSVC
configurations are present, but cinrs does not support the required MSVC dialect.
A recorded configuration alone does not mean that a target builds.

Cross-compilation and Windows/MSVC builds require `CINRS_SYSTEM_INCLUDE_PATH`
to identify the target's headers explicitly. Windows/MSVC requires it even for
native builds because cinrs does not discover those headers automatically.
The build script rejects a missing setting; it cannot check that supplied
headers actually belong to the intended target. This setting does not resolve
the current MSVC dialect limitations.

Normal users do not need to regenerate configurations. For FFmpeg updates and
configuration changes, see [Generating build configurations](../docs/build-configurations.md).

## Public API

Functions, structs and enumerators from the recorded public headers are exposed
at the crate root under their C names, such as
`mini_ffmpeg_sys::avformat_open_input`. Calls use the unsafe C API and its
ownership and lifetime rules; this crate does not provide a safe wrapper.

Public declarations link to definitions translated from individual FFmpeg `.c`
files. Use the crate-root types and declarations rather than implementation
modules, which have separate Rust types and are hidden from documentation.
The Android-only `av_jni_get_android_app_ctx` and `av_jni_set_android_app_ctx`
remain declared but have no implementations in the current configurations;
calling them fails to link.

Platform headers can contribute declarations as well. Target-specific
workarounds validate and remove known declarations whose types were substituted
for parsing; those substitutions do not implement the original ABI. See
[Architecture](../docs/architecture.md) for the translation paths, validation and
rebuilding behavior.

## Source layout

| Path | Purpose |
| --- | --- |
| `build.rs`, `build/` | Translation and validation during Cargo builds. |
| `FFmpeg/` | Unmodified upstream FFmpeg source submodule. |
| `FFmpeg_build/<target-triple>/` | Generated manifest, configuration headers and other configure/make output for one target. |
| `LICENSE.ffmpeg` | Copy of upstream `COPYING.LGPLv2.1`. |

Translated Rust is generated in Cargo's `OUT_DIR`, not committed or distributed
as pretranslated source. It can contain material read from local platform
headers, so translation takes place on the machine building the crate.

## Licenses and provenance

Project code is licensed under GNU Lesser General Public License version 2.1
or later (`LGPL-2.1-or-later`); see the root [LICENSE](../LICENSE).

FFmpeg comes from the revision recorded by the `FFmpeg/` submodule.
[LICENSE.ffmpeg](LICENSE.ffmpeg) is an unchanged copy of that revision's
`COPYING.LGPLv2.1`. Also consult upstream [LICENSE.md](FFmpeg/LICENSE.md) and
individual file notices, since the source tree includes other licenses.

### Generated configuration files

Files under `FFmpeg_build/<target-triple>/`, except `cinrs_manifest.toml`, are
copied unchanged from FFmpeg's `configure`/`make` output. They include
configuration and version headers and component registration tables. They are
included as FFmpeg-derived configuration files in this crate's distribution.

The manifest records the configuration and generating environment. The current
configurations do not enable GPL, nonfree or version3 options. Compiler strings
in `config.h` describe the generating environment. Regeneration replaces the
whole target directory, so handwritten documentation belongs outside it.

### Additional notices

Files used by the current configuration carry the following additional notices.
Read the original conditions in each file; this table is not a substitute for
them. Paths are relative to `FFmpeg/`.

| License | Files |
| --- | --- |
| IJG (libjpeg-derived) | `libavcodec/jfdctfst.c`, `libavcodec/jfdctint_template.c`, `libavcodec/jrevdct.c` |
| BSD-3-Clause (alongside LGPL notices) | `libavutil/fixed_dsp.c`, `libavutil/fixed_dsp.h`, `libavutil/uuid.c` |
| BSL-1.0 (alongside LGPL notices; applies to certain functions) | `libavutil/mathematics.c`, `libavutil/libm.h` |
| ISC | `libavcodec/faandct.c` |
| MIT | `libavutil/avsscanf.c` |
| Zlib | `libavutil/adler32.c` |

For the three libjpeg-derived files:

> This software is based in part on the work of the Independent JPEG Group.

These files are used unchanged from FFmpeg's tree. For FFmpeg's changes to the
original libjpeg files, consult upstream history. Copyright, warranty and
distribution conditions remain in their source notices. Include the above
credit in accompanying documentation when distributing an executable that uses
this code.

Rust builds normally link crate code statically, including the translated
FFmpeg code. Redistribution must account for the applicable original license
conditions; see also [FFmpeg's licensing information](https://ffmpeg.org/legal.html).
The maintainer's [input license scan](../xtask/README.md#license-scanning) helps
inspect recorded inputs but does not establish redistribution compliance.

### Packaging

`Cargo.toml` selects FFmpeg source, generated configuration files, build scripts
and license files for packaging. Inspect the selection from the repository root:

```sh
cargo package -p mini-ffmpeg-sys --list --allow-dirty
```

The crate is not published (`publish = false`). Source selection for packaging
and the docs.rs build setup still need work.
