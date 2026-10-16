//! C の翻訳単位 1 つを、cinrs で Rust のコードに変換する。
//!
//! cinrs のマクロ（`c17!` など）がコンパイル時に行う変換を、`cinrs-core` を直接
//! 呼んで行い、結果を文字列として返す。マクロに書く指定（プラグマ、`#define`、
//! インクルードディレクトリ）は、[`cinrs_core::Options`] の項目として渡す。対応は
//! [`Settings::options`] にまとめてある。
//!
//! マクロを使わないのは、翻訳単位が使う項目だけを出力する指定
//! （`Options::reachable_only`）が、マクロからは選べないため。
//!
//! 変換する翻訳単位は 2 種類ある（[`UnitKind`]）。FFmpeg の `.c` と、公開ヘッダを
//! include するだけの翻訳単位で、変換の指定が一部違う。

use std::path::{Path, PathBuf};

use cinrs_core::{
    CommandLineMacro, Dialect, FileTranslation, Level, Options, Standard, TargetModel,
    include::{HeaderKind, System},
};
use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

/// FFmpeg 自身のビルドだけが定義するマクロ。
///
/// FFmpeg のヘッダは、これが定義されていると `config.h` を読み、アーキテクチャ別の
/// 内部のヘッダ（`libavutil/bswap.h` の `arm/bswap.h` など）や、内部向けの
/// アサーションに切り替わる。FFmpeg を使う側のプログラムは定義しない。
const FFMPEG_INTERNAL_DEFINE: &str = "HAVE_AV_CONFIG_H";

/// 翻訳単位の種類。変換の指定のうち、種類で変わるものを決める。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    /// FFmpeg の `.c`。関数と変数の定義を持ち、C のシンボルとして出力する。
    ///
    /// 使われない項目は出力しない。Rust のコードからは名前で参照せず、他の
    /// 翻訳単位とは C のシンボルの名前で繋がる。
    Implementation,

    /// 公開ヘッダを include するだけの翻訳単位。利用者に見せる関数の宣言、型、
    /// 列挙子を、1 つの名前空間に揃える。
    ///
    /// 定義を持たないので、シンボルは出力しない。宣言は、リンクのときに
    /// [`UnitKind::Implementation`] の翻訳単位の定義に繋がる。FFmpeg を使う側の
    /// プログラムと同じ立場で読むので、[`FFMPEG_INTERNAL_DEFINE`] を定義しない。
    PublicApi,
}

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
    /// 変換後の Rust のコード。翻訳単位のモジュールと、その中身の再エクスポート。
    ///
    /// 人が読むための整形はしていない。字下げは無く、文と波括弧の区切りで
    /// 改行しているだけ。
    pub rust: String,

    /// 翻訳単位が定義する C のシンボル。外部リンケージを持つ関数と変数の名前。
    pub symbols: Vec<String>,

    /// 翻訳単位が宣言だけをしている、外部の関数と変数の C の名前。変換後の
    /// コードの `extern` のブロックにあるもの。
    pub declarations: Vec<String>,

    /// 翻訳単位が読んだヘッダのパス。cinrs 同梱のヘッダは含まない。
    ///
    /// FFmpeg のヘッダは渡したインクルードディレクトリからの相対パス、
    /// プラットフォームのヘッダは絶対パスになる。
    pub headers: Vec<String>,

    /// cinrs の警告。位置と説明を 1 件ずつ整形したもの。
    pub warnings: Vec<String>,
}

impl Settings {
    /// `source` の翻訳単位を変換する。
    ///
    /// `__FILE__` や診断に出るファイルの名前は、渡したパスと、インクルード
    /// ディレクトリのパスがそのまま使われる。FFmpeg は `av_assert0` などで
    /// `__FILE__` を使うので、絶対パスで渡すと、ビルドしたマシンのパスが実行
    /// ファイルに入る。どちらも相対パスで渡し、起点のディレクトリを作業
    /// ディレクトリにして呼ぶ。
    ///
    /// # Errors
    ///
    /// ファイルを読めない場合、C のコードにエラーがある場合、変換後のコードが
    /// cinrs のランタイムを必要とする場合。
    pub fn translate(&self, source: &Path) -> Result<Translated, String> {
        let options = self.options(UnitKind::Implementation)?;

        finish(cinrs_core::translate_file(source, &options)?)
    }

    /// 公開ヘッダを include するだけの翻訳単位を変換する。
    ///
    /// `name` は、ファイルの代わりに渡す C のコード（`source`）の名前で、診断と
    /// `__FILE__` に使われる。[`Settings::translate`] と同じ理由で、相対パスの
    /// 形にする。
    ///
    /// # Errors
    ///
    /// [`Settings::translate`] と同じ。
    pub fn translate_public_api(&self, name: &str, source: &str) -> Result<Translated, String> {
        let options = self.options(UnitKind::PublicApi)?;

        finish(cinrs_core::translate_source(
            name.to_owned(),
            source.to_owned(),
            &options,
        ))
    }

    /// cinrs に渡す指定を組み立てる。
    ///
    /// 既定値は cinrs のマクロと同じなので、ここで設定するのは、マクロの中に
    /// 書く指定と、マクロには無い指定だけ。cinrs には、C コンパイラと同じ
    /// コマンドラインで動く `ccinrs` もあり、あちらはさらに多くの項目を変えている
    /// （cinrs の `crates/ccinrs/src/driver.rs` の `options`）。cinrs を更新して
    /// 変換の結果が変わったときは、あちらで増えた項目が無いかを見比べると良い。
    fn options(&self, kind: UnitKind) -> Result<Options, String> {
        let (standard, dialect) = parse_c_standard(&self.c_standard)?;

        // ターゲットを明示する。明示しない場合、cinrs は自身が動いているマシンか、
        // 環境変数 `CINRS_TARGET` からターゲットを決める。ビルド先と食い違うと、
        // 変換後のコードの assert でコンパイルが失敗する。
        let target = TargetModel::from_triple(&self.target).map_err(|error| {
            format!("ターゲット {} を cinrs が扱えません: {error}", self.target)
        })?;

        let mut options = Options::with_dialect(standard, dialect).for_target(target);

        options.include_paths = self.include_dirs.clone();

        // 外部リンケージを持つ関数と変数を、C の名前のシンボルにする。マクロの
        // `#pragma cinrs export` に当たる。翻訳単位は互いを C のシンボルの名前で
        // 参照するので、これが無いとリンクできない。
        //
        // 公開 API の翻訳単位は定義を持たないので、要らない。ヘッダが誤って定義を
        // 持っていた場合に、実装の側と同じシンボルを重ねて出力しないためでもある。
        options.export = kind == UnitKind::Implementation;

        // プラットフォームのヘッダを、cinrs 同梱のヘッダより先に探させる。マクロの
        // `#pragma cinrs system_include first` に当たる。
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
        // コードになり、今の構成では出力の約 4 分の 3 をそれが占める。その分だけ
        // rustc の処理が増え、ビルドが 1 分以上長くなる。
        //
        // 既定で無効なのは、マクロの周りの Rust のコードが、中の項目を名前で
        // 参照しうるため。取り込んだ翻訳単位は C のシンボルの名前で繋がるだけ
        // なので、リンカから見える定義を起点にして足りる。
        //
        // 引き換えに、翻訳単位が使わない型や宣言は、Rust から見えなくなる。
        //
        // 公開 API の翻訳単位では、これを切る。宣言と型を見せることが目的で、
        // しかも定義を 1 つも持たないので、これが有効だと絞り込みで何も残らない。
        options.reachable_only = kind == UnitKind::Implementation;

        // 複素数型（`_Complex`）を受け付けない。cinrs は複素数型の演算をランタイムの
        // クレート（`cinrs-rt`）の呼び出しに変換するので、受け付けると
        // `mini-ffmpeg-sys` の実行時の依存が増える。切っておけば、変換後の
        // コードは `core` だけで動く。
        //
        // cinrs のマクロでは有効になっている。切ると `__STDC_NO_COMPLEX__` が
        // 定義されるので、このマクロを見て分岐するヘッダは、マクロのときと違う側を
        // 読む。FFmpeg が複素数型を使うようになったら、ここでエラーになる。
        options.complex = false;

        for define in &self.defines {
            let name = define
                .split_once('=')
                .map_or(define.as_str(), |(name, _)| name);

            if kind == UnitKind::PublicApi && name == FFMPEG_INTERNAL_DEFINE {
                continue;
            }

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

/// cinrs の変換の結果を確かめ、ファイルに書き出せる形にする。
///
/// # Errors
///
/// C のコードにエラーがある場合と、変換後のコードが cinrs のランタイムを必要と
/// する場合。
fn finish(translation: FileTranslation) -> Result<Translated, String> {
    let diagnostics = &translation.diagnostics;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    for diagnostic in diagnostics.sorted() {
        let rendered = diagnostics.render(&translation.map, diagnostic);

        match diagnostic.level {
            Level::Error => errors.push(rendered),
            Level::Warning => warnings.push(rendered),
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

    // 複素数型を切っているので、ランタイムを呼ぶコードは出ないはず。cinrs が
    // 別の用途でランタイムを使うようになった場合に、依存に無いクレートを
    // 参照するコードを黙って出力しないための確認。
    if translation.uses_runtime {
        return Err(
            "変換後のコードが cinrs のランタイム（cinrs-rt）を必要とします。\n\
             mini-ffmpeg-sys に依存を足すかどうかを、先に決めてください。"
                .to_owned(),
        );
    }

    let headers = translation
        .headers
        .iter()
        .filter(|header| !matches!(header.kind, HeaderKind::Bundled))
        .map(|header| header.name.clone())
        .collect();

    let mut declarations = Vec::new();
    collect_declarations(items.clone(), &mut declarations);

    let mut writer = TokenWriter::default();
    writer.write(items);

    Ok(Translated {
        rust: writer.out,
        symbols: translation.symbols,
        declarations,
        headers,
        warnings,
    })
}

/// 変換後のコードから、`extern` のブロックにある関数と変数の C の名前を集める。
///
/// cinrs は、宣言だけの関数と変数を `unsafe extern "C" { … }` のブロックに並べる。
/// ここでは、`extern` と ABI の文字列に続く波括弧のブロックを探し、その中の
/// `fn` と `static` の名前を拾う。モジュールの中などに入れ子になっていても、
/// 波括弧を辿って探す。
///
/// Rust の名前と C の名前が違う宣言には、`#[link_name = "…"]` が付く。その場合は
/// 属性の値を C の名前とする。
fn collect_declarations(tokens: TokenStream, out: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();

    for (index, token) in tokens.iter().enumerate() {
        let TokenTree::Group(group) = token else {
            continue;
        };

        let is_extern_block = group.delimiter() == Delimiter::Brace
            && index >= 2
            && matches!(&tokens[index - 2], TokenTree::Ident(ident) if ident == "extern")
            && matches!(&tokens[index - 1], TokenTree::Literal(_));

        if is_extern_block {
            collect_extern_items(group.stream(), out);
        } else {
            collect_declarations(group.stream(), out);
        }
    }
}

/// `extern` のブロックの中身から、宣言された関数と変数の C の名前を集める。
fn collect_extern_items(tokens: TokenStream, out: &mut Vec<String>) {
    let mut link_name = None;
    let mut tokens = tokens.into_iter().peekable();

    while let Some(token) = tokens.next() {
        match token {
            // 属性。`link_name` であれば、次の宣言の C の名前として覚えておく。
            TokenTree::Group(group) if group.delimiter() == Delimiter::Bracket => {
                if let Some(name) = find_link_name(group.stream()) {
                    link_name = Some(name);
                }
            }

            TokenTree::Ident(ident) if ident == "fn" || ident == "static" => {
                // `static mut name` の `mut` を飛ばす。
                if matches!(tokens.peek(), Some(TokenTree::Ident(next)) if next == "mut") {
                    tokens.next();
                }

                if let Some(TokenTree::Ident(name)) = tokens.next() {
                    out.push(link_name.take().unwrap_or_else(|| name.to_string()));
                }
            }

            _ => {}
        }
    }
}

/// 属性の中身（`link_name = "…"`）から、名前の文字列を取り出す。
///
/// 値は文字列リテラルのトークンで、前後の引用符だけを外す。エスケープは解釈
/// しない。エスケープを含むのは、プラットフォームのヘッダの `__asm` のラベル
/// （`"\u{1}_nanosleep"`）くらいで、FFmpeg の名前には現れない。
fn find_link_name(tokens: TokenStream) -> Option<String> {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();

    match tokens.as_slice() {
        [
            TokenTree::Ident(key),
            TokenTree::Punct(equals),
            TokenTree::Literal(value),
        ] if key == "link_name" && equals.as_char() == '=' => {
            let value = value.to_string();
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .map(str::to_owned)
        }

        // `#[unsafe(link_name = "…")]` の形にも備える。
        [TokenTree::Ident(word), TokenTree::Group(inner)] if word == "unsafe" => {
            find_link_name(inner.stream())
        }

        _ => None,
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
/// どの項目も cinrs の側で直れば消せる。消すときは、その項目を外して
/// `cargo test -p mini-ffmpeg-sys` が通ることを確かめる。
fn platform_workarounds(target: &str) -> &'static [&'static str] {
    if target.ends_with("-apple-darwin") {
        &[
            // SDK の `<math.h>` は、半精度の関数（`__fabsf16` など）を `_Float16` で
            // 無条件に宣言している。cinrs は `_Float16` を扱えず、宣言を読んだ
            // だけでエラーにする。
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

/// 1 行がこの長さを超えたら、次の `,` で改行する。配列の初期化子のように、
/// 文や波括弧の区切りが無いまま続く箇所のため。
const LINE_WIDTH: usize = 100;

/// トークンの並びを、適度に改行しながら文字列にする。
///
/// cinrs が返すのはトークンの並びで、そのまま文字列にすると、翻訳単位の全体が
/// 1 行になる。Rust としては正しいが、rustc は極端に長い行の処理が遅く、
/// `mini-ffmpeg-sys` のコンパイルが約 6 秒から約 33 秒に延びた。エラーや panic の
/// 位置も、すべて 1 行目になってしまう。
///
/// 整形のクレート（`syn` と `prettyplease`）を使えば読みやすくなるが、出力は
/// `OUT_DIR` にあって人が読むものではないので、依存を増やさずに、改行だけを
/// 入れている。改行するのは、`;` の後、波括弧の内側と後、長い行の `,` の後。
/// トークンとトークンの間に改行を足すだけなので、コードの意味は変わらない。
/// 文字列リテラルは 1 つのトークンで、その中身には触れない。
#[derive(Default)]
struct TokenWriter {
    out: String,

    /// 今の行に書いた文字数。
    line_width: usize,
}

impl TokenWriter {
    fn write(&mut self, tokens: TokenStream) {
        for token in tokens {
            match token {
                TokenTree::Group(group) => {
                    let (open, close, is_block) = match group.delimiter() {
                        Delimiter::Brace => ("{", "}", true),
                        Delimiter::Parenthesis => ("(", ")", false),
                        Delimiter::Bracket => ("[", "]", false),
                        Delimiter::None => ("", "", false),
                    };

                    self.push(open);
                    self.separate(is_block);

                    self.write(group.stream());

                    self.push(close);
                    self.separate(is_block);
                }

                TokenTree::Punct(punct) => {
                    let character = punct.as_char();
                    self.push(character.encode_utf8(&mut [0; 4]));

                    // `Joint` は、次の記号と合わせて 1 つの演算子になる印（`::` や
                    // `->`、ライフタイムの `'`）。間に空白や改行を入れると、別の
                    // トークンになってしまう。
                    if punct.spacing() == Spacing::Alone {
                        let is_long_line = self.line_width > LINE_WIDTH;
                        self.separate(character == ';' || (character == ',' && is_long_line));
                    }
                }

                TokenTree::Ident(ident) => {
                    self.push(&ident.to_string());
                    self.separate(false);
                }

                TokenTree::Literal(literal) => {
                    self.push(&literal.to_string());
                    self.separate(false);
                }
            }
        }
    }

    fn push(&mut self, text: &str) {
        self.out.push_str(text);
        self.line_width += text.len();
    }

    /// トークンの区切りを書く。`break_line` なら改行、そうでなければ空白。
    fn separate(&mut self, break_line: bool) {
        if break_line {
            self.out.push('\n');
            self.line_width = 0;
        } else {
            self.out.push(' ');
            self.line_width += 1;
        }
    }
}
