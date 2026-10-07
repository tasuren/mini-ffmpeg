//! 一覧（`cinrs_manifest.toml`）から、cinrs で取り込むための Rust のコードを
//! 生成するタスク。
//!
//! FFmpeg の `.c` 1 つを、cinrs のマクロ呼び出し 1 つ（＝翻訳単位 1 つ）として
//! 取り込む。生成するのは次の 2 種類のファイルで、どちらも `mini-ffmpeg-sys/src`
//! に置く。
//!
//! - `<ライブラリ>/<名前>.rs`: 翻訳単位 1 つ分の取り込み。
//! - `<ライブラリ>.rs`: そのライブラリの翻訳単位のモジュールの宣言。
//!
//! このタスクは一覧を読むだけで、FFmpeg のツリーも C コンパイラも使わない。
//! 取り込みの形や除外リストを変えたときは、このタスクだけを実行し直せば良い。
//!
//! # 生成物と手書きのファイルの区別
//!
//! 生成したファイルには、先頭に [`GENERATED_MARKER`] を付ける。このタスクが
//! 上書きしたり削除したりするのは、この印があるファイルだけ。印が無いファイルは
//! 手書きとみなし、上書きが必要になった場合はエラーにして止まる。
//!
//! # 例外の扱い
//!
//! cinrs でそのまま取り込めない翻訳単位は、[`EXCLUDED_UNITS`] に載せる。載せた
//! 翻訳単位は、取り込みのファイルを生成しない。代わりに、同じ場所に手書きの
//! ファイルがあることを確かめ、モジュールの宣言だけを生成する。こうすると、
//! 手書きかどうかでモジュールのパスが変わらない。

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use crate::{MANIFEST_FILE, Result, sys_crate_dir};

/// 取り込みのファイルを生成しない翻訳単位。FFmpeg のツリーからの相対パス。
///
/// ここに載せた翻訳単位は、`mini-ffmpeg-sys/src` の同じ場所（`libavutil/log.c`
/// なら `src/libavutil/log.rs`）に、手書きの取り込みを置く必要がある。
///
/// 全翻訳単位に共通で入れられる回避策は、ここではなく
/// `mini-ffmpeg-sys/src/cinrs_ffmpeg_fixups.h` に書く。除外が要るのは、
/// FFmpeg のソースそのものを書き換えないと通らない場合だけ。
const EXCLUDED_UNITS: &[&str] = &[
    // 関数ポインタを整数にキャストした値で `static` の変数を初期化している。
    //
    //     static atomic_uintptr_t av_log_callback = (uintptr_t)av_log_default_callback;
    //
    // Rust の `static` の初期化子にはこの値を書けないので、cinrs が拒否する。
    // プリプロセッサでは回避できず、ソースの書き換えが要る。上流では FFmpeg の
    // コミット 4ce1a456f2 で `_Atomic` な関数ポインタに直されているので、その
    // 修正を取り込んだコピーを `src/libavutil/log.c` に置いている。この修正を
    // 含むバージョンに FFmpeg を上げたら、除外をやめられる。
    "libavutil/log.c",
];

/// 生成したファイルの 1 行目に付ける印。
///
/// `@generated` は、生成されたファイルであることを示す慣習的な表記で、
/// エディタやレビューのツールにも認識されるものがある。
const GENERATED_MARKER: &str = "// @generated";

/// プラットフォームのヘッダを読むための回避策のヘッダ。`mini-ffmpeg-sys/src` からの相対パス。
const PLATFORM_FIXUPS_HEADER: &str = "cinrs_platform_fixups.h";

/// FFmpeg のコードを読むための回避策のヘッダ。`mini-ffmpeg-sys/src` からの相対パス。
const FFMPEG_FIXUPS_HEADER: &str = "cinrs_ffmpeg_fixups.h";

/// 一覧のうち、取り込みのコードの生成に使う部分。
#[derive(Debug)]
struct Manifest {
    /// C の規格。cinrs のマクロの名前（`c17!` など）を決める。
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

    /// 取り込みのファイル。`src` からの相対パス。
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

/// 翻訳単位 1 つ分の取り込みのファイルを組み立てる。
///
/// FFmpeg のソースには手を加えず、元のビルドがコンパイラのオプションで渡して
/// いた指定をマクロの中で再現してから、`.c` を丸ごと include する。並べる順に
/// 意味があるので、順番を変えるときは注意する。
///
/// 1. `#pragma cinrs export`: 外部リンケージを持つものを実際の C シンボルにする。
///    cinrs はマクロ呼び出し 1 つを独立した翻訳単位として扱うので、他の翻訳単位
///    から名前で参照するために要る。
/// 2. `#pragma cinrs system_include first`: プラットフォームのヘッダを、cinrs
///    同梱のヘッダより先に探させる。`first` が無いと両者が混ざり、macOS では
///    `int64_t` の定義が食い違ってエラーになる。`config.h` はプラットフォームの
///    ヘッダを調べた結果なので、同じヘッダを読ませるのが筋でもある。
/// 3. プラットフォーム用の回避策のヘッダ。プラットフォームのヘッダより前に
///    読む必要があるので、インクルードディレクトリの指定より前に置く。
/// 4. `#pragma cinrs include_path`: `-I` に当たる。元のビルドと同じ順。
/// 5. `#define`: `-D` に当たる。
/// 6. FFmpeg 用の回避策のヘッダ。FFmpeg のヘッダを先に読んでからマクロを定義し
///    直すので、インクルードディレクトリとマクロ定義の後に置く。
/// 7. `.c` の include。
fn render_unit(manifest: &Manifest, unit: &Unit) -> Result<String> {
    let source = unit.source_path();
    let mut out = String::new();

    let _ = writeln!(
        out,
        "{GENERATED_MARKER} by `cargo xtask ffmpeg-units`. 手で編集しない。\n\
         // 取り込みの形を変えるときは、xtask の `ffmpeg_units.rs` を変更して再生成する。\n\
         \n\
         //! `{source}` の取り込み。\n",
    );

    let _ = writeln!(out, "cinrs::{}! {{", cinrs_macro(&manifest.c_standard)?);
    let _ = writeln!(out, "    #pragma cinrs export");
    let _ = writeln!(out, "    #pragma cinrs system_include first\n");

    // 取り込みのファイルは `src/<ライブラリ>/` にあり、cinrs は `.rs` ファイルの
    // 場所を起点に探すので、`src` にあるヘッダは 1 つ上になる。
    let _ = writeln!(out, "    #include \"../{PLATFORM_FIXUPS_HEADER}\"\n");

    for dir in &manifest.include_dirs {
        let _ = writeln!(out, "    #pragma cinrs include_path \"{dir}\"");
    }

    out.push('\n');

    for define in &manifest.defines {
        // コンパイラは、値を省いた `-DNAME` を `NAME` を 1 とする定義として扱う。
        let (name, value) = define.split_once('=').unwrap_or((define, "1"));
        let _ = writeln!(out, "    #define {name} {value}");
    }

    let _ = writeln!(out, "\n    #include \"../{FFMPEG_FIXUPS_HEADER}\"\n");
    let _ = writeln!(out, "    #include \"{source}\"");
    out.push_str("}\n");

    Ok(out)
}

/// C の規格の名前から、cinrs のマクロの名前を決める。
///
/// cinrs のマクロは規格ごとに分かれていて、名前は `-std=` の値と同じ。対応する
/// マクロが無い値は、黙って別の規格で変換せずにエラーにする。
fn cinrs_macro(c_standard: &str) -> Result<&str> {
    const MACROS: &[&str] = &[
        "c89", "c99", "c11", "c17", "c23", "gnu89", "gnu99", "gnu11", "gnu17", "gnu23",
    ];

    if MACROS.contains(&c_standard) {
        Ok(c_standard)
    } else {
        Err(format!("C の規格 {c_standard} に対応する cinrs のマクロがありません").into())
    }
}

/// ライブラリ 1 つ分の、モジュールの宣言のファイルを組み立てる。
///
/// 除外した翻訳単位も含めて、すべてのモジュールを宣言する。
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

/// `src` にすでにあるファイルの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Existing {
    /// [`GENERATED_MARKER`] で始まる、このタスクが生成したファイル。
    Generated,

    /// それ以外。手で書いたファイルとして扱う。
    Handwritten,
}

/// `src` に書き出す内容と、手書きであるべきファイル。パスは `src` からの相対パス。
#[derive(Debug, Default)]
struct Layout {
    /// 生成するファイルと、その内容。
    generated: BTreeMap<PathBuf, String>,

    /// 除外した翻訳単位の、手書きの取り込みが置かれるべき場所。
    handwritten: BTreeSet<PathBuf>,

    /// 翻訳単位のファイルを置くディレクトリ（ライブラリの名前）。
    library_dirs: BTreeSet<PathBuf>,
}

impl Layout {
    fn build(manifest: &Manifest) -> Result<Self> {
        let units = manifest
            .translation_units
            .iter()
            .map(|path| Unit::parse(path))
            .collect::<Result<BTreeSet<_>>>()?;

        // 除外リストに、今の構成には無い翻訳単位が残っていないかを確かめる。
        // FFmpeg の更新や構成の変更でファイルが無くなった場合に、役目を終えた
        // 項目が残り続けるのを防ぐ。
        for excluded in EXCLUDED_UNITS {
            if !manifest
                .translation_units
                .iter()
                .any(|unit| unit == excluded)
            {
                return Err(format!(
                    "除外リストの {excluded} が、{MANIFEST_FILE} の翻訳単位にありません。\n\
                     不要になった項目であれば、xtask の EXCLUDED_UNITS から削除してください。"
                )
                .into());
            }
        }

        let mut layout = Self::default();
        let mut by_library: BTreeMap<&str, Vec<&Unit>> = BTreeMap::new();

        for unit in &units {
            by_library.entry(&unit.library).or_default().push(unit);

            if EXCLUDED_UNITS.contains(&unit.source_path().as_str()) {
                layout.handwritten.insert(unit.module_file());
            } else {
                layout
                    .generated
                    .insert(unit.module_file(), render_unit(manifest, unit)?);
            }
        }

        for (library, units) in by_library {
            layout.library_dirs.insert(PathBuf::from(library));
            layout.generated.insert(
                PathBuf::from(format!("{library}.rs")),
                render_library(library, &units),
            );
        }

        Ok(layout)
    }

    /// すでにあるファイルと突き合わせ、削除するファイルを決める。
    ///
    /// 手書きのファイルを壊しかねない状況と、ビルドが通らなくなる状況は、
    /// 何も書き出す前にエラーにする。
    fn plan(&self, existing: &BTreeMap<PathBuf, Existing>) -> Result<Vec<PathBuf>> {
        for path in self.generated.keys() {
            if existing.get(path) == Some(&Existing::Handwritten) {
                return Err(format!(
                    "src/{} は手書きのファイルなので、上書きしません。\n\
                     生成物に置き換えるなら削除し、手書きのまま使うなら、対応する \
                     翻訳単位を xtask の EXCLUDED_UNITS に追加してください。",
                    path.display(),
                )
                .into());
            }
        }

        for path in &self.handwritten {
            match existing.get(path) {
                Some(Existing::Handwritten) => {}

                Some(Existing::Generated) => {
                    return Err(format!(
                        "src/{} は除外リストにありますが、生成物の印が付いています。\n\
                         手書きの取り込みに置き換えてください。",
                        path.display(),
                    )
                    .into());
                }

                None => {
                    return Err(format!(
                        "src/{} がありません。\n\
                         除外リストにある翻訳単位は、手書きの取り込みを置く必要があります。",
                        path.display(),
                    )
                    .into());
                }
            }
        }

        let mut stale = Vec::new();

        for (path, kind) in existing {
            let expected = self.generated.contains_key(path) || self.handwritten.contains(path);

            match kind {
                // 一覧から消えた翻訳単位の生成物。残すと、宣言されないファイルが
                // 溜まっていく。
                Existing::Generated if !expected => stale.push(path.clone()),

                // ライブラリのディレクトリにある、宣言されない手書きのファイル。
                // モジュールの宣言は生成物の側にあるので、ここに置いても
                // コンパイルされない。置き忘れや、除外リストへの追加漏れを疑う。
                Existing::Handwritten
                    if !expected
                        && path
                            .parent()
                            .is_some_and(|dir| self.library_dirs.contains(dir)) =>
                {
                    return Err(format!(
                        "src/{} は、どの翻訳単位にも対応しない手書きのファイルです。\n\
                         除外リストへの追加漏れでなければ、別の場所に移してください。",
                        path.display(),
                    )
                    .into());
                }

                _ => {}
            }
        }

        Ok(stale)
    }
}

pub fn run() -> Result<()> {
    let sys_crate = sys_crate_dir()?;
    let src = sys_crate.join("src");

    let manifest_path = sys_crate.join(MANIFEST_FILE);
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "{} を読めません: {error}\n先に cargo xtask ffmpeg-manifest を実行してください。",
            manifest_path.display(),
        )
    })?;

    let manifest = Manifest::parse(&manifest_text)?;
    let layout = Layout::build(&manifest)?;

    check_library_declarations(&src, &layout)?;

    let mut existing = BTreeMap::new();
    scan_rust_files(&src, &src, &mut existing)?;

    let stale = layout.plan(&existing)?;

    let mut written = 0;

    for (path, content) in &layout.generated {
        let destination = src.join(path);

        // 内容が変わらないファイルには触れない。書き直すと更新時刻が変わり、
        // 何も変わっていないのに再ビルドが走る。
        if fs::read_to_string(&destination).is_ok_and(|current| current == *content) {
            continue;
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&destination, content)?;
        written += 1;
    }

    for path in &stale {
        fs::remove_file(src.join(path))?;
    }

    println!(
        "{} に生成しました。\n  \
         生成物 {} 個（うち更新 {} 個）、手書き {} 個、削除 {} 個",
        src.display(),
        layout.generated.len(),
        written,
        layout.handwritten.len(),
        stale.len(),
    );

    Ok(())
}

/// クレートのルートのファイル（`lib.rs`）が、各ライブラリのモジュールを宣言して
/// いることを確かめる。
///
/// `lib.rs` はクレートの説明を書く手書きのファイルなので、生成はしない。構成を
/// 変えてライブラリが増えたときに宣言を足し忘れると、生成したコードがまるごと
/// コンパイルされないまま、ビルドだけが通ってしまう。
fn check_library_declarations(src: &Path, layout: &Layout) -> Result<()> {
    let lib_rs = src.join("lib.rs");
    let content = fs::read_to_string(&lib_rs)
        .map_err(|error| format!("{} を読めません: {error}", lib_rs.display()))?;

    for library in &layout.library_dirs {
        let declaration = format!("pub mod {};", library.display());

        if !content.lines().any(|line| line.trim() == declaration) {
            return Err(format!(
                "{} に `{declaration}` がありません。追加してください。",
                lib_rs.display(),
            )
            .into());
        }
    }

    Ok(())
}

/// `dir` 以下の `.rs` を集め、生成物か手書きかを調べる。パスは `root` からの相対パス。
fn scan_rust_files(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Existing>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            scan_rust_files(root, &path, files)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            let kind = if fs::read_to_string(&path)?.starts_with(GENERATED_MARKER) {
                Existing::Generated
            } else {
                Existing::Handwritten
            };

            let relative = path.strip_prefix(root).expect("root 以下を走査している");
            files.insert(relative.to_owned(), kind);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(units: &[&str]) -> Manifest {
        Manifest {
            c_standard: "c17".to_owned(),
            defines: vec!["PIC".to_owned(), "_FILE_OFFSET_BITS=64".to_owned()],
            include_dirs: vec!["FFmpeg_generated".to_owned(), "FFmpeg".to_owned()],
            translation_units: units.iter().map(|unit| (*unit).to_owned()).collect(),
        }
    }

    fn existing(files: &[(&str, Existing)]) -> BTreeMap<PathBuf, Existing> {
        files
            .iter()
            .map(|(path, kind)| (PathBuf::from(path), *kind))
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

    /// 生成される取り込みの全体。並べる順に意味がある（プラットフォーム用の
    /// 回避策はインクルードディレクトリの前、FFmpeg 用の回避策はマクロ定義の後）
    /// ので、順番ごと固定している。
    #[test]
    fn renders_unit() {
        let rendered = render_unit(
            &manifest(&["libavutil/time.c"]),
            &Unit::parse("libavutil/time.c").unwrap(),
        )
        .unwrap();

        let expected = "\
// @generated by `cargo xtask ffmpeg-units`. 手で編集しない。
// 取り込みの形を変えるときは、xtask の `ffmpeg_units.rs` を変更して再生成する。

//! `libavutil/time.c` の取り込み。

cinrs::c17! {
    #pragma cinrs export
    #pragma cinrs system_include first

    #include \"../cinrs_platform_fixups.h\"

    #pragma cinrs include_path \"FFmpeg_generated\"
    #pragma cinrs include_path \"FFmpeg\"

    #define PIC 1
    #define _FILE_OFFSET_BITS 64

    #include \"../cinrs_ffmpeg_fixups.h\"

    #include \"libavutil/time.c\"
}
";

        assert_eq!(rendered, expected);
    }

    /// 対応するマクロが無い規格で、別の規格のマクロを使って変換しないこと。
    #[test]
    fn rejects_c_standard_without_cinrs_macro() {
        let mut manifest = manifest(&["libavutil/time.c"]);
        manifest.c_standard = "c2y".to_owned();

        assert!(render_unit(&manifest, &Unit::parse("libavutil/time.c").unwrap()).is_err());
    }

    /// 除外した翻訳単位は、取り込みのファイルを生成しないが、モジュールの宣言には
    /// 含める。宣言が無いと、手書きのファイルがコンパイルされない。
    #[test]
    fn declares_excluded_unit_without_generating_it() {
        let layout = Layout::build(&manifest(&["libavutil/log.c", "libavutil/time.c"])).unwrap();

        assert!(!layout.generated.contains_key(Path::new("libavutil/log.rs")));
        assert!(layout.handwritten.contains(Path::new("libavutil/log.rs")));
        assert!(
            layout
                .generated
                .contains_key(Path::new("libavutil/time.rs"))
        );

        let library = &layout.generated[Path::new("libavutil.rs")];
        assert!(library.contains("pub mod log;\n"));
        assert!(library.contains("pub mod time;\n"));
    }

    /// 除外リストの翻訳単位が一覧から消えたら、エラーにして気づかせる。
    /// 上流の修正を含むバージョンに上げた後などに、不要な項目を残さないため。
    #[test]
    fn rejects_excluded_unit_missing_from_manifest() {
        assert!(Layout::build(&manifest(&["libavutil/time.c"])).is_err());
    }

    /// 手書きのファイルは、どんな場合も上書きしない。生成を足す前に手で書いた
    /// 取り込みが残っている状態で実行しても、黙って消さないこと。
    #[test]
    fn refuses_to_overwrite_handwritten_file() {
        let layout = Layout::build(&manifest(&["libavutil/log.c", "libavutil/time.c"])).unwrap();

        let existing = existing(&[
            ("libavutil/log.rs", Existing::Handwritten),
            ("libavutil/time.rs", Existing::Handwritten),
        ]);

        assert!(layout.plan(&existing).is_err());
    }

    /// 除外した翻訳単位の手書きの取り込みが無いと、モジュールの宣言だけが
    /// 生成されてビルドが壊れる。何か書き出す前に止めること。
    #[test]
    fn requires_handwritten_file_for_excluded_unit() {
        let layout = Layout::build(&manifest(&["libavutil/log.c", "libavutil/time.c"])).unwrap();

        assert!(layout.plan(&existing(&[])).is_err());

        let marked = existing(&[("libavutil/log.rs", Existing::Generated)]);
        assert!(layout.plan(&marked).is_err());
    }

    /// 一覧から消えた翻訳単位の生成物は削除する。手書きのファイルと、ライブラリの
    /// ディレクトリの外にあるファイル（`lib.rs` など）には触れないこと。
    #[test]
    fn removes_only_stale_generated_files() {
        let layout = Layout::build(&manifest(&["libavutil/log.c", "libavutil/time.c"])).unwrap();

        let existing = existing(&[
            ("lib.rs", Existing::Handwritten),
            ("libavutil.rs", Existing::Generated),
            ("libavutil/log.rs", Existing::Handwritten),
            ("libavutil/time.rs", Existing::Generated),
            ("libavutil/removed.rs", Existing::Generated),
            ("libswscale.rs", Existing::Generated),
            ("libswscale/swscale.rs", Existing::Generated),
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

    /// ライブラリのディレクトリに、どの翻訳単位にも対応しない手書きのファイルが
    /// あれば止める。モジュールの宣言は生成物の側にあるので、そのファイルは
    /// コンパイルされない。除外リストへの追加漏れに気づくための検査。
    #[test]
    fn rejects_orphan_handwritten_file_in_library_dir() {
        let layout = Layout::build(&manifest(&["libavutil/log.c", "libavutil/time.c"])).unwrap();

        let existing = existing(&[
            ("libavutil/log.rs", Existing::Handwritten),
            ("libavutil/helper.rs", Existing::Handwritten),
        ]);

        assert!(layout.plan(&existing).is_err());
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
}
