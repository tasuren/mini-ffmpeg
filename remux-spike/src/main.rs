//! `remux-spike <入力> <出力>` で、入力のストリームを再エンコードせずに出力へ書く。

use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1).map(PathBuf::from);

    let (Some(input), Some(output), None) = (arguments.next(), arguments.next(), arguments.next())
    else {
        eprintln!("使い方: remux-spike <入力> <出力>");
        return ExitCode::FAILURE;
    };

    match remux_spike::remux(&input, &output) {
        0 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}
