//! C の翻訳単位 1 つを、cinrs で Rust のコードに変換する。
//!
//! cinrs のマクロ（`c17!` など）がコンパイル時に行う変換を、`cinrs-core` を直接
//! 呼んで開発時に行い、結果を文字列として返す。マクロに書いていた指定（プラグマ、
//! `#define`、インクルードディレクトリ）は、[`cinrs_core::Options`] の項目として
//! 渡す。対応は [`Settings::options`] にまとめてある。

use std::path::{Path, PathBuf};

use cinrs_core::{
    CommandLineMacro, Dialect, FileTranslation, Options, Standard, TargetModel, include::System,
};

/// 全翻訳単位に共通の、変換の指定。
#[derive(Debug, Clone)]
pub struct Settings {
    /// C の規格。`-std=` の値と同じ書き方（`c17`、`gnu17` など）。
    pub c_standard: String,

    /// 変換後のコードのコンパイル先。ターゲットトリプル。
    pub target: String,

    /// マクロ定義。`NAME` または `NAME=VALUE` の形で、`-D` に当たる。
    pub defines: Vec<String>,

    /// インクルードディレクトリ。探索する順で、`-I` に当たる。
    pub include_dirs: Vec<PathBuf>,
}

/// 翻訳単位 1 つを変換した結果。
#[derive(Debug)]
pub struct Translated {
    /// 整形済みの Rust のコード。翻訳単位のモジュールと、その中身の再エクスポート。
    pub rust: String,

    /// 翻訳単位が定義する C のシンボル。外部リンケージを持つ関数と変数の名前。
    pub symbols: Vec<String>,

    /// cinrs の警告。位置と説明を 1 件ずつ整形したもの。
    pub warnings: Vec<String>,
}

impl Settings {
    /// `source` の翻訳単位を変換する。
    ///
    /// `__FILE__` や診断に出るファイルの名前は、渡したパスと、インクルード
    /// ディレクトリのパスがそのまま使われる。生成物に絶対パスを残さないためには、
    /// どちらも相対パスで渡し、起点のディレクトリを作業ディレクトリにして呼ぶ。
    ///
    /// # Errors
    ///
    /// エラーが起きるのは、ファイルを読めない場合や、C のコードにエラーがある場合、
    /// 変換後のコードが cinrs のランタイムを必要とする場合。
    pub fn translate(&self, source: &Path) -> Result<Translated, String> {
        let translation = cinrs_core::translate_file(source, &self.options()?)?;

        finish(translation)
    }

    /// ファイルの代わりに、文字列の C のコードを変換する。
    #[cfg(test)]
    fn translate_source(&self, name: &str, text: &str) -> Result<Translated, String> {
        let translation =
            cinrs_core::translate_source(name.to_owned(), text.to_owned(), &self.options()?);

        finish(translation)
    }

    /// cinrs に渡す指定を組み立てる。
    ///
    /// 既定値は cinrs のマクロと同じなので、ここで設定するのは、マクロの中に
    /// 書いていた指定とマクロには無い指定だけ。cinrs には、C コンパイラと同じ
    /// コマンドラインで動く `ccinrs` もあり、あちらはさらに多くの項目を変えている
    /// （cinrs の `crates/ccinrs/src/driver.rs` の `options`）。cinrs を更新して
    /// 変換の結果が変わったときは、あちらで増えた項目が無いかを見比べると良い。
    fn options(&self) -> Result<Options, String> {
        let (standard, dialect) = parse_c_standard(&self.c_standard)?;

        // ターゲットを明示する。明示しない場合、cinrs は自身が動いているマシンか、
        // 環境変数 `CINRS_TARGET` からターゲットを決める。生成物の置き場所は
        // ターゲットごとに分けているので、実行した環境に左右されると、置き場所と
        // 中身が食い違う。
        let target = TargetModel::from_triple(&self.target).map_err(|error| {
            format!("ターゲット {} を cinrs が扱えません: {error}", self.target)
        })?;

        let mut options = Options::with_dialect(standard, dialect).for_target(target);

        options.include_paths = self.include_dirs.clone();

        // 外部リンケージを持つ関数と変数を、C の名前のシンボルにする。
        // 翻訳単位は互いを C のシンボルの名前で参照するので、
        // これが無いとリンクできない。
        options.export = true;

        // プラットフォームのヘッダを、cinrs 同梱のヘッダより先に探させる。
        //
        // 同梱のヘッダを先にすると両者が混ざり、macOS では `int64_t` の定義が
        // 食い違ってエラーになる。`config.h` はプラットフォームのヘッダを調べた
        // 結果なので、同じヘッダを読ませるのが筋でもある。
        options.system_include = System::First;

        // GCC が警告に留める規格違反を受け入れる。ISO の規格（`c17`）で変換する
        // 場合にだけ意味があり、GNU の方言（`gnu17`）では元から受け入れられる。
        //
        // 元のビルドは `-std=c17` を C コンパイラに渡していて、コンパイラはこれらを
        // エラーにしない。プラットフォームのヘッダにも該当する書き方がある。macOS
        // の `<mach_debug/ipc_info.h>` は、列挙子に `int` の範囲を超える値を使って
        // いて、`libavutil/random_seed.c` が `<mach/mach_time.h>` 経由で読む。
        options.gnu_leniencies = true;

        // 翻訳単位が定義するシンボルと、そこから辿って届く項目だけを出力する。
        //
        // C の `#include` は、ヘッダの宣言をすべて翻訳単位に持ち込む。cinrs の
        // 既定では、使われない宣言や `static inline` 関数、型もすべて Rust の
        // コードになり、今の構成では出力の約 4 分の 3 をそれが占める。
        //
        // 既定で無効なのは、マクロを置いた Rust のコードが、中の項目を名前で
        // 参照しうるため。取り込んだ翻訳単位は C のシンボルの名前で繋がるだけ
        // なので、リンカから見える定義を起点にして足りる。
        //
        // 引き換えに、翻訳単位が使わない型や宣言は、Rust から見えなくなる。
        options.reachable_only = true;

        // 複素数型（`_Complex`）を受け付けない。cinrs は複素数型の演算をランタイムの
        // クレート（`cinrs-rt`）の呼び出しに変換するので、受け付けると
        // `mini-ffmpeg-sys` が cinrs に依存することになる。切っておけば、変換後の
        // コードは `core` だけで動く。
        //
        // cinrs の `c17!` などでは有効になっている。切ると、C のマクロ
        // `__STDC_NO_COMPLEX__` が定義される。これが定義されているかどうかで分岐する
        // ヘッダは、`c17!` で変換していたときとは逆の分岐を読むことになる。
        options.complex = false;

        for define in &self.defines {
            options
                .macros
                .push(CommandLineMacro::Define(define.clone()));
        }

        for define in platform_workarounds(&self.target) {
            options
                .macros
                .push(CommandLineMacro::Define((*define).to_owned()));
        }

        Ok(options)
    }
}

/// C の規格の名前を、cinrs の規格と方言の組に直す。
///
/// 名前は `-std=` の値と同じで、`gnu` で始まるものが GNU の方言。対応が無い値は、
/// 黙って別の規格で変換せずにエラーにする。
fn parse_c_standard(name: &str) -> Result<(Standard, Dialect), String> {
    let (dialect, revision) = match name.strip_prefix("gnu") {
        Some(revision) => (Dialect::Gnu, revision),
        None => (Dialect::Iso, name.strip_prefix('c').unwrap_or_default()),
    };

    let standard = match revision {
        "89" => Standard::C89,
        "99" => Standard::C99,
        "11" => Standard::C11,
        "17" => Standard::C17,
        "23" => Standard::C23,
        _ => return Err(format!("C の規格 {name} を cinrs が扱えません")),
    };

    Ok((standard, dialect))
}

/// プラットフォームのヘッダを cinrs で読むための回避策。翻訳単位の先頭で定義する
/// マクロを、`NAME=VALUE` の形で返す。
///
/// プラットフォームのヘッダは、そのプラットフォームの C コンパイラ（macOS なら
/// Apple clang）に読まれる前提で書かれている。cinrs は GCC として振る舞うので、
/// ヘッダの期待と食い違う箇所が出ることがある。
///
/// 消すときは、その項目を外して `cargo xtask ffmpeg-units` が通ることを確かめる。
fn platform_workarounds(target: &str) -> &'static [&'static str] {
    if target.ends_with("-apple-darwin") {
        &[
            // SDK の `<math.h>` は、半精度の関数（`__fabsf16` など）を `_Float16` で
            // 無条件に宣言している。cinrs は `_Float16` を扱えず、宣言を読んんだ場合
            // にエラーとなる。
            //
            // FFmpeg はこれらの関数を呼ばないので、宣言を読み飛ばせれば十分。型を
            // `float` に置き換えて、宣言を通している。置き換えた宣言の型は実際の
            // 関数と食い違うが、呼ばれない宣言は出力されないので、Rust から誤って
            // 呼ぶこともできない。
            "_Float16=float",
        ]
    } else {
        &[]
    }
}

/// cinrs の変換の結果を確かめ、整形した Rust のコードにする。
fn finish(translation: FileTranslation) -> Result<Translated, String> {
    let diagnostics = &translation.diagnostics;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for diagnostic in diagnostics.sorted() {
        let rendered = diagnostics.render(&translation.map, diagnostic);

        match diagnostic.level {
            cinrs_core::Level::Error => errors.push(rendered),
            cinrs_core::Level::Warning => warnings.push(rendered),
        }
    }

    // ファイルを開けないなど、位置を持たないエラー。
    if let Some(message) = diagnostics.fatal_message() {
        errors.push(message.to_owned());
    }

    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }

    let items = translation
        .items
        .ok_or("cinrs がエラーを報告せずに、変換後のコードを返しませんでした")?;

    // 複素数型を切っているので、ランタイムを呼ぶコードは出ないはず。cinrs が別の
    // 用途でランタイムを使うようになった場合に、`mini-ffmpeg-sys` の依存を黙って
    // 増やさないための確認。
    if translation.uses_runtime {
        return Err(
            "変換後のコードが cinrs のランタイム（cinrs-rt）を必要とします。\n\
             mini-ffmpeg-sys に依存を足すかどうかを、先に決めてください。"
                .to_owned(),
        );
    }

    let file: syn::File = syn::parse2(items)
        .map_err(|error| format!("変換後のコードを Rust として構文解析できません: {error}"))?;

    Ok(Translated {
        rust: prettyplease::unparse(&file),
        symbols: translation.symbols,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            c_standard: "c17".to_owned(),
            target: "aarch64-apple-darwin".to_owned(),
            defines: Vec::new(),
            include_dirs: Vec::new(),
        }
    }

    #[test]
    fn parses_c_standard_names() {
        assert_eq!(
            parse_c_standard("c17").unwrap(),
            (Standard::C17, Dialect::Iso)
        );
        assert_eq!(
            parse_c_standard("gnu99").unwrap(),
            (Standard::C99, Dialect::Gnu)
        );
    }

    /// 対応が無い規格で、別の規格として変換しないこと。
    #[test]
    fn rejects_unknown_c_standard() {
        for name in ["c2y", "gnu", "17", ""] {
            assert!(parse_c_standard(name).is_err(), "{name}");
        }
    }

    /// 変換の指定のうち、リンクの成否と出力の大きさを決める 2 つが効いていること。
    ///
    /// - 外部リンケージを持つ関数は、C の名前のシンボルになる（`export`）。
    ///   これが外れると、翻訳単位どうしが互いを参照できず、リンクで失敗する。
    /// - 使われない `static` 関数と、使われない宣言は出力されない
    ///   （`reachable_only`）。`static` 関数でも、使われていれば残る。
    ///
    /// cinrs を更新したときに、項目の名前や既定値が変わって指定が効かなくなって
    /// いないかを、FFmpeg のツリー無しで確かめられる。
    #[test]
    fn exports_symbols_and_drops_unreachable_items() {
        let translated = settings()
            .translate_source(
                "sample.c",
                "int declared_only(int);\n\
                 static int unused_helper(void) { return 1; }\n\
                 static int used_helper(int value) { return value + 1; }\n\
                 int exported(int value) { return used_helper(value); }\n",
            )
            .unwrap();

        assert_eq!(translated.symbols, ["exported"]);
        assert!(translated.rust.contains("#[unsafe(no_mangle)]"));
        assert!(translated.rust.contains("fn exported("));
        assert!(translated.rust.contains("fn used_helper("));

        assert!(!translated.rust.contains("unused_helper"));
        assert!(!translated.rust.contains("declared_only"));
    }

    /// 一覧のマクロ定義が、翻訳単位の先頭で定義されること。値を省いた定義は、
    /// コンパイラの `-DNAME` と同じく 1 になる。
    ///
    /// 定義が効いていなければ `#error` で変換が失敗する。
    #[test]
    fn applies_defines() {
        let mut settings = settings();
        settings.defines = vec!["FLAG".to_owned(), "VALUE=40".to_owned()];

        let result = settings.translate_source(
            "sample.c",
            "#if FLAG != 1 || VALUE != 40\n\
             #error \"defines are not applied\"\n\
             #endif\n\
             int defined(void) { return 0; }\n",
        );

        assert!(result.is_ok(), "{result:?}");
    }

    /// C のコードのエラーは、位置と説明を付けて返す。変換後のコードを返さない。
    #[test]
    fn reports_c_errors() {
        let error = settings()
            .translate_source("sample.c", "int broken(void) { return missing; }\n")
            .unwrap_err();

        assert!(error.contains("sample.c:1"), "{error}");
        assert!(error.contains("missing"), "{error}");
    }

    /// 複素数型を受け付けると、変換後のコードが cinrs のランタイムを呼び、
    /// 生成物だけでビルドできなくなる。エラーとして止まること。
    #[test]
    fn rejects_complex_types() {
        let result = settings().translate_source(
            "sample.c",
            "double _Complex twice(double _Complex value) { return value * 2; }\n",
        );

        assert!(result.is_err());
    }
}
