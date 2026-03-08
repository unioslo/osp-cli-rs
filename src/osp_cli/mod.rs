pub(crate) mod app;
pub(crate) mod cli;
pub(crate) mod invocation;
mod logging;
pub mod pipeline;
mod plugin_config;
pub(crate) mod plugin_manager;
mod repl;
mod rows;
pub mod state;
mod theme_loader;
mod ui_presentation;
mod ui_sink;

use crate::osp_ui::messages::{MessageBuffer, MessageLevel, adjust_verbosity};
use std::ffi::OsString;

pub use app::{classify_exit_code, render_report_message, run_from};
pub use cli::Cli;
pub use ui_sink::{BufferedUiSink, StdIoUiSink, UiSink};

/// Minimal host application object for embedding or composing the CLI.
#[derive(Debug, Default, Clone, Copy)]
pub struct App;

impl App {
    pub const fn new() -> Self {
        Self
    }

    pub fn run_from<I, T>(&self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        run_from(args)
    }

    pub fn with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        AppRunner { app: self, sink }
    }

    pub fn run_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        app::run_from_with_sink(args, sink)
    }

    pub fn run_process<I, T>(&self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        run_process(args)
    }

    pub fn run_process_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        run_process_with_sink(args, sink)
    }
}

/// Bound application handle that runs commands through one caller-owned sink.
pub struct AppRunner<'a> {
    app: App,
    sink: &'a mut dyn UiSink,
}

impl<'a> AppRunner<'a> {
    pub fn run_from<I, T>(&mut self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.app.run_with_sink(args, self.sink)
    }

    pub fn run_process<I, T>(&mut self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.app.run_process_with_sink(args, self.sink)
    }
}

/// Staged composition surface for the future single-crate foundation.
#[derive(Debug, Default, Clone, Copy)]
pub struct AppBuilder;

impl AppBuilder {
    pub const fn new() -> Self {
        Self
    }

    pub fn build(self) -> App {
        App::new()
    }

    pub fn build_with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        self.build().with_sink(sink)
    }
}

pub fn run_process<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut sink = ui_sink::StdIoUiSink;
    run_process_with_sink(args, &mut sink)
}

pub fn run_process_with_sink<I, T>(args: I, sink: &mut dyn UiSink) -> i32
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

pub use crate::row;

#[cfg(test)]
mod tests {
    use super::{App, AppBuilder, AppRunner, bootstrap_message_verbosity, run_process_with_sink};
    use crate::osp_cli::BufferedUiSink;
    use crate::osp_ui::messages::MessageLevel;
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

    #[test]
    fn app_builder_exposes_stable_host_surface_unit() {
        let app = AppBuilder::new().build();
        let direct = App::new();
        let mut sink = BufferedUiSink::default();

        let exit_code = app.run_process_with_sink(
            ["osp", "--theme", "missing-theme", "config", "show"],
            &mut sink,
        );

        assert_eq!(exit_code, 1);
        assert!(sink.stderr.contains("unknown theme"));

        let _ = direct;
    }

    #[test]
    fn app_runner_reuses_one_sink_across_invocations_unit() {
        let mut sink = BufferedUiSink::default();
        let mut runner: AppRunner<'_> = AppBuilder::new().build_with_sink(&mut sink);

        let exit_code = runner.run_process(["osp", "--theme", "missing-theme", "config", "show"]);

        assert_eq!(exit_code, 1);
        assert!(sink.stderr.contains("unknown theme"));
    }
}
