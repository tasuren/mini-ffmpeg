//! ビルドマニフェストが列挙する FFmpeg 入力を licensecheck で全文検査する。
//!
//! 自動検出値をそのまま保存し、IJG の補完や複数ライセンスの法的な解釈は行わない。
//! Docker の Debian は検査環境にだけ使い、FFmpeg の構成は変更しない。

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{MANIFEST_FILE, Result, sys_crate_dir, workspace_root};

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

const HELP: &str = "使い方: cargo xtask ffmpeg-license [--docker | --native] [--output DIR]

Linux は PATH の licensecheck を直接実行し、それ以外の OS は Docker を使う。
--docker / --native で実行方法を指定できる（自動フォールバックはしない）。
Docker は Debian trixie の検査用イメージをビルドし、キャッシュして使う。
初回はイメージとパッケージの取得にネットワークが必要。
Linux の直接実行には licensecheck のインストールが必要（Debian: apt install licensecheck）。
既定の出力先: target/xtask/ffmpeg-license（--output は作業ディレクトリ基準）。
出力: report.md、licensecheck.tsv、versions.txt、manifest.toml。
検査対象はビルドマニフェストの sources 全体。システムヘッダと Cargo 依存は含まない。";

/// 明示指定が無ければ、Linux だけローカルのツールを使う。
#[derive(Debug)]
struct Options {
    docker: bool,
    output: PathBuf,
}

impl Options {
    fn parse(args: &[String]) -> Result<Option<Self>> {
        if args == ["--help"] || args == ["-h"] {
            return Ok(None);
        }

        let mut mode = None;
        let mut output = workspace_root()?.join("target/xtask/ffmpeg-license");
        let mut args = args.iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--docker" | "--native" => {
                    if mode.is_some() {
                        return Err("--docker / --native は一度だけ指定してください".into());
                    }
                    mode = Some(arg == "--docker");
                }
                "--output" => {
                    output = PathBuf::from(args.next().ok_or("--output に DIR が必要です")?);
                }
                _ => return Err(format!("未知の引数です: {arg}\n\n{HELP}").into()),
            }
        }

        Ok(Some(Self {
            docker: mode.unwrap_or(!cfg!(target_os = "linux")),
            output,
        }))
    }
}

/// 検査完了と入力集合の一致を確認してから、結果を出力する。
pub fn run(args: &[String]) -> Result<()> {
    let Some(options) = Options::parse(args)? else {
        println!("{HELP}");
        return Ok(());
    };

    let root = sys_crate_dir()?;
    let manifest = fs::read_to_string(root.join(MANIFEST_FILE))?;
    let inputs = inputs(&manifest)?;

    for path in inputs.keys() {
        if !root.join(path).is_file() {
            return Err(format!("検査対象がありません: {}", root.join(path).display()).into());
        }
    }

    let (versions, scan) = if options.docker {
        docker_scan(&root, &inputs)?
    } else {
        native_scan(&root, &inputs)?
    };
    let rows = validate_scan(&scan, &inputs)?;
    let report = report(&rows, &inputs, &versions, options.docker);

    fs::create_dir_all(&options.output)?;
    fs::write(options.output.join("manifest.toml"), manifest)?;
    fs::write(options.output.join("versions.txt"), versions)?;
    fs::write(options.output.join("licensecheck.tsv"), scan)?;
    fs::write(options.output.join("report.md"), report)?;

    println!(
        "{} ファイルを検査しました: {}",
        inputs.len(),
        options.output.display()
    );
    Ok(())
}

/// ビルドマニフェストの分類を保持しつつ、クレート基準のパスで重複を除く。
fn inputs(manifest: &str) -> Result<BTreeMap<String, String>> {
    let table: toml::Table = manifest.parse()?;
    let sources = table
        .get("sources")
        .and_then(toml::Value::as_table)
        .ok_or("manifest に [sources] がありません")?;
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
            .ok_or_else(|| format!("sources.{category} の配列がありません"))?;
        let prefix = if category == "generated" {
            "FFmpeg_generated"
        } else {
            "FFmpeg"
        };

        for value in paths {
            let path = value
                .as_str()
                .ok_or("ファイルのパスが文字列ではありません")?;
            // ビルドマニフェストのパスは OS に依存しない '/' 区切り。検査対象がマウント外や
            // TSV の別行に逃げないよう、曖昧な要素と制御文字を受け付けない。
            if path.is_empty()
                || path.contains('\\')
                || path.contains(':')
                || path.chars().any(char::is_control)
                || path.split('/').any(|part| matches!(part, "" | "." | ".."))
            {
                return Err(format!("不正な manifest パスです: {path:?}").into());
            }
            inputs.insert(format!("{prefix}/{path}"), category.to_owned());
        }
    }

    if inputs.is_empty() {
        return Err("manifest の検査対象が空です".into());
    }
    Ok(inputs)
}

/// 失敗時に部分的な標準出力を成功した検査結果として扱わない。
fn capture(command: &mut Command, input: Option<&[u8]>) -> Result<String> {
    command.stdout(Stdio::piped()).stderr(Stdio::inherit());
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .spawn()
        .map_err(|error| format!("{command:?} を実行できません: {error}"))?;
    // 大きい入力一覧と検査結果を同時に流せるよう、stdin の書き込み中にも
    // stdout を読み取る。パイプの容量に依存したデッドロックを避ける。
    let output = std::thread::scope(|scope| -> Result<_> {
        let writer = input.map(|input| {
            let mut stdin = child.stdin.take().expect("piped stdin");
            scope.spawn(move || stdin.write_all(input))
        });
        let output = child.wait_with_output()?;
        if let Some(writer) = writer {
            writer
                .join()
                .map_err(|_| "stdin 書き込みスレッドが失敗しました")??;
        }
        Ok(output)
    })?;

    if !output.status.success() {
        return Err(format!("{command:?} が失敗しました: {}", output.status).into());
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

    // Windows のコマンドライン長にも収まるように分割する。ファイル名は shell を
    // 経由せず別々の引数として渡すため、空白があっても検査対象は変わらない。
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
    eprintln!("Debian の licensecheck イメージを準備します（Docker のキャッシュを使用）");
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
    // NUL 区切りでファイル名を渡す。xargs は argv 長を制限し、失敗を終了コードで伝える。
    let scan = capture(
        docker_command(root)
            .args(["xargs", "-0", "-r", "-n", "16", "licensecheck"])
            .args(SCAN_ARGS)
            .arg("--"),
        Some(&list),
    )?;
    Ok((versions, scan))
}

/// licensecheck の TSV が入力集合を過不足なく含むことを検証する。
fn validate_scan<'a>(
    scan: &'a str,
    inputs: &BTreeMap<String, String>,
) -> Result<BTreeMap<&'a str, (&'a str, &'a str)>> {
    let mut rows = BTreeMap::new();

    for line in scan.lines() {
        let fields: Vec<_> = line.splitn(3, '\t').collect();
        if fields.len() != 3 || fields[1].is_empty() || !inputs.contains_key(fields[0]) {
            return Err(format!("予期しない licensecheck 出力です: {line:?}").into());
        }
        if rows.insert(fields[0], (fields[1], fields[2])).is_some() {
            return Err(format!("検査結果が重複しています: {}", fields[0]).into());
        }
    }

    if rows.len() != inputs.len() {
        let missing: BTreeSet<_> = inputs
            .keys()
            .filter(|p| !rows.contains_key(p.as_str()))
            .collect();
        return Err(format!("検査結果に不足があります: {missing:?}").into());
    }
    Ok(rows)
}

fn report(
    rows: &BTreeMap<&str, (&str, &str)>,
    inputs: &BTreeMap<String, String>,
    versions: &str,
    docker: bool,
) -> String {
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (path, (license, _)) in rows {
        groups.entry(license).or_default().push(path);
    }
    let mut out = format!(
        "# FFmpeg licensecheck report\n\n実行方法: {}。対象: {} ファイル。\n\nビルドマニフェストの sources（翻訳単位・include 専用ソース・ヘッダ・生成物）を全文検査。\nシステムヘッダ、cinrs / Cargo 依存、ビルドマニフェスト外のファイルは対象外。\nプリプロセッサの分岐や最終バイナリへの到達性は判定しない。\n\nUNKNOWN はライセンス不適用を意味しない。IJG の表示も UNKNOWN になる場合がある。\nand/or は検出ツールの出力であり、選択可能なライセンスや確定した SPDX 式を意味しない。\n\n検査オプション: `{}`\n\n## ツールのバージョン\n\n```text\n{}\n```\n\n## 集計\n\n| 検出値 | ファイル数 |\n| --- | ---: |\n",
        if docker {
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
    out.push_str("\n## ファイル一覧\n");
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

    fn manifest(path: &str) -> String {
        format!(
            "[sources]\ntranslation_units = [{path:?}]\nincluded_sources = []\nheaders = []\ngenerated = [\"config.h\"]\n"
        )
    }

    /// ビルドマニフェストは全 OS で同じ形式。Windows の絶対パスや親参照も拒否する。
    #[test]
    fn rejects_paths_outside_scan_roots() {
        for path in ["../foo.c", "/foo.c", "C:/foo.c", "a\\b.c", "a\nb.c"] {
            assert!(inputs(&manifest(path)).is_err(), "{path:?}");
        }
        let inputs = inputs(&manifest("dir/a b.c")).unwrap();
        assert_eq!(inputs["FFmpeg/dir/a b.c"], "translation_units");
        assert_eq!(inputs["FFmpeg_generated/config.h"], "generated");
    }

    /// 終了コードが成功でも、検査漏れや余分な対象を黙って受け入れない。
    #[test]
    fn rejects_incomplete_duplicate_or_unexpected_results() {
        let inputs = inputs(&manifest("a.c")).unwrap();
        let scan = "FFmpeg/a.c\tUNKNOWN\t*No copyright*\nFFmpeg_generated/config.h\tUNKNOWN\t*No copyright*\n";
        assert_eq!(validate_scan(scan, &inputs).unwrap().len(), 2);
        assert!(validate_scan(scan.lines().next().unwrap(), &inputs).is_err());
        assert!(validate_scan(&format!("{scan}{scan}"), &inputs).is_err());
        assert!(validate_scan(&scan.replace("FFmpeg/a.c", "other.c"), &inputs).is_err());
    }
}
