//! cinrs のマクロに、C のコードを変換する対象のターゲットを伝える。
//!
//! cinrs は、`sizeof` や構造体のレイアウト、`#if` の値を、マクロの展開時に
//! ターゲットのデータモデルから決める。ところが proc-macro は rustc にターゲットを
//! 問い合わせられないので、クレートの側から環境変数で渡す。
//!
//! 渡さない場合、cinrs はホストのデータモデルで変換する。クロスコンパイルで
//! ターゲットと食い違うと、展開されたコードの assert でコンパイルが失敗する。

use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let target = env::var("TARGET").expect("Cargo がビルドスクリプトに TARGET を設定する");
    println!("cargo:rustc-env=CINRS_TARGET={target}");
}
