mod app;
mod cli;
mod invocation;
mod logging;
pub mod pipeline;
mod plugin_config;
mod plugin_manager;
mod repl;
mod rows;
pub mod state;
mod theme_loader;
mod ui_presentation;
mod ui_sink;

use osp_ui::messages::{MessageBuffer, MessageLevel, adjust_verbosity};
use std::ffi::OsString;

pub use app::{classify_exit_code, render_report_message, run_from};
pub use cli::Cli;

pub fn run_process<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut sink = ui_sink::StdIoUiSink;
    run_process_with_sink(args, &mut sink)
}

pub(crate) fn run_process_with_sink<I, T>(args: I, sink: &mut dyn ui_sink::UiSink) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let message_verbosity = bootstrap_message_verbosity(&args);

    match app::run_from_with_sink(args, sink) {
        Ok(code) => code,
        Err(err) => {
            let mut messages = MessageBuffer::default();
            messages.error(render_report_message(&err, message_verbosity));
            sink.write_stderr(&messages.render_grouped(message_verbosity));
            classify_exit_code(&err)
        }
    }
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
    use super::{bootstrap_message_verbosity, run_process_with_sink};
    use crate::ui_sink::BufferedUiSink;
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

    #[test]
    fn run_process_with_sink_routes_top_level_errors_to_stderr_unit() {
        let mut sink = BufferedUiSink::default();

        let exit_code = run_process_with_sink(
            ["osp", "--theme", "missing-theme", "config", "show"],
            &mut sink,
        );

        assert_eq!(exit_code, 1);
        assert!(sink.stdout.is_empty());
        assert!(sink.stderr.contains("unknown theme"));
    }
}
