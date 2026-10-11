//! 一覧（`cinrs_manifest.toml`）の翻訳単位を、cinrs で Rust のコードに変換するタスク。
//!
//! FFmpeg の `.c` 1 つを、Rust のファイル 1 つに変換する。変換はこのタスクが
//! 行い、`mini-ffmpeg-sys` のビルドは変換後のコードを読むだけになる。そのため、
//! `mini-ffmpeg-sys` のビルドには cinrs も、FFmpeg のソースも、プラットフォームの
//! ヘッダも要らない。
//!
//! 生成物は、今はリポジトリに入れていない。変換後のコードには、プラットフォームの
//! ヘッダの宣言やインライン関数も含まれ、再配布するときのライセンスの扱いが
//! 決まっていないため。`mini-ffmpeg-sys` をビルドする前に、このタスクを実行する。
//!
//! 変換は、実行したマシンのプラットフォームのヘッダを読んで行う。結果はその
//! ターゲット専用になるので、生成物は `mini-ffmpeg-sys/targets/<ターゲット>/` に
//! ターゲットごとに分けて置く。
//!
//! - `rust/<ライブラリ>/<名前>.rs`: 翻訳単位 1 つを変換したもの。
//! - `rust/<ライブラリ>.rs`: そのライブラリの翻訳単位のモジュールの宣言。
//! - `rust/mod.rs`: ライブラリのモジュールの宣言。`src/lib.rs` がこれを読む。
//! - `generator.toml`: 生成に使った cinrs のコミット。
//!
//! このタスクは C コンパイラを使わない。FFmpeg のツリーと、プラットフォームの
//! ヘッダ（macOS なら Command Line Tools の SDK）は要る。変換の指定を変えたときは、
//! このタスクだけを実行し直せば良い。
//!
//! # 生成物の管理
//!
//! `rust/` は、このタスクが丸ごと管理する。生成したファイルには、先頭に
//! [`GENERATED_MARKER`] を付ける。一覧から消えた翻訳単位のファイルは削除するが、
//! 削除するのはこの印があるファイルだけ。印が無いファイルが `rust/` にあれば、
//! 手で置いたものとみなし、何も書き出さずにエラーにして止まる。

mod translate;

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    num::NonZero,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use self::translate::{Settings, Translated};
use crate::{FFMPEG_DIR, HOST_TARGET, MANIFEST_FILE, Result, sys_crate_dir};

/// ターゲットごとの生成物を置くディレクトリ。`mini-ffmpeg-sys` からの相対パス。
const TARGETS_DIR: &str = "targets";

/// 変換した Rust のコードを置くディレクトリ。ターゲットのディレクトリからの相対パス。
const RUST_DIR: &str = "rust";

/// 生成に使った cinrs を記録するファイル。ターゲットのディレクトリからの相対パス。
const GENERATOR_FILE: &str = "generator.toml";

/// 生成したファイルの 1 行目に付ける印。
///
/// `@generated` は、生成されたファイルであることを示す慣習的な表記で、
/// エディタやレビューのツールにも認識されるものがある。
const GENERATED_MARKER: &str = "// @generated";

/// xtask 自身の `Cargo.toml`。生成に使う cinrs のコミットを、ここから読む。
///
/// コンパイル時に埋め込むので、実行中の xtask がリンクしている cinrs と必ず
/// 一致する。`Cargo.toml` を変えると、Cargo が xtask をビルドし直す。
const XTASK_MANIFEST: &str = include_str!("../Cargo.toml");

/// 変換と整形を行うスレッドのスタックの大きさ。
///
/// 変換後のコードの構文解析と整形は、式の入れ子の深さだけ再帰する。C のマクロが
/// 展開された式は深くなりやすく、スレッドの既定の大きさ（2 MiB）に収まる保証が
/// 無いので、余裕を持たせている。確保されるのは仮想メモリで、使わなかった分は
/// 実メモリを消費しない。
const WORKER_STACK_SIZE: usize = 64 << 20;

/// 一覧のうち、変換に使う部分。
#[derive(Debug)]
struct Manifest {
    /// C の規格。`-std=` の値と同じ書き方。
    c_standard: String,

    /// マクロ定義。`NAME` または `NAME=VALUE` の形。
    defines: Vec<String>,

    /// インクルードディレクトリ。`mini-ffmpeg-sys` のルートからの相対パス。
    include_dirs: Vec<String>,

    /// 翻訳単位。FFmpeg のツリーからの相対パス。
    translation_units: Vec<String>,
}

impl Manifest {
    fn parse(text: &str) -> Result<Self> {
        let table: toml::Table = text
            .parse()
            .map_err(|error| format!("{MANIFEST_FILE} を読めません: {error}"))?;

        let c_standard = table
            .get("compile")
            .and_then(|compile| compile.get("std"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("{MANIFEST_FILE} に compile.std がありません"))?
            .to_owned();

        Ok(Self {
            c_standard,
            defines: string_array(&table, "compile", "defines")?,
            include_dirs: string_array(&table, "compile", "include_dirs")?,
            translation_units: string_array(&table, "sources", "translation_units")?,
        })
    }
}

fn string_array(table: &toml::Table, section: &str, key: &str) -> Result<Vec<String>> {
    let missing = || format!("{MANIFEST_FILE} に {section}.{key} の文字列の配列がありません");

    table
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_array)
        .ok_or_else(missing)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| missing().into())
        })
        .collect()
}

/// 翻訳単位 1 つ。`libavutil/log.c` なら、ライブラリが `libavutil`、名前が `log`。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Unit {
    library: String,
    name: String,
}

impl Unit {
    /// FFmpeg のツリーからの相対パスを、ライブラリと名前に分ける。
    ///
    /// ライブラリと名前は、そのまま Rust のモジュールの名前にする。そのままでは
    /// モジュールの名前にできないパスはエラーにする。今の構成には該当するものが
    /// 無いので、名前の変換の規則は、必要になった時点で決める。
    ///
    /// - サブディレクトリにある翻訳単位（`libavcodec/aac/aacdec.c` など）。
    ///   モジュールを入れ子にする必要があり、同じ名前の翻訳単位
    ///   （`libavcodec/aac.c`）があると衝突する。
    /// - Rust の識別子として書けない名前や、予約語と同じ名前。
    fn parse(path: &str) -> Result<Self> {
        let unsupported = |reason: &str| -> Box<dyn std::error::Error> {
            format!("翻訳単位 {path} をモジュールにできません: {reason}").into()
        };

        let (library, file) = path
            .split_once('/')
            .ok_or_else(|| unsupported("ライブラリのディレクトリの下にありません"))?;

        if file.contains('/') {
            return Err(unsupported("サブディレクトリの翻訳単位には未対応です"));
        }

        let name = file
            .strip_suffix(".c")
            .ok_or_else(|| unsupported("拡張子が .c ではありません"))?;

        for identifier in [library, name] {
            if !is_plain_identifier(identifier) {
                return Err(unsupported("Rust の識別子として書けない名前です"));
            }

            if RUST_KEYWORDS.contains(&identifier) {
                return Err(unsupported("Rust の予約語と同じ名前です"));
            }
        }

        Ok(Self {
            library: library.to_owned(),
            name: name.to_owned(),
        })
    }

    /// FFmpeg のツリーからの相対パス。
    fn source_path(&self) -> String {
        format!("{}/{}.c", self.library, self.name)
    }

    /// 変換したファイル。`rust/` からの相対パス。
    fn module_file(&self) -> PathBuf {
        Path::new(&self.library).join(format!("{}.rs", self.name))
    }
}

fn is_plain_identifier(text: &str) -> bool {
    let mut characters = text.chars();

    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && characters.all(|rest| rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '_')
}

/// Rust の予約語。小文字だけの名前を対象にしているので、`Self` は含めていない。
const RUST_KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// すべての翻訳単位を変換する。結果は `units` と同じ順。
///
/// 翻訳単位は互いに独立なので、CPU のコアの数だけ並列に変換する。エラーに
/// なった翻訳単位があれば、すべてをまとめて報告する。
fn translate_units(settings: &Settings, units: &[Unit]) -> Result<Vec<Translated>> {
    let next_index = AtomicUsize::new(0);
    let worker_count = thread::available_parallelism().map_or(1, NonZero::get);

    let work = || {
        let mut finished = Vec::new();

        // 翻訳単位ごとに掛かる時間が大きく違うので、先に均等に分けず、
        // 手の空いたスレッドが次の 1 つを取っていく。
        while let Some(unit) = units.get(next_index.fetch_add(1, Ordering::Relaxed)) {
            let source = Path::new(FFMPEG_DIR).join(unit.source_path());
            finished.push((unit, settings.translate(&source)));
        }

        finished
    };

    let finished = thread::scope(|scope| -> Result<Vec<_>> {
        let workers = (0..worker_count)
            .map(|_| {
                thread::Builder::new()
                    .stack_size(WORKER_STACK_SIZE)
                    .spawn_scoped(scope, work)
            })
            .collect::<std::io::Result<Vec<_>>>()?;

        let mut finished = Vec::new();

        for worker in workers {
            finished.extend(
                worker
                    .join()
                    .map_err(|_| "変換のスレッドが異常終了しました")?,
            );
        }

        Ok(finished)
    })?;

    let mut translated = BTreeMap::new();
    let mut failures = BTreeMap::new();

    for (unit, result) in finished {
        match result {
            Ok(output) => {
                translated.insert(unit, output);
            }
            Err(error) => {
                failures.insert(unit, error);
            }
        }
    }

    if !failures.is_empty() {
        let mut message = format!("{} 個の翻訳単位を変換できません。", failures.len());

        for (unit, error) in &failures {
            let _ = write!(message, "\n\n{}:\n{error}", unit.source_path());
        }

        return Err(message.into());
    }

    Ok(units
        .iter()
        .map(|unit| translated.remove(&unit).expect("全翻訳単位の結果がある"))
        .collect())
}

/// 同じ C のシンボルを、2 つ以上の翻訳単位が定義していないことを確かめる。
///
/// 翻訳単位は Rust のモジュールとしては別々なので、名前が重なっても rustc は
/// エラーにしない。重複が分かるのは、最終的な実行ファイルのリンクのときで、
/// `mini-ffmpeg-sys` のビルドだけでは気づけない。
///
/// C のビルドでは、静的ライブラリの中で同じシンボルが重複することは無い。
/// 重複があれば、cinrs が本来シンボルにしない定義（インライン関数など）を
/// シンボルとして出力している可能性が高い。
fn check_duplicate_symbols(units: &[Unit], translated: &[Translated]) -> Result<()> {
    let mut owners: BTreeMap<&str, &Unit> = BTreeMap::new();

    for (unit, output) in units.iter().zip(translated) {
        for symbol in &output.symbols {
            if let Some(first) = owners.insert(symbol, unit) {
                return Err(format!(
                    "シンボル {symbol} を、{} と {} の両方が定義しています。",
                    first.source_path(),
                    unit.source_path(),
                )
                .into());
            }
        }
    }

    Ok(())
}

/// 翻訳単位 1 つ分のファイルを組み立てる。cinrs が出力したコードに、説明を付ける。
fn render_unit(unit: &Unit, target: &str, rust: &str) -> String {
    let source = unit.source_path();

    format!(
        "{GENERATED_MARKER} by `cargo xtask ffmpeg-units`. 手で編集しない。\n\
         // 変換の指定を変えるときは、xtask の `ffmpeg_units` を変更して再生成する。\n\
         \n\
         //! FFmpeg の `{source}` を、cinrs で {target} 向けの Rust に変換したもの。\n\
         //!\n\
         //! 元のファイルの著作権とライセンスの表示は、FFmpeg のツリーの同じパスにある。\n\
         \n\
         {rust}"
    )
}

/// ライブラリ 1 つ分の、モジュールの宣言のファイルを組み立てる。
fn render_library(library: &str, units: &[&Unit]) -> String {
    let mut out = String::new();

    let _ = writeln!(
        out,
        "{GENERATED_MARKER} by `cargo xtask ffmpeg-units`. 手で編集しない。\n\
         \n\
         //! {library} の翻訳単位。FFmpeg の `.c` 1 つにつき、モジュールを 1 つ置く。\n",
    );

    for unit in units {
        let _ = writeln!(out, "pub mod {};", unit.name);
    }

    out
}

/// ターゲット 1 つ分の、ライブラリのモジュールの宣言のファイルを組み立てる。
fn render_root<'a>(target: &str, libraries: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();

    let _ = writeln!(
        out,
        "{GENERATED_MARKER} by `cargo xtask ffmpeg-units`. 手で編集しない。\n\
         \n\
         //! {target} 向けに変換した FFmpeg のライブラリ。\n",
    );

    for library in libraries {
        let _ = writeln!(out, "pub mod {library};");
    }

    out
}

/// 生成に使った cinrs のリビジョン情報。xtask の `Cargo.toml` にある依存の指定。
#[derive(Debug, PartialEq, Eq)]
struct CinrsSource {
    git: String,
    rev: String,
}

impl CinrsSource {
    /// xtask の `Cargo.toml` から、`cinrs-core` の依存の指定を読む。
    ///
    /// Git のコミットで固定していない指定はエラーにする。ブランチやバージョンの
    /// 範囲では、生成物を作った cinrs を後から特定できない。
    fn parse(xtask_manifest: &str) -> Result<Self> {
        let table: toml::Table = xtask_manifest
            .parse()
            .map_err(|error| format!("xtask の Cargo.toml を読めません: {error}"))?;

        let dependency = table
            .get("dependencies")
            .and_then(|dependencies| dependencies.get("cinrs-core"));

        let field = |key: &str| -> Result<String> {
            dependency
                .and_then(|dependency| dependency.get(key))
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    format!(
                        "xtask の Cargo.toml の cinrs-core に {key} がありません。\n\
                         生成物を作った cinrs を記録するために、Git のコミットで固定してください。"
                    )
                    .into()
                })
        };

        Ok(Self {
            git: field("git")?,
            rev: field("rev")?,
        })
    }
}

/// 生成に使った cinrs を記録するファイルを組み立てる。
fn render_generator(target: &str, cinrs: &CinrsSource) -> String {
    format!(
        "# `cargo xtask ffmpeg-units` が生成したファイル。手で編集しない。\n\
         #\n\
         # このディレクトリの `{RUST_DIR}/` を、どの cinrs で生成したかの記録。\n\
         # 生成物は cinrs のバージョンで変わる。xtask の `Cargo.toml` が指す cinrs と\n\
         # ここの記録が違っていれば、生成物は古い cinrs で作られたもので、再生成が要る。\n\
         \n\
         target = \"{target}\"\n\
         \n\
         [cinrs]\n\
         git = \"{}\"\n\
         rev = \"{}\"\n",
        cinrs.git, cinrs.rev,
    )
}

/// `rust/` に書き出す内容。パスは `rust/` からの相対パス。
#[derive(Debug, Default)]
struct Layout {
    files: BTreeMap<PathBuf, String>,
}

impl Layout {
    /// 変換の結果から、書き出すファイルを組み立てる。`rust` は `units` と同じ順の、
    /// 翻訳単位ごとの変換後のコード。
    fn build(target: &str, units: &[Unit], rust: &[&str]) -> Self {
        let mut layout = Self::default();
        let mut by_library: BTreeMap<&str, Vec<&Unit>> = BTreeMap::new();

        for (unit, rust) in units.iter().zip(rust) {
            by_library.entry(&unit.library).or_default().push(unit);

            layout
                .files
                .insert(unit.module_file(), render_unit(unit, target, rust));
        }

        layout.files.insert(
            PathBuf::from("mod.rs"),
            render_root(target, by_library.keys().copied()),
        );

        for (library, mut units) in by_library {
            // 宣言の並びを、一覧の並びではなく名前の順にする。フォーマッタの
            // 並び順と同じで、一覧の側の順序が変わっても差分が出ない。
            units.sort();

            layout.files.insert(
                PathBuf::from(format!("{library}.rs")),
                render_library(library, &units),
            );
        }

        layout
    }

    /// すでにあるファイルと突き合わせ、削除するファイルを決める。
    ///
    /// `existing` は、`rust/` にあるファイルと、[`GENERATED_MARKER`] の有無。
    /// 印の無いファイルがあれば、何も書き出す前にエラーにする。
    fn plan(&self, existing: &BTreeMap<PathBuf, bool>) -> Result<Vec<PathBuf>> {
        let mut stale = Vec::new();

        for (path, &is_generated) in existing {
            if !is_generated {
                return Err(format!(
                    "{RUST_DIR}/{} に、生成物の印がありません。\n\
                     このディレクトリは cargo xtask ffmpeg-units が丸ごと管理します。\
                     手で置いたファイルであれば、別の場所に移してください。",
                    path.display(),
                )
                .into());
            }

            // 一覧から消えた翻訳単位の生成物。残すと、宣言されないファイルが
            // 溜まっていく。
            if !self.files.contains_key(path) {
                stale.push(path.clone());
            }
        }

        Ok(stale)
    }
}

pub fn run() -> Result<()> {
    let sys_crate = sys_crate_dir()?;

    // 以降のパスは、すべて `mini-ffmpeg-sys` からの相対パスで扱う。cinrs は、
    // 渡されたパスをそのまま `__FILE__` の文字列にする。FFmpeg は `av_assert0`
    // などで `__FILE__` を使うので、絶対パスで渡すと、生成したマシンのパスが
    // 生成物に入り、マシンごとに内容が変わってしまう。
    env::set_current_dir(&sys_crate)
        .map_err(|error| format!("{} に移動できません: {error}", sys_crate.display()))?;

    let manifest_text = fs::read_to_string(MANIFEST_FILE).map_err(|error| {
        format!(
            "{MANIFEST_FILE} を読めません: {error}\n\
             先に cargo xtask ffmpeg-manifest を実行してください。",
        )
    })?;

    let manifest = Manifest::parse(&manifest_text)?;

    // 翻訳単位を名前の順に並べ、重複を除く。
    // `translate_units` は、翻訳単位が重複していないことを前提にしている。
    // 順序を揃えるのは、警告やエラーの表示の順を、一覧の側の順序に左右されない
    // ようにするため。生成物の内容は、`Layout` の側で順序に依存しない。
    let units: Vec<Unit> = manifest
        .translation_units
        .iter()
        .map(|path| Unit::parse(path))
        .collect::<Result<BTreeSet<_>>>()?
        .into_iter()
        .collect();

    require_ffmpeg_tree()?;

    // 変換はこのマシンのプラットフォームのヘッダを読むので、生成物はこのマシンの
    // ターゲット向けになる。
    let target = HOST_TARGET;
    let target_dir = Path::new(TARGETS_DIR).join(target);
    let rust_dir = target_dir.join(RUST_DIR);

    let cinrs = CinrsSource::parse(XTASK_MANIFEST)?;

    check_root_declaration(target)?;

    let settings = Settings {
        c_standard: manifest.c_standard,
        target: target.to_owned(),
        defines: manifest.defines,
        include_dirs: manifest.include_dirs.iter().map(PathBuf::from).collect(),
    };

    let translated = translate_units(&settings, &units)?;

    check_duplicate_symbols(&units, &translated)?;

    for (unit, output) in units.iter().zip(&translated) {
        for warning in &output.warnings {
            eprintln!("warning: {}: {warning}", unit.source_path());
        }
    }

    let rust: Vec<&str> = translated
        .iter()
        .map(|output| output.rust.as_str())
        .collect();
    let layout = Layout::build(target, &units, &rust);

    let mut existing = BTreeMap::new();

    if rust_dir.is_dir() {
        scan_files(&rust_dir, &rust_dir, &mut existing)?;
    }

    let stale = layout.plan(&existing)?;

    let mut written = 0;

    for (path, content) in &layout.files {
        if write_if_changed(&rust_dir.join(path), content)? {
            written += 1;
        }
    }

    for path in &stale {
        fs::remove_file(rust_dir.join(path))?;
    }

    write_if_changed(
        &target_dir.join(GENERATOR_FILE),
        &render_generator(target, &cinrs),
    )?;

    let symbol_count: usize = translated.iter().map(|output| output.symbols.len()).sum();
    let line_count: usize = rust.iter().map(|rust| rust.lines().count()).sum();

    println!(
        "{} に生成しました。\n  \
         翻訳単位 {} 個（シンボル {symbol_count} 個、Rust のコード {line_count} 行）\n  \
         ファイル {} 個（うち更新 {written} 個）、削除 {} 個",
        sys_crate.join(&target_dir).display(),
        units.len(),
        layout.files.len(),
        stale.len(),
    );

    Ok(())
}

/// FFmpeg のツリーがあることを確かめる。
///
/// ツリーは Git のサブモジュールなので、`--recurse-submodules` を付けずに
/// クローンした直後は空のディレクトリになっている。そのまま変換すると、cinrs が
/// 「ファイルが見つからない」というエラーを翻訳単位の数だけ出し、原因が
/// 分かりにくい。先にここで止めて、何をすれば良いかを伝える。
fn require_ffmpeg_tree() -> Result<()> {
    // どのファイルでも良いが、libavutil の中心のヘッダで代表させている。
    let marker = Path::new(FFMPEG_DIR).join("libavutil/avutil.h");

    if marker.is_file() {
        Ok(())
    } else {
        Err(format!(
            "FFmpeg のツリーがありません（{} が見つかりません）。\n\
             サブモジュールを取得してください: git submodule update --init",
            marker.display(),
        )
        .into())
    }
}

/// クレートのルートのファイル（`src/lib.rs`）が、このターゲットの生成物を読んで
/// いることを確かめる。
///
/// `lib.rs` は、ターゲットごとの生成物を条件付きで選ぶ手書きのファイルなので、
/// 生成はしない。新しいターゲットの生成物を足したときに宣言を足し忘れると、
/// 生成したコードがどこからも読まれないまま残る。変換には時間が掛かるので、
/// 変換の前に確かめる。
fn check_root_declaration(target: &str) -> Result<()> {
    let lib_rs = Path::new("src/lib.rs");
    let content = fs::read_to_string(lib_rs)
        .map_err(|error| format!("{} を読めません: {error}", lib_rs.display()))?;

    let declaration = format!("#[path = \"../{TARGETS_DIR}/{target}/{RUST_DIR}/mod.rs\"]");

    if content.lines().any(|line| line.trim() == declaration) {
        Ok(())
    } else {
        Err(format!(
            "{} に `{declaration}` がありません。\n\
             このターゲットの生成物を読むモジュールの宣言を、追加してください。",
            lib_rs.display(),
        )
        .into())
    }
}

/// `content` を `path` に書き出す。書き出したかどうかを返す。
///
/// 内容が変わらないファイルには触れない。書き直すと更新時刻が変わり、何も
/// 変わっていないのに再ビルドが走る。
fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if fs::read_to_string(path).is_ok_and(|current| current == content) {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, content)?;

    Ok(true)
}

/// `dir` 以下のファイルを集め、[`GENERATED_MARKER`] で始まるかどうかを調べる。
/// パスは `root` からの相対パス。
fn scan_files(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, bool>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if entry.file_type()?.is_dir() {
            scan_files(root, &path, files)?;
            continue;
        }

        // 生成物は UTF-8 のテキストだけ。読めないファイルは、生成物ではない。
        let is_generated =
            fs::read_to_string(&path).is_ok_and(|content| content.starts_with(GENERATED_MARKER));

        let relative = path.strip_prefix(root).expect("root 以下を走査している");
        files.insert(relative.to_owned(), is_generated);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "aarch64-apple-darwin";

    fn units(paths: &[&str]) -> Vec<Unit> {
        paths
            .iter()
            .map(|path| Unit::parse(path).unwrap())
            .collect()
    }

    fn layout(paths: &[&str]) -> Layout {
        let units = units(paths);
        let rust = vec!["mod unit {}\n"; units.len()];

        Layout::build(TARGET, &units, &rust)
    }

    fn existing(files: &[(&str, bool)]) -> BTreeMap<PathBuf, bool> {
        files
            .iter()
            .map(|(path, is_generated)| (PathBuf::from(path), *is_generated))
            .collect()
    }

    #[test]
    fn splits_path_into_library_and_name() {
        let unit = Unit::parse("libavutil/time.c").unwrap();

        assert_eq!(unit.library, "libavutil");
        assert_eq!(unit.name, "time");
        assert_eq!(unit.source_path(), "libavutil/time.c");
        assert_eq!(unit.module_file(), Path::new("libavutil/time.rs"));
    }

    /// モジュールの名前にできないパスは、名前を勝手に変えずにエラーにする。
    /// 音声のコーデックを足すと、サブディレクトリの翻訳単位（`libavcodec/aac/`
    /// など）が現れるはずなので、そのときにここで気づける。
    #[test]
    fn rejects_paths_that_cannot_become_modules() {
        for path in [
            "libavcodec/aac/aacdec.c",
            "libavcodec/8svx.c",
            "libavutil/match.c",
            "libavutil/x86-cpu.c",
            "libavutil/timer.h",
            "toplevel.c",
        ] {
            assert!(Unit::parse(path).is_err(), "{path}");
        }
    }

    /// 生成されるファイルの全体。1 行目が印で始まることと、cinrs が出力した
    /// コードに手を加えずに後ろへ続けることを固定している。
    #[test]
    fn renders_unit() {
        let rendered = render_unit(
            &Unit::parse("libavutil/time.c").unwrap(),
            TARGET,
            "mod unit {}\n",
        );

        let expected = "\
// @generated by `cargo xtask ffmpeg-units`. 手で編集しない。
// 変換の指定を変えるときは、xtask の `ffmpeg_units` を変更して再生成する。

//! FFmpeg の `libavutil/time.c` を、cinrs で aarch64-apple-darwin 向けの Rust に変換したもの。
//!
//! 元のファイルの著作権とライセンスの表示は、FFmpeg のツリーの同じパスにある。

mod unit {}
";

        assert_eq!(rendered, expected);
        assert!(rendered.starts_with(GENERATED_MARKER));
    }

    /// 翻訳単位、ライブラリごとの宣言、ターゲットのルートの宣言が揃うこと。
    /// 宣言が欠けると、生成したファイルがコンパイルされないまま残る。
    #[test]
    fn declares_every_unit_and_library() {
        let layout = layout(&["libavutil/time.c", "libavformat/mux.c", "libavutil/log.c"]);

        assert_eq!(
            layout.files.keys().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                &PathBuf::from("libavformat.rs"),
                &PathBuf::from("libavformat/mux.rs"),
                &PathBuf::from("libavutil.rs"),
                &PathBuf::from("libavutil/log.rs"),
                &PathBuf::from("libavutil/time.rs"),
                &PathBuf::from("mod.rs"),
            ]),
        );

        let root = &layout.files[Path::new("mod.rs")];
        assert!(root.ends_with("pub mod libavformat;\npub mod libavutil;\n"));

        let library = &layout.files[Path::new("libavutil.rs")];
        assert!(library.ends_with("pub mod log;\npub mod time;\n"));

        assert!(
            layout
                .files
                .values()
                .all(|content| content.starts_with(GENERATED_MARKER))
        );
    }

    /// 一覧から消えた翻訳単位やライブラリの生成物は削除する。今も生成される
    /// ファイルは、削除の対象にしないこと。
    #[test]
    fn removes_only_stale_generated_files() {
        let layout = layout(&["libavutil/time.c"]);

        let existing = existing(&[
            ("mod.rs", true),
            ("libavutil.rs", true),
            ("libavutil/time.rs", true),
            ("libavutil/removed.rs", true),
            ("libswscale.rs", true),
            ("libswscale/swscale.rs", true),
        ]);

        let stale: BTreeSet<PathBuf> = layout.plan(&existing).unwrap().into_iter().collect();

        assert_eq!(
            stale,
            BTreeSet::from([
                PathBuf::from("libavutil/removed.rs"),
                PathBuf::from("libswscale.rs"),
                PathBuf::from("libswscale/swscale.rs"),
            ]),
        );
    }

    /// 印の無いファイルは、生成物と同じ名前でも、宣言されない名前でも、上書きも
    /// 削除もせずに止まる。手で置いたファイルを黙って消さないため。
    #[test]
    fn refuses_to_touch_unmarked_files() {
        let layout = layout(&["libavutil/time.c"]);

        for path in ["libavutil/time.rs", "libavutil/helper.rs", "notes.txt"] {
            assert!(layout.plan(&existing(&[(path, false)])).is_err(), "{path}");
        }
    }

    /// 2 つの翻訳単位が同じシンボルを定義していたら、止める。rustc は別々の
    /// モジュールにある同じ名前のシンボルをエラーにしないので、ここで見逃すと、
    /// 利用する側の最終的なリンクまで気づけない。
    #[test]
    fn rejects_symbol_defined_by_two_units() {
        let units = units(&["libavutil/log.c", "libavutil/time.c"]);

        let translated = |symbols: &[&str]| Translated {
            rust: String::new(),
            symbols: symbols.iter().map(|symbol| (*symbol).to_owned()).collect(),
            warnings: Vec::new(),
        };

        let distinct = [translated(&["av_log"]), translated(&["av_gettime"])];
        assert!(check_duplicate_symbols(&units, &distinct).is_ok());

        let duplicated = [
            translated(&["av_log", "helper"]),
            translated(&["av_gettime", "helper"]),
        ];
        let error = check_duplicate_symbols(&units, &duplicated).unwrap_err();
        assert!(error.to_string().contains("helper"), "{error}");
    }

    /// 実際に書き出される形の一覧を読めること。書き出す側（`ffmpeg-manifest`）と
    /// 読む側で、キーの名前や階層がずれていないことの確認。
    #[test]
    fn parses_manifest() {
        let text = r#"
[ffmpeg]
version = "n9.0.2"

[compile]
std = "c17"
defines = [
    "PIC",
]
include_dirs = [
    "FFmpeg_generated",
    "FFmpeg",
]

[sources]
translation_units = [
    "libavutil/log.c",
]
headers = []
"#;

        let manifest = Manifest::parse(text).unwrap();

        assert_eq!(manifest.c_standard, "c17");
        assert_eq!(manifest.defines, ["PIC"]);
        assert_eq!(manifest.include_dirs, ["FFmpeg_generated", "FFmpeg"]);
        assert_eq!(manifest.translation_units, ["libavutil/log.c"]);
    }

    /// xtask 自身の `Cargo.toml` から、cinrs のコミットを読めること。依存の
    /// 書き方を変えて読めなくなった場合に、タスクを実行する前に気づける。
    #[test]
    fn reads_pinned_cinrs_commit_from_own_manifest() {
        let cinrs = CinrsSource::parse(XTASK_MANIFEST).unwrap();

        assert!(cinrs.git.starts_with("https://"), "{}", cinrs.git);
        assert_eq!(cinrs.rev.len(), 40, "{}", cinrs.rev);
        assert!(cinrs.rev.chars().all(|digit| digit.is_ascii_hexdigit()));
    }

    /// コミットで固定していない依存は、生成物を作った cinrs を特定できないので
    /// 受け付けない。
    #[test]
    fn rejects_cinrs_dependency_without_pinned_commit() {
        for dependency in [
            r#"cinrs-core = "0.2.0""#,
            r#"cinrs-core = { git = "https://example.invalid/cinrs", branch = "master" }"#,
        ] {
            let manifest = format!("[dependencies]\n{dependency}\n");

            assert!(CinrsSource::parse(&manifest).is_err(), "{dependency}");
        }
    }

    #[test]
    fn renders_generator_record() {
        let cinrs = CinrsSource {
            git: "https://example.invalid/cinrs".to_owned(),
            rev: "0123456789abcdef0123456789abcdef01234567".to_owned(),
        };

        let table: toml::Table = render_generator(TARGET, &cinrs).parse().unwrap();

        assert_eq!(table["target"].as_str(), Some(TARGET));
        assert_eq!(table["cinrs"]["git"].as_str(), Some(cinrs.git.as_str()));
        assert_eq!(table["cinrs"]["rev"].as_str(), Some(cinrs.rev.as_str()));
    }
}
