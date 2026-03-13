#![warn(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

//! Binary entrypoint for the `osp` command.

fn main() {
    std::process::exit(osp_cli::run_process(std::env::args_os()));
}
