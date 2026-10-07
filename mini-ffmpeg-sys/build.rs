//! ビルドの前提を確かめ、cinrs に変換の対象のターゲットを伝える。

use std::{env, path::Path, process};

fn main() {
    // 指定しないと、パッケージ内のファイルが変わるたびに再実行される。
    println!("cargo:rerun-if-changed=build.rs");

    require_ffmpeg_tree();

    // cinrs は、`sizeof` や構造体のレイアウト、`#if` の値を、マクロの展開時に
    // ターゲットのデータモデルから決める。ところが proc-macro は rustc に
    // ターゲットを問い合わせられないので、クレートの側から環境変数で渡す。
    //
    // 渡さない場合、cinrs はホストのデータモデルで変換する。クロスコンパイルで
    // ターゲットと食い違うと、展開されたコードの assert でコンパイルが失敗する。
    let target = env::var("TARGET").expect("Cargo がビルドスクリプトに TARGET を設定する");
    println!("cargo:rustc-env=CINRS_TARGET={target}");
}

/// FFmpeg のツリーがあることを確かめ、無ければ取得の方法を示して止める。
///
/// ツリーは Git のサブモジュールなので、`--recurse-submodules` を付けずに
/// クローンした直後は空のディレクトリになっている。そのままビルドすると、
/// cinrs が「ファイルが見つからない」というエラーを翻訳単位の数だけ出し、
/// 原因が分かりにくい。先にここで止めて、何をすれば良いかを伝える。
///
/// crates.io から取得した場合は、ツリーがパッケージに同梱されているので、
/// この検査には掛からない。
fn require_ffmpeg_tree() {
    // どのファイルでも良いが、libavutil の中心のヘッダで代表させている。
    let marker = Path::new("FFmpeg/libavutil/avutil.h");

    if marker.is_file() {
        return;
    }

    println!(
        "cargo::error=FFmpeg のツリーがありません（{} が見つかりません）。\
         サブモジュールを取得してください: git submodule update --init",
        marker.display(),
    );

    process::exit(1);
}
