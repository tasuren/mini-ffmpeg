//! `remux-spike <input> <output>` writes input streams to the output without
//! re-encoding.

use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    let mut arguments = env::args_os().skip(1).map(PathBuf::from);

    let (Some(input), Some(output), None) = (arguments.next(), arguments.next(), arguments.next())
    else {
        eprintln!("Usage: remux-spike <input> <output>");
        return ExitCode::FAILURE;
    };

    match remux_spike::remux(&input, &output) {
        0 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}
