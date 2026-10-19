//! Read headers installed as FFmpeg's public API from `make -n install-headers` output.
//!
//! FFmpeg's `ffbuild/library.mak` installs headers for each library as follows:
//!
//! ```make
//! install-lib$(NAME)-headers: $(addprefix $(SUBDIR),$(HEADERS) $(BUILT_HEADERS))
//!     $(Q)mkdir -p "$(INCINSTDIR)"
//!     $$(INSTALL) -m 644 $$^ "$(INCINSTDIR)"
//! ```
//!
//! `INCINSTDIR` is `<prefix>/include/lib<name>`. Consumers include headers using the
//! library directory name, as in `#include <libavutil/avutil.h>`. Assume commands end
//! with `... -m 644 <headers>... <directory>` to extract the headers and their
//! `#include` names.
//!
//! Nothing is actually installed: `-n` only prints commands, allowing public headers
//! to be identified without writing to the installation destination.

use std::collections::BTreeSet;

use super::{
    compile_command::split_shell_words,
    source_tree::{Location, SourceTree},
};
use crate::Result;

/// One header to be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledHeader {
    /// The source file, with its path as written in the command.
    pub source: String,

    /// The destination directory name: the library name (such as `libavutil`).
    pub library: String,
}

impl InstalledHeader {
    /// The name consumers use in `#include`, such as `libavutil/avutil.h`.
    ///
    /// `install` copies files directly into the destination directory, so only the
    /// library name and file name matter, regardless of the source location.
    fn include_name(&self) -> String {
        let file_name = self.source.rsplit('/').next().unwrap_or(&self.source);

        format!("{}/{file_name}", self.library)
    }
}

/// Parse one command line and return its contents if it installs headers.
///
/// Return an empty `Vec` for other commands, such as `mkdir` or installation of
/// pkg-config files.
///
/// # Errors
///
/// Reject configurations that install headers and other files in the same command:
/// they violate the assumption that public headers can be extracted independently.
pub fn parse_line(line: &str) -> Result<Vec<InstalledHeader>> {
    // Ignore lines with unterminated quotes, which cannot be parsed as commands.
    let Some(words) = split_shell_words(line) else {
        return Ok(Vec::new());
    };

    // The install program's name can vary by environment (`configure` chooses it).
    // Identify it by `-m 644`, which FFmpeg's Makefile always adds, rather than by name.
    // Search within the line because it follows a progress message
    // (`printf "INSTALL\t%s\n" ...;`) on the same line.
    let Some(mode) = words
        .windows(2)
        .position(|pair| pair[0] == "-m" && pair[1] == "644")
    else {
        return Ok(Vec::new());
    };

    // The words after `-m 644` are source files followed by the destination.
    let [sources @ .., destination] = &words[mode + 2..] else {
        return Ok(Vec::new());
    };

    let is_header = |source: &&String| source.ends_with(".h");

    if !sources.iter().any(|source| is_header(&source)) {
        return Ok(Vec::new());
    }

    if let Some(other) = sources.iter().find(|source| !is_header(source)) {
        return Err(format!(
            "expected only public headers to be installed, but \
            the current FFmpeg configuration installs headers and non-header files \
            in the same command: {other}"
        )
        .into());
    }

    let library = destination
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("cannot parse header installation destination: {destination}"))?;

    Ok(sources
        .iter()
        .map(|source| InstalledHeader {
            source: source.clone(),
            library: library.to_owned(),
        })
        .collect())
}

/// Select installed headers that translation units actually included.
/// Return their `#include` names (such as `libavutil/avutil.h`) in alphabetical order.
///
/// `headers` and `generated` correspond to the build manifest fields of those names.
///
/// # Why not use all installed headers?
///
/// The set of installed headers is mostly independent of the `configure`
/// configuration, so it includes headers unusable in the current configuration:
///
/// - Headers requiring unavailable dependencies. `libavcodec/d3d11va.h` includes
///   Windows's `<d3d11.h>`, and `libavcodec/vdpau.h` includes `<vdpau/vdpau.h>`.
///   Including them fails immediately.
/// - Headers whose implementations are not compiled. `libavcodec/smpte_436m.h` can
///   be included on its own, but `smpte_436m.c` is not compiled in this configuration.
///   Its declarations are visible, but calling them fails at link time.
///
/// Restricting the set to headers included by translation units excludes both cases.
/// Included headers were readable in the original build. Also, a compiled
/// implementation `.c` includes its own header.
///
/// This does not exclude declarations in otherwise used headers whose implementations
/// are unavailable in this environment. `libavcodec/jni.h` unconditionally declares
/// Android-only functions. That requires handling individual declarations rather
/// than entire headers.
///
/// # Errors
///
/// - A header is located at a different path from its `#include` name. The build
///   manifest stores only names, assuming they match paths in the source tree.
/// - The selection is empty, indicating that the command format assumptions failed.
pub fn select_public_headers(
    installed: &[InstalledHeader],
    tree: &SourceTree,
    headers: &[String],
    generated: &[String],
) -> Result<Vec<String>> {
    let mut public = BTreeSet::new();

    for header in installed {
        let include_name = header.include_name();

        // Source tree headers use absolute `SRC_PATH` paths; generated files use paths
        // relative to the build directory.
        let (path, used) = match tree.classify(&header.source) {
            Location::Tree(path) => {
                let used = headers.contains(&path);
                (path, used)
            }
            Location::Generated(path) => {
                let used = generated.contains(&path);
                (path, used)
            }
            Location::External => {
                return Err(format!(
                    "the configuration installs an unknown header outside the FFmpeg source tree\
                    ; this is unexpected: {}",
                    header.source,
                )
                .into());
            }
        };

        // The include directories are the source tree root and generated files root.
        // If a relative path differs from the include name, `#include` cannot find it.
        // Configurations that install headers from subdirectories trigger this case.
        if path != include_name {
            return Err(format!(
                "public header {include_name} is located at a different path ({path}).\n\
                 Support for storing the `#include` name separately from the actual path is required."
            )
            .into());
        }

        if used {
            public.insert(include_name);
        }
    }

    if public.is_empty() {
        return Err(
            "could not find any public headers included by translation units \
             in make -n install-headers output"
                .into(),
        );
    }

    Ok(public.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(source: &str, library: &str) -> InstalledHeader {
        InstalledHeader {
            source: source.to_owned(),
            library: library.to_owned(),
        }
    }

    fn tree() -> SourceTree {
        SourceTree::new(&["src", "/work/FFmpeg"], &["/work/build"])
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    /// An actual `make -n install-headers` line has an `install` command after the
    /// progress message on the same line. Source tree headers use absolute paths;
    /// generated files use paths relative to the build directory.
    #[test]
    fn parses_header_install_command() {
        let line = "printf \"INSTALL\\t%s\\n\" work/FFmpeg/libavutil/avutil.h \
                    libavutil/avconfig.h; install -m 644 /work/FFmpeg/libavutil/avutil.h \
                    libavutil/avconfig.h \"/usr/local/include/libavutil\"";

        assert_eq!(
            parse_line(line).unwrap(),
            [
                header("/work/FFmpeg/libavutil/avutil.h", "libavutil"),
                header("libavutil/avconfig.h", "libavutil"),
            ],
        );
    }

    /// `install-headers` also installs pkg-config files and prints commands that create
    /// directories. Skip both because neither describes headers.
    #[test]
    fn ignores_other_commands() {
        for line in [
            "mkdir -p \"/usr/local/include/libavutil\"",
            "printf \"INSTALL\\t%s\\n\" libavutil/libavutil.pc; install -m 644 \
             libavutil/libavutil.pc \"/usr/local/lib/pkgconfig\"",
            "install -m 644",
            "echo \"unterminated",
        ] {
            assert_eq!(parse_line(line).unwrap(), [], "{line}");
        }
    }

    /// Mixing headers and other files in one command violates the public-header-only
    /// assumption. Stop rather than silently discarding one group.
    #[test]
    fn rejects_headers_mixed_with_other_files() {
        let line =
            "install -m 644 libavutil/avconfig.h libavutil/libavutil.pc \"/x/include/libavutil\"";

        assert!(parse_line(line).is_err());
    }

    /// Keep only headers included by translation units. Match source tree headers
    /// against `headers` and generated files against `generated`.
    ///
    /// `d3d11va.h` is installed but no translation unit includes it in the current
    /// configuration. Keeping it would prevent translation because it requires Windows
    /// headers.
    #[test]
    fn selects_only_headers_that_units_include() {
        let installed = [
            header("/work/FFmpeg/libavcodec/avcodec.h", "libavcodec"),
            header("/work/FFmpeg/libavcodec/d3d11va.h", "libavcodec"),
            header("src/libavutil/avutil.h", "libavutil"),
            header("libavutil/avconfig.h", "libavutil"),
            header("libavutil/unused_generated.h", "libavutil"),
        ];

        let public = select_public_headers(
            &installed,
            &tree(),
            &strings(&[
                "libavcodec/avcodec.h",
                "libavcodec/internal.h",
                "libavutil/avutil.h",
            ]),
            &strings(&["config.h", "libavutil/avconfig.h"]),
        )
        .unwrap();

        assert_eq!(
            public,
            [
                "libavcodec/avcodec.h",
                "libavutil/avconfig.h",
                "libavutil/avutil.h"
            ],
        );
    }

    /// When installing a header from a subdirectory, `install` copies it directly into
    /// the destination, making its `#include` name differ from its source tree path.
    /// Stop on mismatches because the build manifest only stores the name.
    #[test]
    fn rejects_header_whose_path_differs_from_include_name() {
        let installed = [header("/work/FFmpeg/libavcodec/aac/aac.h", "libavcodec")];
        let headers = strings(&["libavcodec/aac/aac.h"]);

        assert!(select_public_headers(&installed, &tree(), &headers, &[]).is_err());
    }

    #[test]
    fn rejects_header_outside_ffmpeg_tree() {
        let installed = [header("/opt/include/libavutil/avutil.h", "libavutil")];

        assert!(select_public_headers(&installed, &tree(), &[], &[]).is_err());
    }

    /// An empty selection indicates that command parsing assumptions failed.
    /// Do not silently write an empty list.
    #[test]
    fn rejects_empty_selection() {
        let installed = [header("/work/FFmpeg/libavcodec/d3d11va.h", "libavcodec")];

        assert!(select_public_headers(&installed, &tree(), &[], &[]).is_err());
        assert!(select_public_headers(&[], &tree(), &[], &[]).is_err());
    }
}
