//! Interactive terminal services for the canonical UI pipeline.
//!
//! This module owns only runtime gating and prompt/spinner mechanics. Callers
//! decide whether a workflow should prompt; this module only answers whether
//! the current terminal can support that and provides the mechanics when it can.

use dialoguer::{Confirm, Password, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use std::env;
use std::io::{self, IsTerminal};
use std::time::Duration;

/// Runtime facts used to decide whether interactive UI is safe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractRuntime {
    pub stdin_is_tty: bool,
    pub stderr_is_tty: bool,
    pub terminal: Option<String>,
}

impl InteractRuntime {
    pub fn new(stdin_is_tty: bool, stderr_is_tty: bool, terminal: Option<String>) -> Self {
        Self {
            stdin_is_tty,
            stderr_is_tty,
            terminal,
        }
    }

    pub fn detect() -> Self {
        Self::new(
            io::stdin().is_terminal(),
            io::stderr().is_terminal(),
            env::var("TERM").ok(),
        )
    }

    pub fn allows_prompting(&self) -> bool {
        self.stdin_is_tty && self.stderr_is_tty
    }

    pub fn allows_live_output(&self) -> bool {
        self.stderr_is_tty && !matches!(self.terminal.as_deref(), Some("dumb"))
    }
}

pub type InteractResult<T> = io::Result<T>;

/// Prompt and transient-status service bound to an explicit runtime.
#[derive(Debug, Clone)]
pub struct Interact {
    runtime: InteractRuntime,
}

impl Default for Interact {
    fn default() -> Self {
        Self::detect()
    }
}

impl Interact {
    pub fn detect() -> Self {
        Self::new(InteractRuntime::detect())
    }

    pub fn new(runtime: InteractRuntime) -> Self {
        Self { runtime }
    }

    pub fn runtime(&self) -> &InteractRuntime {
        &self.runtime
    }

    pub fn confirm(&self, prompt: &str) -> InteractResult<bool> {
        self.confirm_default(prompt, false)
    }

    pub fn confirm_default(&self, prompt: &str, default: bool) -> InteractResult<bool> {
        self.require_prompting("confirmation prompt")?;
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .default(default)
            .interact()
            .map_err(io::Error::other)
    }

    pub fn password(&self, prompt: &str) -> InteractResult<String> {
        self.password_with_options(prompt, false)
    }

    pub fn password_allow_empty(&self, prompt: &str) -> InteractResult<String> {
        self.password_with_options(prompt, true)
    }

    fn password_with_options(&self, prompt: &str, allow_empty: bool) -> InteractResult<String> {
        self.require_prompting("password prompt")?;
        Password::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .allow_empty_password(allow_empty)
            .interact()
            .map_err(io::Error::other)
    }

    pub fn spinner(&self, message: impl Into<String>) -> Spinner {
        Spinner::with_runtime(&self.runtime, message)
    }

    fn require_prompting(&self, kind: &str) -> InteractResult<()> {
        if self.runtime.allows_prompting() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "{kind} requires an interactive terminal"
            )))
        }
    }
}

/// Handle for a transient spinner shown on stderr.
#[must_use]
pub struct Spinner {
    pb: ProgressBar,
}

impl Spinner {
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_runtime(&InteractRuntime::detect(), message)
    }

    pub fn with_runtime(runtime: &InteractRuntime, message: impl Into<String>) -> Self {
        Self::with_enabled(runtime.allows_live_output(), message)
    }

    pub fn set_message(&self, message: impl Into<String>) {
        self.pb.set_message(message.into());
    }

    pub fn suspend<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        self.pb.suspend(f)
    }

    pub fn finish_success(&self, message: impl Into<String>) {
        self.pb.finish_with_message(message.into());
    }

    pub fn finish_failure(&self, message: impl Into<String>) {
        self.pb.abandon_with_message(message.into());
    }

    pub fn finish_with_message(&self, message: impl Into<String>) {
        self.finish_success(message);
    }

    pub fn finish_and_clear(&self) {
        self.pb.finish_and_clear();
    }

    pub fn with_enabled(enabled: bool, message: impl Into<String>) -> Self {
        let pb = if enabled {
            let pb = ProgressBar::new_spinner();
            pb.enable_steady_tick(Duration::from_millis(120));
            pb.set_style(
                ProgressStyle::default_spinner()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
                    .template("{spinner:.cyan} {msg}")
                    .unwrap_or_else(|_| ProgressStyle::default_spinner()),
            );
            pb
        } else {
            ProgressBar::hidden()
        };
        pb.set_message(message.into());
        Self { pb }
    }
}

#[cfg(test)]
mod tests {
    use super::{Interact, InteractRuntime, Spinner};

    fn runtime(stdin_is_tty: bool, stderr_is_tty: bool, terminal: Option<&str>) -> InteractRuntime {
        InteractRuntime::new(stdin_is_tty, stderr_is_tty, terminal.map(str::to_string))
    }

    #[test]
    fn runtime_capability_matrix_covers_prompting_and_live_output_unit() {
        let cases = [
            (runtime(true, true, Some("xterm-256color")), true, true),
            (runtime(true, true, Some("dumb")), true, false),
            (runtime(false, true, Some("xterm-256color")), false, true),
            (runtime(true, false, Some("xterm-256color")), false, false),
            (runtime(true, true, None), true, true),
        ];

        for (runtime, allows_prompting, allows_live_output) in cases {
            assert_eq!(runtime.allows_prompting(), allows_prompting);
            assert_eq!(runtime.allows_live_output(), allows_live_output);
        }
    }

    #[test]
    fn hidden_spinner_supports_full_lifecycle_unit() {
        let spinner = Spinner::with_enabled(false, "Working");
        spinner.set_message("Still working");
        spinner.suspend(|| ());
        spinner.finish_success("Done");
        spinner.finish_failure("Failed");
        spinner.finish_and_clear();
    }

    #[test]
    fn spinner_respects_runtime_policy_unit() {
        let live = Spinner::with_runtime(&runtime(true, true, Some("xterm-256color")), "Working");
        live.set_message("Still working");
        live.finish_success("Done");

        let muted = Spinner::with_runtime(&runtime(true, true, Some("dumb")), "Muted");
        muted.finish_failure("Still muted");
    }

    #[test]
    fn interact_runtime_accessor_and_spinner_follow_runtime_unit() {
        let runtime = runtime(true, true, Some("xterm-256color"));
        let interact = Interact::new(runtime.clone());

        assert_eq!(interact.runtime(), &runtime);
        interact.spinner("Working").finish_and_clear();
    }

    #[test]
    fn prompting_helpers_fail_fast_without_interactive_terminal_unit() {
        let interact = Interact::new(runtime(false, false, None));

        for err in [
            interact
                .confirm_default("Proceed?", false)
                .expect_err("confirm should fail"),
            interact
                .confirm("Proceed?")
                .expect_err("confirm default-false should fail"),
            interact
                .password("Password")
                .expect_err("password should fail"),
            interact
                .password_allow_empty("Password")
                .expect_err("password should fail"),
        ] {
            assert!(
                err.to_string().contains("interactive terminal"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn detect_and_default_are_callable_unit() {
        let detected = Interact::detect();
        let defaulted = Interact::default();

        detected.spinner("Working").finish_and_clear();
        Spinner::new("Booting").finish_and_clear();

        assert_eq!(
            defaulted.runtime().stdin_is_tty,
            detected.runtime().stdin_is_tty
        );
        assert_eq!(
            defaulted.runtime().stderr_is_tty,
            detected.runtime().stderr_is_tty
        );
    }

    #[test]
    fn finish_with_message_alias_and_public_with_enabled_are_callable_unit() {
        let spinner = Spinner::with_enabled(false, "Working");
        spinner.finish_with_message("Done");
    }
}
