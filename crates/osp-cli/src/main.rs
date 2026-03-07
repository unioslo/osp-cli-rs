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

        if value == "--" {
            break;
        }

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

#[cfg(test)]
mod tests {
    use super::bootstrap_message_verbosity;
    use osp_ui::messages::MessageLevel;
    use std::ffi::OsString;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn bootstrap_message_verbosity_counts_short_and_long_flags_until_double_dash() {
        let args = vec![
            OsString::from("osp"),
            OsString::from("-vvq"),
            OsString::from("--verbose"),
            OsString::from("--"),
            OsString::from("--quiet"),
        ];

        let level = bootstrap_message_verbosity(&args);
        assert_eq!(level, MessageLevel::Trace);
    }

    #[cfg(unix)]
    #[test]
    fn bootstrap_message_verbosity_ignores_non_utf8_and_balances_quiet_flags() {
        let args = vec![
            OsString::from("osp"),
            OsString::from("--quiet"),
            OsString::from_vec(vec![0x66, 0x6f, 0x80]),
            OsString::from("-q"),
        ];

        let level = bootstrap_message_verbosity(&args);
        assert_eq!(level, MessageLevel::Error);
    }
}
