//! FFmpeg の C のコードを、cinrs で Rust のコードに変換する。
//!
//! ビルドマニフェスト（`cinrs_manifest.toml`）の翻訳単位を 1 つずつ変換し、Cargo が用意する
//! 出力用のディレクトリ（`OUT_DIR`）に書き出す。`src/lib.rs` が、それを読み込む。
//!
//! - `<ライブラリ>/<名前>.rs`: 翻訳単位 1 つを変換したもの。
//! - `generated.rs`: モジュールの宣言。`src/lib.rs` が `include!` で読む。
//!
//! # ビルドスクリプトで変換する理由
//!
//! cinrs のマクロ（`c17!` など）を翻訳単位ごとに書く方法もあるが、次の理由で
//! 採っていない。
//!
//! - マクロでは、翻訳単位が使う項目だけを出力する指定を選べない。ヘッダの
//!   中身がすべて Rust のコードになり、ビルドが 1 分以上長くなる。指定の説明は
//!   `build/translate.rs` にある。
//! - マクロは、クレートがコンパイルされるたびに、1 つずつ順に展開される。
//!   ビルドスクリプトは、入力が変わらない限り再実行されず、翻訳単位を並列に
//!   変換できる。
//!
//! 変換済みの Rust のコードをリポジトリやクレートに入れておく方法も採っていない。
//! 変換後のコードには、FFmpeg のコードに加えて、プラットフォームのヘッダの宣言や
//! インライン関数が含まれる。それを配布する代わりに、FFmpeg のソースを無編集で
//! 配布し、ビルドするマシンで、そのマシンのヘッダを読んで変換する。C コンパイラで
//! ビルドする場合と同じ形になる。
//!
//! そのため、このクレートのビルドには、FFmpeg のツリーと、プラットフォームの
//! ヘッダ（macOS なら Command Line Tools の SDK）が要る。C コンパイラは要らない。

#[path = "build/translate.rs"]
mod translate;

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    num::NonZero,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};

use self::translate::{Settings, Translated};

/// エラーは表示して終了するだけなので、型を分けずに文字列で扱う。
type Result<T> = std::result::Result<T, String>;

/// ビルドマニフェスト。`cargo xtask ffmpeg-manifest` が書き出す。
const MANIFEST_FILE: &str = "cinrs_manifest.toml";

/// FFmpeg のツリー（サブモジュール）。
const FFMPEG_DIR: &str = "FFmpeg";

/// モジュールの宣言を書き出すファイル。`OUT_DIR` からの相対パス。
///
/// `src/lib.rs` の `include!` と揃える。
const ROOT_FILE: &str = "generated.rs";

/// 変換を行うスレッドのスタックの大きさ。
///
/// 変換後のコードを文字列にする処理は、式の入れ子の深さだけ再帰する。C の
/// マクロが展開された式は深くなりやすく、スレッドの既定の大きさ（2 MiB）に
/// 収まる保証が無いので、余裕を持たせている。確保されるのは仮想メモリで、
/// 使わなかった分は実メモリを消費しない。
const WORKER_STACK_SIZE: usize = 64 << 20;

fn main() -> ExitCode {
    // ビルドスクリプトが何かを `rerun-if-changed` で指定すると、Cargo は指定した
    // ファイルが変わったときだけ再実行する。指定が 1 つも無いと、パッケージ内の
    // どのファイルが変わっても再実行される。エラーで終わる場合にも効かせるために、
    // 最初に出力する。
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build/translate.rs");
    println!("cargo:rerun-if-changed={MANIFEST_FILE}");

    match run() {
        Ok(()) => ExitCode::SUCCESS,

        Err(error) => {
            // `cargo::error` に渡せるのは 1 行だけ。変換のエラーは複数行になるので、
            // 全文は標準エラー出力に書く。Cargo は、ビルドスクリプトが失敗したときに
            // 標準エラー出力の内容を表示する。
            eprintln!("{error}");

            let summary = error.lines().next().unwrap_or_default();
            println!("cargo::error={summary}");

            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    // Cargo は、ビルドスクリプトをパッケージのルートを作業ディレクトリにして
    // 実行する。以降のパスは、すべてそこからの相対パスで扱う。
    // cinrs に絶対パスを渡すと困るが、`build.rs` では上記により問題ない。
    // 理由は `Settings::translate` の説明にある。

    let manifest_text = fs::read_to_string(MANIFEST_FILE)
        .map_err(|error| format!("{MANIFEST_FILE} を読めません: {error}"))?;
    let manifest = Manifest::parse(&manifest_text)?;

    let target = cargo_env("TARGET")?;

    check_target(&manifest, &target)?;
    require_ffmpeg_tree()?;

    // 翻訳単位を名前の順に並べ、重複を除く。出力を、ビルドマニフェストの側の順序に
    // 左右されないようにするため。
    let units: Vec<Unit> = manifest
        .translation_units
        .iter()
        .map(|path| Unit::parse(path))
        .collect::<Result<BTreeSet<_>>>()?
        .into_iter()
        .collect();

    let settings = Settings {
        c_standard: manifest.c_standard,
        target,
        defines: manifest.defines,
        include_dirs: manifest.include_dirs.iter().map(PathBuf::from).collect(),
    };

    let translated = translate_units(&settings, &units)?;

    check_duplicate_symbols(&units, &translated)?;

    let out_dir = PathBuf::from(cargo_env("OUT_DIR")?);

    for (unit, output) in units.iter().zip(&translated) {
        write_file(
            &out_dir.join(unit.module_file()),
            &render_unit(unit, &output.rust),
        )?;

        // 警告は 1 行にして Cargo に渡す。改行があると、2 行目以降が Cargo への
        // 指示として解釈されうる。
        for warning in &output.warnings {
            let warning = warning.replace('\n', " ");
            println!("cargo::warning={}: {warning}", unit.source_path());
        }
    }

    write_file(&out_dir.join(ROOT_FILE), &render_root(&out_dir, &units)?)?;

    // 変換の入力が変わったら、変換し直す。翻訳単位の `.c` と、それが読んだ
    // ヘッダのすべて。プラットフォームのヘッダも含めているので、SDK を更新した
    // ときにも変換し直される。
    let mut inputs = BTreeSet::new();

    for (unit, output) in units.iter().zip(&translated) {
        inputs.insert(unit.source_file().display().to_string());
        inputs.extend(output.headers.iter().cloned());
    }

    for input in inputs {
        println!("cargo:rerun-if-changed={input}");
    }

    Ok(())
}

/// Cargo がビルドスクリプトに設定する環境変数を読む。
fn cargo_env(name: &str) -> Result<String> {
    env::var(name).map_err(|error| format!("環境変数 {name} を読めません: {error}"))
}

/// ビルドマニフェストのうち、変換に使う部分。
#[derive(Debug)]
struct Manifest {
    /// ビルドマニフェストを作ったマシンのアーキテクチャ。`std::env::consts::ARCH` の値。
    host_arch: String,

    /// ビルドマニフェストを作ったマシンの OS。`std::env::consts::OS` の値。
    host_os: String,

    /// C の規格。`-std=` の値と同じ書き方。
    c_standard: String,

    /// マクロ定義。`NAME` または `NAME=VALUE` の形。
    defines: Vec<String>,

    /// インクルードディレクトリ。パッケージのルートからの相対パス。
    include_dirs: Vec<String>,

    /// 翻訳単位。FFmpeg のツリーからの相対パス。
    translation_units: Vec<String>,
}

impl Manifest {
    fn parse(text: &str) -> Result<Self> {
        let table: toml::Table = text
            .parse()
            .map_err(|error| format!("{MANIFEST_FILE} を読めません: {error}"))?;

        Ok(Self {
            host_arch: string(&table, "host", "arch")?,
            host_os: string(&table, "host", "os")?,
            c_standard: string(&table, "compile", "std")?,
            defines: string_array(&table, "compile", "defines")?,
            include_dirs: string_array(&table, "compile", "include_dirs")?,
            translation_units: string_array(&table, "sources", "translation_units")?,
        })
    }
}

fn string(table: &toml::Table, section: &str, key: &str) -> Result<String> {
    table
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{MANIFEST_FILE} に {section}.{key} の文字列がありません"))
}

fn string_array(table: &toml::Table, section: &str, key: &str) -> Result<Vec<String>> {
    let missing = || format!("{MANIFEST_FILE} に {section}.{key} の文字列の配列がありません");

    table
        .get(section)
        .and_then(|section| section.get(key))
        .and_then(toml::Value::as_array)
        .ok_or_else(missing)?
        .iter()
        .map(|value| value.as_str().map(str::to_owned).ok_or_else(missing))
        .collect()
}

/// ビルド先が、取り込んだ FFmpeg の構成で扱えるターゲットであることを確かめる。
///
/// 止める場合は 2 つある。どちらも、そのまま進むと、変換は通るのに動作が
/// 食い違うか、プラットフォームのヘッダのエラーが大量に出て原因が分かりにくい。
fn check_target(manifest: &Manifest, target: &str) -> Result<()> {
    // `FFmpeg_generated/config.h` は、`configure` を実行したマシンを調べた結果で、
    // 型の大きさや使える関数がそのマシンのものになっている。別のターゲットに
    // 使うと、FFmpeg が存在しない関数を呼んだり、違う分岐を選んだりする。
    let target_arch = cargo_env("CARGO_CFG_TARGET_ARCH")?;
    let target_os = cargo_env("CARGO_CFG_TARGET_OS")?;

    if (target_arch.as_str(), target_os.as_str())
        != (manifest.host_arch.as_str(), manifest.host_os.as_str())
    {
        return Err(format!(
            "mini-ffmpeg-sys は {target} 向けにビルドできません。\n\
             取り込んでいる FFmpeg の構成（{MANIFEST_FILE} と FFmpeg_generated）は、\
             {} の {} 向けに生成したものです。",
            manifest.host_os, manifest.host_arch,
        ));
    }

    // 変換は、ビルドスクリプトが動いているマシンのプラットフォームのヘッダを
    // 読んで行う。ビルド先が別のマシンだと、ヘッダとターゲットが食い違う。
    let host = cargo_env("HOST")?;

    if host != target {
        return Err(format!(
            "mini-ffmpeg-sys はクロスコンパイルに対応していません\
             （ホストが {host}、ターゲットが {target}）。\n\
             C のコードの変換に、ターゲットのプラットフォームのヘッダが要ります。"
        ));
    }

    Ok(())
}

/// FFmpeg のツリーがあることを確かめる。
///
/// ツリーは Git のサブモジュールなので、`--recurse-submodules` を付けずに
/// クローンした直後は空のディレクトリになっている。そのまま変換すると、cinrs が
/// 「ファイルが見つからない」というエラーを翻訳単位の数だけ出し、原因が
/// 分かりにくい。先にここで止めて、何をすれば良いかを伝える。
///
/// crates.io から取得した場合は、ツリーがパッケージに同梱されているので、
/// この検査には掛からない。
fn require_ffmpeg_tree() -> Result<()> {
    // どのファイルでも良いが、libavutil の中心のヘッダで代表させている。
    let marker = Path::new(FFMPEG_DIR).join("libavutil/avutil.h");

    // サブモジュールを取得したあとに、ビルドスクリプトを再実行させる。
    println!("cargo:rerun-if-changed={}", marker.display());

    if marker.is_file() {
        Ok(())
    } else {
        Err(format!(
            "FFmpeg のツリーがありません（{} が見つかりません）。\
             サブモジュールを取得してください: git submodule update --init",
            marker.display(),
        ))
    }
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
        let unsupported =
            |reason: &str| format!("翻訳単位 {path} をモジュールにできません: {reason}");

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

    /// 翻訳単位のファイル。パッケージのルートからの相対パス。
    fn source_file(&self) -> PathBuf {
        Path::new(FFMPEG_DIR).join(self.source_path())
    }

    /// 変換したファイル。`OUT_DIR` からの相対パス。
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
/// なった翻訳単位があれば、すべてをまとめて報告する。1 つ直すたびに全体を
/// 変換し直さずに済ませるため。
fn translate_units(settings: &Settings, units: &[Unit]) -> Result<Vec<Translated>> {
    let next_index = AtomicUsize::new(0);
    let worker_count = thread::available_parallelism().map_or(1, NonZero::get);

    let work = || {
        let mut finished = Vec::new();

        // 翻訳単位ごとに掛かる時間が大きく違うので、先に均等に分けず、
        // 手の空いたスレッドが次の 1 つを取っていく。
        while let Some(unit) = units.get(next_index.fetch_add(1, Ordering::Relaxed)) {
            finished.push((unit, settings.translate(&unit.source_file())));
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
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|error| format!("変換のスレッドを起動できません: {error}"))?;

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

        return Err(message);
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
/// このクレートのビルドだけでは気づけない。
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
                ));
            }
        }
    }

    Ok(())
}

/// 翻訳単位 1 つ分のファイルを組み立てる。cinrs が出力したコードに、説明を付ける。
fn render_unit(unit: &Unit, rust: &str) -> String {
    format!(
        "//! FFmpeg の `{}` を、cinrs で Rust に変換したもの。\n\n{rust}\n",
        unit.source_path(),
    )
}

/// モジュールの宣言のファイルを組み立てる。`libavutil/time.c` を
/// `libavutil::time` として読めるように、ライブラリのモジュールの中に、
/// 翻訳単位のモジュールを宣言する。
///
/// 翻訳単位のファイルの場所は、`#[path]` に絶対パスで書く。`include!` で読んだ
/// ファイルの中の `mod time;` は、`OUT_DIR` ではなく、`include!` を書いた側の
/// ファイル（`src/lib.rs`）の場所から探されるので、パスを省くと見つからない。
/// 絶対パスが入るのは `OUT_DIR` の中のこのファイルだけで、配布物には入らない。
fn render_root(out_dir: &Path, units: &[Unit]) -> Result<String> {
    let mut by_library: BTreeMap<&str, Vec<&Unit>> = BTreeMap::new();

    for unit in units {
        by_library.entry(&unit.library).or_default().push(unit);
    }

    let mut out = String::new();

    for (library, units) in by_library {
        let _ = writeln!(
            out,
            "/// {library} の翻訳単位。FFmpeg の `.c` 1 つにつき、モジュールを 1 つ置く。\n\
             pub mod {library} {{",
        );

        for unit in units {
            let file = out_dir.join(unit.module_file());
            let file = file.to_str().ok_or_else(|| {
                format!(
                    "出力先のパス {} を UTF-8 の文字列にできません",
                    file.display()
                )
            })?;

            // `{:?}` で、パスを Rust の文字列リテラルにする。Windows のパスの
            // `\` などが、エスケープされる。
            let _ = writeln!(out, "    #[path = {file:?}]\n    pub mod {};", unit.name);
        }

        out.push_str("}\n\n");
    }

    Ok(out)
}

/// `content` を `path` に書き出す。親のディレクトリが無ければ作る。
fn write_file(path: &Path, content: &str) -> Result<()> {
    let failed = |error| format!("{} に書き出せません: {error}", path.display());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }

    fs::write(path, content).map_err(failed)
}
