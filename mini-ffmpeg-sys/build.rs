//! FFmpeg の C のコードを、cinrs で Rust のコードに変換する。
//!
//! ビルドマニフェスト（`FFmpeg_build/<ターゲット>/cinrs_manifest.toml`）の翻訳単位を 1 つずつ変換し、Cargo が用意する
//! 出力用のディレクトリ（`OUT_DIR`）に書き出す。`src/lib.rs` が、それを読み込む。
//!
//! - `<ライブラリ>/<名前>.rs`: 翻訳単位 1 つを変換したもの。FFmpeg の関数と変数の
//!   定義を持つ。
//! - `public_api.rs`: 公開ヘッダ（ビルドマニフェストの `public_headers`）を
//!   変換したもの。利用者に見せる関数の宣言、型、列挙子を持つ。
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

use cinrs_core::include::SYSTEM_PATH_ENV_VAR;

use self::translate::{Settings, Translated};

/// エラーは表示して終了するだけなので、型を分けずに文字列で扱う。
type Result<T> = std::result::Result<T, String>;

/// ターゲットごとのビルド構成を置くディレクトリ。`FFmpeg_build/<ターゲットトリプル>/` に、
/// ビルドマニフェストと `configure` / `make` の生成物がある。
const BUILD_CONFIG_DIR: &str = "FFmpeg_build";

/// ビルドマニフェスト。ターゲットの構成のディレクトリの中にある。
/// `cargo xtask ffmpeg-manifest` が書き出す。
const MANIFEST_FILE: &str = "cinrs_manifest.toml";

/// ビルドマニフェストのパスを、テストに伝える環境変数。
///
/// ビルドマニフェストはターゲットごとに別の場所にあり、`include_str!` に固定の
/// パスを書けない。そのため、ビルドスクリプトが選んだパスを `rustc-env` で渡す。
const MANIFEST_PATH_ENV: &str = "MINI_FFMPEG_SYS_MANIFEST";

/// FFmpeg のツリー（サブモジュール）。
const FFMPEG_DIR: &str = "FFmpeg";

/// モジュールの宣言を書き出すファイル。`OUT_DIR` からの相対パス。
///
/// `src/lib.rs` の `include!` と揃える。
const ROOT_FILE: &str = "generated.rs";

/// 公開ヘッダを変換したファイル。`OUT_DIR` からの相対パス。
const PUBLIC_API_FILE: &str = "public_api.rs";

/// 公開ヘッダを include するだけの翻訳単位の名前。診断と `__FILE__` に使われる。
///
/// ファイルとしては存在しない。C のコードは、ビルドスクリプトが組み立てて、
/// 文字列のまま cinrs に渡す。
const PUBLIC_API_SOURCE_NAME: &str = "public_api.c";

/// FFmpeg の公開の関数と変数の名前が、共通して持つ先頭の文字列。
///
/// 公開 API の翻訳単位には、FFmpeg の宣言のほかに、公開ヘッダが読む
/// プラットフォームのヘッダの宣言（`strcat` など）も入る。宣言と定義の突き合わせ
/// （[`check_public_declarations`]）では、FFmpeg の宣言だけを見る必要がある。
/// cinrs は宣言がどのヘッダから来たかを返さないので、名前で見分ける。
///
/// FFmpeg の公開の名前は、`av_`、`avio_`、`avcodec_`、`avformat_`、`avutil_` の
/// ように、すべて `av` で始まる命名規則になっている。外れた場合の振る舞いは
/// 次の通り。
///
/// - プラットフォームに `av` で始まる関数があると、定義が無いと誤って報告されて
///   止まる。[`UNDEFINED_PUBLIC_DECLARATIONS`] に足すかを判断する。
/// - `av` で始まらない FFmpeg の公開の名前があると、検査されない。
const FFMPEG_PUBLIC_PREFIX: &str = "av";

/// 公開ヘッダが宣言しているが、この構成では定義されない名前。
///
/// 呼ばなければ問題は無いが、呼ぶとリンクで失敗する。宣言が見えていても使えない
/// ことを、ここに理由とともに残す。
const UNDEFINED_PUBLIC_DECLARATIONS: &[&str] = &[
    // `libavcodec/jni.h` が無条件に宣言している、Android 専用の関数。定義は
    // `libavcodec/jni.c` の `#if defined(__ANDROID__)` の中にある。ヘッダの説明にも
    // 「Android でのみ使える」とある。
    "av_jni_get_android_app_ctx",
    "av_jni_set_android_app_ctx",
];

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
    println!("cargo:rerun-if-changed=build/platform_workarounds.rs");

    // ビルドマニフェストはターゲットごとに別のファイルなので、今のターゲットのものを
    // 指定する。無い場合も、置かれたときに再実行させるために指定する。
    if let Ok(target) = env::var("TARGET") {
        println!(
            "cargo:rerun-if-changed={}",
            manifest_path(&target).display()
        );
    }

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

    let target = cargo_env("TARGET")?;
    let manifest_path = manifest_path(&target);

    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "{} を読めません: {error}\n\
             {target} 向けの構成が取り込まれていない可能性があります。\
             取り込み済みのターゲット: {}",
            manifest_path.display(),
            available_targets(),
        )
    })?;
    let manifest = Manifest::parse(&manifest_text)?;

    println!(
        "cargo:rustc-env={MANIFEST_PATH_ENV}={}",
        env::current_dir()
            .map_err(|error| format!("作業ディレクトリを取得できません: {error}"))?
            .join(&manifest_path)
            .display(),
    );

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

    // 公開 API の翻訳単位は 1 つで、絞り込みをしないので、実装の翻訳単位の
    // 1 つより時間が掛かる。実装の翻訳単位の変換と並べて進める。
    let (translated, public_api) = thread::scope(|scope| {
        let public_api = thread::Builder::new()
            .stack_size(WORKER_STACK_SIZE)
            .spawn_scoped(scope, || {
                translate_public_api(&settings, &manifest.public_headers)
            })
            .map_err(|error| format!("変換のスレッドを起動できません: {error}"))?;

        let translated = translate_units(&settings, &units);
        let public_api = public_api
            .join()
            .map_err(|_| "変換のスレッドが異常終了しました".to_owned())?;

        Ok::<_, String>((translated?, public_api?))
    })?;

    check_duplicate_symbols(&units, &translated)?;
    check_public_declarations(&public_api, &translated)?;

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

    write_file(
        &out_dir.join(PUBLIC_API_FILE),
        &render_public_api(&public_api.rust),
    )?;

    for warning in &public_api.warnings {
        let warning = warning.replace('\n', " ");
        println!("cargo::warning=公開ヘッダ: {warning}");
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

    inputs.extend(public_api.headers.iter().cloned());

    for input in inputs {
        println!("cargo:rerun-if-changed={input}");
    }

    Ok(())
}

/// ターゲットのビルドマニフェストのパス。パッケージのルートからの相対パス。
fn manifest_path(target: &str) -> PathBuf {
    Path::new(BUILD_CONFIG_DIR).join(target).join(MANIFEST_FILE)
}

/// 取り込み済みのターゲットトリプルの一覧。エラー文に添えるための文字列。
fn available_targets() -> String {
    let mut targets: Vec<String> = fs::read_dir(BUILD_CONFIG_DIR)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();

    targets.sort();

    if targets.is_empty() {
        "なし".to_owned()
    } else {
        targets.join(", ")
    }
}

/// Cargo がビルドスクリプトに設定する環境変数を読む。
fn cargo_env(name: &str) -> Result<String> {
    env::var(name).map_err(|error| format!("環境変数 {name} を読めません: {error}"))
}

/// ビルドマニフェストのうち、変換に使う部分。
#[derive(Debug)]
struct Manifest {
    /// この構成が対象とするターゲットトリプル。Cargo の `TARGET` と同じ書き方。
    triple: String,

    /// C の規格。`-std=` の値と同じ書き方。
    c_standard: String,

    /// マクロ定義。`NAME` または `NAME=VALUE` の形。
    defines: Vec<String>,

    /// インクルードディレクトリ。パッケージのルートからの相対パス。
    include_dirs: Vec<String>,

    /// 翻訳単位。FFmpeg のツリーからの相対パス。
    translation_units: Vec<String>,

    /// 公開ヘッダ。`#include` に書く名前（`libavutil/avutil.h` の形）。
    public_headers: Vec<String>,
}

impl Manifest {
    fn parse(text: &str) -> Result<Self> {
        let table: toml::Table = text
            .parse()
            .map_err(|error| format!("{MANIFEST_FILE} を読めません: {error}"))?;

        Ok(Self {
            triple: string(&table, "target", "triple")?,
            c_standard: string(&table, "compile", "std")?,
            defines: string_array(&table, "compile", "defines")?,
            include_dirs: string_array(&table, "compile", "include_dirs")?,
            translation_units: string_array(&table, "sources", "translation_units")?,
            public_headers: string_array(&table, "sources", "public_headers")?,
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
    // `FFmpeg_build/<ターゲット>/config.h` は、`configure` を実行したマシンを調べた結果で、
    // 型の大きさや使える関数がそのマシンのものになっている。別のターゲットに
    // 使うと、FFmpeg が存在しない関数を呼んだり、違う分岐を選んだりする。
    //
    // 構成はターゲットのディレクトリから選んでいるので、普通は一致する。食い違うのは、
    // 別のターゲットのディレクトリを名前を変えてコピーしたときなど。
    if manifest.triple != target {
        return Err(format!(
            "{BUILD_CONFIG_DIR}/{target} の {MANIFEST_FILE} は、{} 向けの構成です。\n\
             ディレクトリの名前とターゲットが食い違っています。\
             `cargo xtask ffmpeg-manifest` を {target} のマシンで実行して作り直してください。",
            manifest.triple,
        ));
    }

    check_system_include_path(target)
}

/// プラットフォームのヘッダの置き場所が、ターゲットのものになることを確かめる。
///
/// 変換は、ビルドスクリプトが動いているマシンのプラットフォームのヘッダを
/// 読んで行う。cinrs は、そのマシンに合うディレクトリだけを自分で探す。
///
/// - ターゲットがホストと違う場合: ホストのヘッダはターゲットと配置が違う
///   （`struct stat` のオフセットなど）ので、cinrs は探さない。
/// - Linux と macOS 以外（Windows など）: 探す規則が無い。
///
/// どちらも [`SYSTEM_PATH_ENV_VAR`] で、ターゲット用のディレクトリを指定させる。
/// cinrs は、変換のときに同じことを検査してエラーにするが、ここで先に止めれば、
/// 何を設定すればよいかを mini-ffmpeg-sys の側の言葉で伝えられる。
///
/// 指定の中身がターゲットのものかは、ここでは分からない。存在しないディレクトリが
/// 混じっていないことだけを見る。
fn check_system_include_path(target: &str) -> Result<()> {
    // 環境変数は、ネイティブのビルドでも cinrs の探索に使われる。値が変わったら
    // 変換し直す。
    println!("cargo:rerun-if-env-changed={SYSTEM_PATH_ENV_VAR}");

    let host = cargo_env("HOST")?;
    let target_os = cargo_env("CARGO_CFG_TARGET_OS")?;

    let reason = if host != target {
        format!("ホストが {host}、ターゲットが {target} で、ホストのヘッダはターゲットに使えません")
    } else if !matches!(target_os.as_str(), "linux" | "macos") {
        format!("{target_os} 向けには、cinrs がヘッダの場所を自分では探せません")
    } else {
        return Ok(());
    };

    // cinrs と同じ規則で読む。空の要素は無視し、残りが無ければ未設定と同じ。
    let directories: Vec<PathBuf> = env::var_os(SYSTEM_PATH_ENV_VAR)
        .iter()
        .flat_map(env::split_paths)
        .filter(|directory| !directory.as_os_str().is_empty())
        .collect();

    if directories.is_empty() {
        return Err(format!(
            "{SYSTEM_PATH_ENV_VAR} が必要です（{reason}）。\n\
             {target} のプラットフォームのヘッダ（C ライブラリのヘッダ）があるディレクトリを、\
             {SYSTEM_PATH_ENV_VAR} に指定してください。複数ある場合は、パスの区切り文字\
             （Unix は `:`、Windows は `;`）で並べます。"
        ));
    }

    if let Some(missing) = directories.iter().find(|directory| !directory.is_dir()) {
        return Err(format!(
            "{SYSTEM_PATH_ENV_VAR} に指定されたディレクトリ {} がありません。",
            missing.display(),
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

/// 公開ヘッダをすべて include するだけの翻訳単位を組み立て、変換する。
///
/// cinrs が変換できるのは翻訳単位（`.c` と、そこから読まれるヘッダ）で、ヘッダを
/// 単体では渡せない。そこで、`#include` だけを並べた C のコードを作る。
fn translate_public_api(settings: &Settings, public_headers: &[String]) -> Result<Translated> {
    let mut source = String::from(
        "/* mini-ffmpeg-sys のビルドスクリプトが組み立てた翻訳単位。\n\
         * ビルドマニフェストの public_headers を include する。 */\n",
    );

    for header in public_headers {
        let _ = writeln!(source, "#include \"{header}\"");
    }

    let output = settings
        .translate_public_api(PUBLIC_API_SOURCE_NAME, &source)
        .map_err(|error| format!("公開ヘッダを変換できません。\n\n{error}"))?;

    // 公開 API の翻訳単位は宣言と型だけを持つ。シンボルを定義していると、実装の
    // 翻訳単位の定義と重なり、利用する側のリンクで失敗する。
    if !output.symbols.is_empty() {
        return Err(format!(
            "公開ヘッダの翻訳単位が、シンボルを定義しています: {:?}",
            output.symbols,
        ));
    }

    Ok(output)
}

/// 公開 API が宣言している FFmpeg の関数と変数が、すべて実装の翻訳単位で
/// 定義されていることを確かめる。
///
/// 宣言だけで定義が無い名前は、Rust のコードからは呼べるように見えるが、呼ぶと
/// 利用する側のリンクで失敗する。このクレートのビルドでは気づけないので、ここで
/// 確かめる。
///
/// この構成で定義されないと分かっている名前は、[`UNDEFINED_PUBLIC_DECLARATIONS`]
/// に理由とともに並べる。並べた名前が宣言から消えたり、定義されるようになったり
/// したら、役目を終えた項目が残らないよう、それも止める。
fn check_public_declarations(public_api: &Translated, translated: &[Translated]) -> Result<()> {
    let defined: BTreeSet<&str> = translated
        .iter()
        .flat_map(|output| output.symbols.iter().map(String::as_str))
        .collect();

    let declared: BTreeSet<&str> = public_api
        .declarations
        .iter()
        .map(String::as_str)
        .filter(|name| name.starts_with(FFMPEG_PUBLIC_PREFIX))
        .collect();

    let missing: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|name| !defined.contains(name) && !UNDEFINED_PUBLIC_DECLARATIONS.contains(name))
        .collect();

    if !missing.is_empty() {
        return Err(format!(
            "公開ヘッダが宣言している {} 個の名前が、どの翻訳単位でも定義されていません。\n\
             構成で無効にした部品の宣言であれば、UNDEFINED_PUBLIC_DECLARATIONS に理由とともに\
             足してください。\n{missing:?}",
            missing.len(),
        ));
    }

    for name in UNDEFINED_PUBLIC_DECLARATIONS {
        if !declared.contains(name) || defined.contains(name) {
            return Err(format!(
                "UNDEFINED_PUBLIC_DECLARATIONS の {name} は、もう公開ヘッダに宣言が無いか、\
                 定義されるようになりました。項目を削除してください。"
            ));
        }
    }

    Ok(())
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

/// 公開ヘッダを変換したファイルを組み立てる。cinrs が出力したコードに、説明を付ける。
fn render_public_api(rust: &str) -> String {
    format!(
        "//! FFmpeg の公開ヘッダ（ビルドマニフェストの `public_headers`）を、cinrs で\n\
         //! Rust に変換したもの。\n\
         //!\n\
         //! 関数は宣言だけで、実体はライブラリのモジュールの翻訳単位が持つ。両者は\n\
         //! C のシンボルの名前で繋がる。\n\
         \n\
         {rust}\n"
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

    let path_literal = |file: PathBuf| -> Result<String> {
        let file = file.to_str().ok_or_else(|| {
            format!(
                "出力先のパス {} を UTF-8 の文字列にできません",
                file.display()
            )
        })?;

        // `{:?}` で、パスを Rust の文字列リテラルにする。Windows のパスの
        // `\` などが、エスケープされる。
        Ok(format!("{file:?}"))
    };

    let mut out = String::new();

    // 公開 API。クレートのルートに再エクスポートして、`mini_ffmpeg_sys::av_log`
    // のように使えるようにする。
    let _ = writeln!(
        out,
        "/// FFmpeg の公開ヘッダを変換したもの。中身をクレートのルートに再エクスポートする。\n\
         #[path = {}]\n\
         mod public_api;\n\
         \n\
         pub use public_api::*;\n",
        path_literal(out_dir.join(PUBLIC_API_FILE))?,
    );

    for (library, units) in by_library {
        // 実装の翻訳単位は、利用者に見せるものではないので、ドキュメントから隠す。
        // 定義は `#[no_mangle]` のシンボルなので、モジュールが見えるかどうかに
        // 関係なく出力され、公開 API の宣言から使える。
        let _ = writeln!(
            out,
            "/// {library} の翻訳単位。FFmpeg の `.c` 1 つにつき、モジュールを 1 つ置く。\n\
             #[doc(hidden)]\n\
             pub mod {library} {{",
        );

        for unit in units {
            let file = path_literal(out_dir.join(unit.module_file()))?;

            let _ = writeln!(out, "    #[path = {file}]\n    pub mod {};", unit.name);
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
