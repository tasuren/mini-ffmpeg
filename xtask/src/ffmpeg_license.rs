//! Scan the full contents of FFmpeg inputs listed in the build manifest with licensecheck.
//!
//! Preserve detected values as-is, without supplementing IJG detection or legally
//! interpreting multiple licenses. Debian in Docker is only the scanning environment;
//! it does not change the FFmpeg configuration.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{
    BUILD_CONFIG_DIR, MANIFEST_FILE, Result, available_targets, sys_crate_dir, workspace_root,
};

const IMAGE: &str = "mini-ffmpeg-licensecheck:trixie";
const DOCKERFILE: &str = "FROM debian:trixie-slim\n\
ENV DEBIAN_FRONTEND=noninteractive LC_ALL=C.UTF-8 PERL_UNICODE=SD\n\
RUN apt-get update && apt-get install -y --no-install-recommends licensecheck && rm -rf /var/lib/apt/lists/*\n";
const SCAN_ARGS: &[&str] = &[
    "--encoding",
    "UTF-8",
    "--lines",
    "0",
    "--machine",
    "--copyright",
    "--shortname-scheme",
    "spdx",
];

const HELP: &str =
    "Usage: cargo xtask ffmpeg-license --target TRIPLE [--docker | --native] [--output DIR]

--target is required. Scan the configuration for that target triple under
mini-ffmpeg-sys/FFmpeg_build/. Configurations and results are target-specific.
Requiring a target avoids implying that all inputs potentially used by mini-ffmpeg
have been scanned.

Linux runs licensecheck from PATH directly; other operating systems use Docker.
Use --docker / --native to select the mode explicitly (there is no automatic fallback).
Docker builds and caches a Debian trixie image for scanning.
The first run requires network access to download the image and packages.
Native execution on Linux requires licensecheck (Debian: apt install licensecheck).
Default output: target/xtask/ffmpeg-license/TRIPLE (--output is relative to the working directory).
Output files: report.md, licensecheck.tsv, versions.txt, manifest.toml.
The scan covers all sources in the build manifest, excluding system headers and Cargo dependencies.";

/// Use the local tool by default only on Linux, unless explicitly overridden.
#[derive(Debug)]
struct Options {
    /// The target triple of the configuration to scan.
    target: String,
    docker: bool,
    output: PathBuf,
}

impl Options {
    fn parse(args: &[String]) -> Result<Option<Self>> {
        if args == ["--help"] || args == ["-h"] {
            return Ok(None);
        }

        let mut mode = None;
        let mut target = None;
        let mut output = None;
        let mut args = args.iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--docker" | "--native" => {
                    if mode.is_some() {
                        return Err("specify --docker / --native only once".into());
                    }
                    mode = Some(arg == "--docker");
                }
                "--output" => {
                    output = Some(PathBuf::from(args.next().ok_or("--output requires DIR")?));
                }
                "--target" => {
                    target = Some(args.next().ok_or("--target requires TRIPLE")?.clone());
                }
                _ => return Err(format!("unknown argument: {arg}\n\n{HELP}").into()),
            }
        }

        let target = target.ok_or_else(|| {
            format!(
                "--target is required. Available targets: {}\n\n{HELP}",
                available_targets().map_or_else(|_| "(unavailable)".to_owned(), |t| t.join(", ")),
            )
        })?;

        validate_target(&target)?;

        // Separate output by target. Sharing a location would overwrite another target's
        // results and obscure which target was scanned.
        let output = match output {
            Some(output) => output,
            None => workspace_root()?
                .join("target/xtask/ffmpeg-license")
                .join(&target),
        };

        Ok(Some(Self {
            target,
            docker: mode.unwrap_or(!cfg!(target_os = "linux")),
            output,
        }))
    }
}

/// Check that `--target` is a triple safe to use as a directory name.
///
/// The value is joined directly onto a path, so accepting `..` or separators would
/// allow files outside the configuration to be scanned.
fn validate_target(target: &str) -> Result<()> {
    let plain = !target.is_empty()
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !target.starts_with('.');

    if !plain {
        return Err(format!("invalid target: {target:?}").into());
    }

    Ok(())
}

/// Write results only after confirming scan completion and that the input set matches.
pub fn run(args: &[String]) -> Result<()> {
    let Some(options) = Options::parse(args)? else {
        println!("{HELP}");
        return Ok(());
    };

    let root = sys_crate_dir()?;
    let generated_dir = format!("{BUILD_CONFIG_DIR}/{}", options.target);
    let manifest_path = root.join(&generated_dir).join(MANIFEST_FILE);

    let manifest = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "cannot read {}: {error}\nAvailable targets: {}",
            manifest_path.display(),
            available_targets().map_or_else(|_| "(unavailable)".to_owned(), |t| t.join(", ")),
        )
    })?;
    let inputs = inputs(&manifest, &generated_dir)?;

    for path in inputs.keys() {
        if !root.join(path).is_file() {
            return Err(format!("scan input is missing: {}", root.join(path).display()).into());
        }
    }

    let (versions, scan) = if options.docker {
        docker_scan(&root, &inputs)?
    } else {
        native_scan(&root, &inputs)?
    };
    let rows = validate_scan(&scan, &inputs)?;
    let report = report(&rows, &inputs, &versions, &options);

    fs::create_dir_all(&options.output)?;
    fs::write(options.output.join("manifest.toml"), manifest)?;
    fs::write(options.output.join("versions.txt"), versions)?;
    fs::write(options.output.join("licensecheck.tsv"), scan)?;
    fs::write(options.output.join("report.md"), report)?;

    println!(
        "Scanned {} files: {}",
        inputs.len(),
        options.output.display()
    );
    Ok(())
}

/// Deduplicate crate-relative paths while preserving build manifest categories.
///
/// Generated file paths are based in `generated_dir`, a crate-relative path of the
/// form `FFmpeg_build/<triple>`.
fn inputs(manifest: &str, generated_dir: &str) -> Result<BTreeMap<String, String>> {
    let table: toml::Table = manifest.parse()?;
    let sources = table
        .get("sources")
        .and_then(toml::Value::as_table)
        .ok_or("manifest is missing [sources]")?;
    let mut inputs = BTreeMap::new();

    for category in [
        "translation_units",
        "included_sources",
        "headers",
        "generated",
    ] {
        let paths = sources
            .get(category)
            .and_then(toml::Value::as_array)
            .ok_or_else(|| format!("missing sources.{category} array"))?;
        let prefix = if category == "generated" {
            generated_dir
        } else {
            "FFmpeg"
        };

        for value in paths {
            let path = value.as_str().ok_or("file path is not a string")?;
            // Build manifest paths use OS-independent '/' separators. Reject ambiguous
            // components and control characters so scanned paths cannot escape the mount or
            // create extra TSV lines.
            if path.is_empty()
                || path.contains('\\')
                || path.contains(':')
                || path.chars().any(char::is_control)
                || path.split('/').any(|part| matches!(part, "" | "." | ".."))
            {
                return Err(format!("invalid manifest path: {path:?}").into());
            }
            inputs.insert(format!("{prefix}/{path}"), category.to_owned());
        }
    }

    if inputs.is_empty() {
        return Err("manifest has no scan inputs".into());
    }
    Ok(inputs)
}

/// Do not treat partial stdout from a failed command as a successful scan result.
fn capture(command: &mut Command, input: Option<&[u8]>) -> Result<String> {
    command.stdout(Stdio::piped()).stderr(Stdio::inherit());
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot run {command:?}: {error}"))?;
    // Read stdout while writing stdin so large input lists and scan results can flow
    // simultaneously, avoiding deadlocks caused by pipe capacity limits.
    let output = std::thread::scope(|scope| -> Result<_> {
        let writer = input.map(|input| {
            let mut stdin = child.stdin.take().expect("piped stdin");
            scope.spawn(move || stdin.write_all(input))
        });
        let output = child.wait_with_output()?;
        if let Some(writer) = writer {
            writer.join().map_err(|_| "stdin writer thread failed")??;
        }
        Ok(output)
    })?;

    if !output.status.success() {
        return Err(format!("{command:?} failed: {}", output.status).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn native_command(root: &Path) -> Command {
    let mut command = Command::new("licensecheck");
    command
        .current_dir(root)
        .env("LC_ALL", "C.UTF-8")
        .env("PERL_UNICODE", "SD");
    command
}

fn native_scan(root: &Path, inputs: &BTreeMap<String, String>) -> Result<(String, String)> {
    let versions = capture(native_command(root).arg("--version"), None)?;
    let paths: Vec<_> = inputs.keys().collect();
    let mut scan = String::new();

    // Split inputs to also fit Windows command-line length limits. Pass filenames as
    // separate arguments without a shell, so spaces do not change the scanned files.
    for chunk in paths.chunks(16) {
        scan.push_str(&capture(
            native_command(root).args(SCAN_ARGS).arg("--").args(chunk),
            None,
        )?);
    }
    Ok((versions, scan))
}

fn docker_command(root: &Path) -> Command {
    let mut command = Command::new("docker");
    command
        .args(["run", "--rm", "-i", "--mount"])
        .arg(format!(
            "type=bind,source={},target=/work,readonly",
            root.display()
        ))
        .args(["--workdir", "/work", IMAGE]);
    command
}

fn docker_scan(root: &Path, inputs: &BTreeMap<String, String>) -> Result<(String, String)> {
    eprintln!("Preparing the Debian licensecheck image (using the Docker cache)");
    capture(
        Command::new("docker").args(["build", "--tag", IMAGE, "-"]),
        Some(DOCKERFILE.as_bytes()),
    )?;
    let versions = capture(docker_command(root).args(["sh", "-c",
        "cat /etc/os-release; dpkg-query -W licensecheck libregexp-pattern-license-perl; licensecheck --version"]), None)?;
    let list: Vec<u8> = inputs
        .keys()
        .flat_map(|path| path.bytes().chain([0]))
        .collect();
    // Pass NUL-separated filenames. xargs limits argv length and reports failures through its exit code.
    let scan = capture(
        docker_command(root)
            .args(["xargs", "-0", "-r", "-n", "16", "licensecheck"])
            .args(SCAN_ARGS)
            .arg("--"),
        Some(&list),
    )?;
    Ok((versions, scan))
}

/// Verify that the licensecheck TSV contains exactly the input set.
fn validate_scan<'a>(
    scan: &'a str,
    inputs: &BTreeMap<String, String>,
) -> Result<BTreeMap<&'a str, (&'a str, &'a str)>> {
    let mut rows = BTreeMap::new();

    for line in scan.lines() {
        let fields: Vec<_> = line.splitn(3, '\t').collect();
        if fields.len() != 3 || fields[1].is_empty() || !inputs.contains_key(fields[0]) {
            return Err(format!("unexpected licensecheck output: {line:?}").into());
        }
        if rows.insert(fields[0], (fields[1], fields[2])).is_some() {
            return Err(format!("duplicate scan result: {}", fields[0]).into());
        }
    }

    if rows.len() != inputs.len() {
        let missing: BTreeSet<_> = inputs
            .keys()
            .filter(|p| !rows.contains_key(p.as_str()))
            .collect();
        return Err(format!("scan results are missing inputs: {missing:?}").into());
    }
    Ok(rows)
}

fn report(
    rows: &BTreeMap<&str, (&str, &str)>,
    inputs: &BTreeMap<String, String>,
    versions: &str,
    options: &Options,
) -> String {
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (path, (license, _)) in rows {
        groups.entry(license).or_default().push(path);
    }
    let mut out = format!(
        "# FFmpeg licensecheck report\n\nTarget: {}. Mode: {}. Inputs: {} files.\n\nThe full contents of the build manifest's sources (translation units, include-only sources, headers, and generated files) are scanned.\nSystem headers, cinrs / Cargo dependencies, and files outside the build manifest are excluded.\nPreprocessor branches and reachability in the final binary are not evaluated.\n\nUNKNOWN does not mean that no license applies. IJG notices may also be reported as UNKNOWN.\nand/or reflects the detection tool's output; it does not establish a choice of licenses or a definitive SPDX expression.\n\nScan options: `{}`\n\n## Tool versions\n\n```text\n{}\n```\n\n## Summary\n\n| Detected value | File count |\n| --- | ---: |\n",
        options.target,
        if options.docker {
            "Docker / Debian trixie"
        } else {
            "native"
        },
        inputs.len(),
        SCAN_ARGS.join(" "),
        versions.trim()
    );
    for (license, paths) in &groups {
        out.push_str(&format!(
            "| {} | {} |\n",
            license.replace('|', "\\|"),
            paths.len()
        ));
    }
    out.push_str("\n## Files\n");
    for (license, paths) in groups {
        out.push_str(&format!("\n### {license}\n\n"));
        for path in paths {
            out.push_str(&format!("- `{path}` ({})\n", inputs[path]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATED: &str = "FFmpeg_build/aarch64-apple-darwin";

    fn manifest(path: &str) -> String {
        format!(
            "[sources]\ntranslation_units = [{path:?}]\nincluded_sources = []\nheaders = []\ngenerated = [\"config.h\"]\n"
        )
    }

    /// Build manifests use the same format on every OS. Also reject Windows absolute
    /// paths and parent references.
    #[test]
    fn rejects_paths_outside_scan_roots() {
        for path in ["../foo.c", "/foo.c", "C:/foo.c", "a\\b.c", "a\nb.c"] {
            assert!(inputs(&manifest(path), GENERATED).is_err(), "{path:?}");
        }
        let inputs = inputs(&manifest("dir/a b.c"), GENERATED).unwrap();
        assert_eq!(inputs["FFmpeg/dir/a b.c"], "translation_units");
        assert_eq!(
            inputs["FFmpeg_build/aarch64-apple-darwin/config.h"],
            "generated"
        );
    }

    /// `--target` is joined onto a path. Reject values pointing outside the configuration.
    #[test]
    fn rejects_unsafe_targets() {
        for target in ["", ".", "..", "../x", "a/b", "a\\b", ".hidden", "a b"] {
            assert!(validate_target(target).is_err(), "{target:?}");
        }

        assert!(validate_target("aarch64-apple-darwin").is_ok());
    }

    /// Do not silently accept missing or extra inputs even when the command exits successfully.
    #[test]
    fn rejects_incomplete_duplicate_or_unexpected_results() {
        let inputs = inputs(&manifest("a.c"), GENERATED).unwrap();
        let scan = "FFmpeg/a.c\tUNKNOWN\t*No copyright*\nFFmpeg_build/aarch64-apple-darwin/config.h\tUNKNOWN\t*No copyright*\n";
        assert_eq!(validate_scan(scan, &inputs).unwrap().len(), 2);
        assert!(validate_scan(scan.lines().next().unwrap(), &inputs).is_err());
        assert!(validate_scan(&format!("{scan}{scan}"), &inputs).is_err());
        assert!(validate_scan(&scan.replace("FFmpeg/a.c", "other.c"), &inputs).is_err());
    }
}
