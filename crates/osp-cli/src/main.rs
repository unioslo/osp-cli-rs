use osp_ui::messages::{MessageBuffer, MessageLevel, adjust_verbosity};
use std::ffi::OsString;

fn main() {
    let args = std::env::args_os().collect::<Vec<OsString>>();
    let message_verbosity = bootstrap_message_verbosity(&args);

    let exit_code = match osp_cli::run_from(args) {
        Ok(code) => code,
        Err(err) => {
            let mut messages = MessageBuffer::default();
            messages.error(osp_cli::render_report_message(&err, message_verbosity));
            eprint!("{}", messages.render_grouped(message_verbosity));
            osp_cli::classify_exit_code(&err)
        }
    };
    std::process::exit(exit_code);
}

fn bootstrap_message_verbosity(args: &[OsString]) -> MessageLevel {
    let mut verbose = 0u8;
    let mut quiet = 0u8;

    for token in args.iter().skip(1) {
        let Some(value) = token.to_str() else {
            continue;
        };

        match value {
            "--verbose" => {
                verbose = verbose.saturating_add(1);
                continue;
            }
            "--quiet" => {
                quiet = quiet.saturating_add(1);
                continue;
            }
            _ => {}
        }

        if value.starts_with('-') && !value.starts_with("--") {
            for ch in value.chars().skip(1) {
                match ch {
                    'v' => verbose = verbose.saturating_add(1),
                    'q' => quiet = quiet.saturating_add(1),
                    _ => {}
                }
            }
        }
    }

    adjust_verbosity(MessageLevel::Success, verbose, quiet)
}
