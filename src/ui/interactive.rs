//! Interactive terminal helpers for prompts and transient status UI.
//!
//! This module exists to keep blocking prompts and live terminal widgets behind
//! a small runtime-aware surface. Callers decide whether prompting is part of
//! their workflow; this module only answers whether the current terminal makes
//! that safe and provides the mechanics when it does.
//!
//! Contract:
//!
//! - this module owns prompt/runtime gating and spinner mechanics
//! - it should not absorb command policy, validation rules, or higher-level
//!   workflow decisions
//!
//! Public API shape:
//!
//! - [`InteractiveRuntime`] uses the crate-wide constructor/factory naming:
//!   `new(...)` for exact runtime hints and `detect()` for process probing
//! - [`Interactive`] is a lightweight wrapper over those runtime hints rather
//!   than another place to encode workflow policy

use dialoguer::{Confirm, Password, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use std::env;
use std::io::{self, IsTerminal};
use std::time::Duration;

/// Interactive runtime hints used to decide whether live terminal UI is safe.
///
/// This mirrors the render/runtime split elsewhere in `osp-ui`: callers can
/// inject explicit values for tests or special hosts, while `detect()` remains
/// the boring default for normal CLI entrypoints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InteractiveRuntime {
    /// Whether stdin is attached to a terminal.
    pub stdin_is_tty: bool,
    /// Whether stderr is attached to a terminal.
    pub stderr_is_tty: bool,
    /// Detected terminal identifier such as `xterm-256color`.
    pub terminal: Option<String>,
}

impl InteractiveRuntime {
    /// Creates explicit interactive runtime hints.
    pub fn new(stdin_is_tty: bool, stderr_is_tty: bool, terminal: Option<String>) -> Self {
        Self {
            stdin_is_tty,
            stderr_is_tty,
            terminal,
        }
    }

    /// Detect interactive terminal capabilities from the current process.
    pub fn detect() -> Self {
        Self::new(
            io::stdin().is_terminal(),
            io::stderr().is_terminal(),
            env::var("TERM").ok(),
        )
    }

    /// Returns `true` when both stdin and stderr can support interactive
    /// prompts.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::InteractiveRuntime;
    ///
    /// let runtime = InteractiveRuntime::new(
    ///     true,
    ///     true,
    ///     Some("xterm-256color".to_string()),
    /// );
    ///
    /// assert!(runtime.allows_prompting());
    /// ```
    pub fn allows_prompting(&self) -> bool {
        self.stdin_is_tty && self.stderr_is_tty
    }

    /// Returns `true` when transient terminal output such as spinners is safe
    /// to show.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::InteractiveRuntime;
    ///
    /// let runtime = InteractiveRuntime::new(true, true, Some("dumb".to_string()));
    ///
    /// assert!(!runtime.allows_live_output());
    /// ```
    pub fn allows_live_output(&self) -> bool {
        self.stderr_is_tty && !matches!(self.terminal.as_deref(), Some("dumb"))
    }
}

/// Result type used by interactive prompt helpers.
pub type InteractiveResult<T> = io::Result<T>;

#[derive(Debug, Clone)]
/// Interactive prompt helper bound to a detected or injected terminal runtime.
pub struct Interactive {
    runtime: InteractiveRuntime,
}

impl Default for Interactive {
    fn default() -> Self {
        Self::detect()
    }
}

impl Interactive {
    /// Create an interaction helper from the current process runtime.
    pub fn detect() -> Self {
        Self::new(InteractiveRuntime::detect())
    }

    /// Creates an interaction helper from an explicit runtime description.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::{Interactive, InteractiveRuntime};
    ///
    /// let ui = Interactive::new(InteractiveRuntime::new(
    ///     true,
    ///     false,
    ///     Some("xterm-256color".to_string()),
    /// ));
    ///
    /// assert!(!ui.runtime().allows_prompting());
    /// ```
    pub fn new(runtime: InteractiveRuntime) -> Self {
        Self { runtime }
    }

    /// Returns the runtime hints used by this helper.
    pub fn runtime(&self) -> &InteractiveRuntime {
        &self.runtime
    }

    /// Prompts for confirmation with a default answer of `false`.
    pub fn confirm(&self, prompt: &str) -> InteractiveResult<bool> {
        self.confirm_default(prompt, false)
    }

    /// Prompt for a yes/no answer without baking business policy into the UI.
    pub fn confirm_default(&self, prompt: &str, default: bool) -> InteractiveResult<bool> {
        self.require_prompting("confirmation prompt")?;
        Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .default(default)
            .interact()
            .map_err(io::Error::other)
    }

    /// Prompt for a secret value. Blank handling is a caller policy.
    pub fn password(&self, prompt: &str) -> InteractiveResult<String> {
        self.password_with_options(prompt, false)
    }

    /// Prompts for a secret value and permits empty input.
    pub fn password_allow_empty(&self, prompt: &str) -> InteractiveResult<String> {
        self.password_with_options(prompt, true)
    }

    /// Creates a spinner that follows the current runtime policy.
    pub fn spinner(&self, message: impl Into<String>) -> Spinner {
        Spinner::with_runtime(&self.runtime, message)
    }

    fn password_with_options(&self, prompt: &str, allow_empty: bool) -> InteractiveResult<String> {
        self.require_prompting("password prompt")?;
        Password::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .allow_empty_password(allow_empty)
            .interact()
            .map_err(io::Error::other)
    }

    fn require_prompting(&self, kind: &str) -> InteractiveResult<()> {
        if self.runtime.allows_prompting() {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "{kind} requires an interactive terminal"
        )))
    }
}

/// Handle for a transient spinner shown on stderr.
#[must_use]
pub struct Spinner {
    pb: ProgressBar,
}

impl Spinner {
    /// Convenience constructor for normal CLI entrypoints.
    ///
    /// Hosts that already resolved runtime policy should prefer
    /// `Spinner::with_runtime(...)`.
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_runtime(&InteractiveRuntime::detect(), message)
    }

    /// Build a spinner that respects explicit runtime hints.
    pub fn with_runtime(runtime: &InteractiveRuntime, message: impl Into<String>) -> Self {
        Self::with_enabled(runtime.allows_live_output(), message)
    }

    /// Build either a live spinner or a hidden no-op handle.
    ///
    /// Hidden spinners let callers keep the same lifecycle code path in tests,
    /// pipes, and dumb terminals without branching on every update.
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

    /// Updates the spinner message.
    pub fn set_message(&self, message: impl Into<String>) {
        self.pb.set_message(message.into());
    }

    /// Runs `f` while temporarily suspending the spinner from the terminal.
    pub fn suspend<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        self.pb.suspend(f)
    }

    /// Marks the spinner as successfully finished and leaves the final message visible.
    pub fn finish_success(&self, message: impl Into<String>) {
        self.pb.finish_with_message(message.into());
    }

    /// Marks the spinner as failed and leaves the final message visible.
    pub fn finish_failure(&self, message: impl Into<String>) {
        self.pb.abandon_with_message(message.into());
    }

    /// Backward-compatible success alias matching `indicatif` naming.
    ///
    /// New callers should prefer [`Spinner::finish_success`] so the public API
    /// makes the success/failure distinction explicit.
    pub fn finish_with_message(&self, message: impl Into<String>) {
        self.finish_success(message);
    }

    /// Finishes the spinner and clears it from the terminal.
    pub fn finish_and_clear(&self) {
        self.pb.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{Interactive, InteractiveRuntime, Spinner};

    fn runtime(
        stdin_is_tty: bool,
        stderr_is_tty: bool,
        terminal: Option<&str>,
    ) -> InteractiveRuntime {
        InteractiveRuntime::new(stdin_is_tty, stderr_is_tty, terminal.map(str::to_string))
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
    fn hidden_spinner_supports_full_lifecycle() {
        let spinner = Spinner::with_enabled(false, "Working");
        spinner.set_message("Still working");
        spinner.suspend(|| ());
        spinner.finish_success("Done");
        spinner.finish_failure("Failed");
        spinner.finish_and_clear();
    }

    #[test]
    fn spinner_respects_runtime_policy_and_finish_alias() {
        let live_runtime = runtime(true, true, Some("xterm-256color"));
        let muted_runtime = runtime(true, true, Some("dumb"));

        let live = Spinner::with_runtime(&live_runtime, "Working");
        live.set_message("Still working");
        live.finish_with_message("Done");

        let muted = Spinner::with_runtime(&muted_runtime, "Muted");
        muted.finish_with_message("Still muted");
    }

    #[test]
    fn interactive_runtime_accessor_and_spinner_follow_runtime() {
        let runtime = runtime(true, true, Some("xterm-256color"));
        let interactive = Interactive::new(runtime.clone());

        assert_eq!(interactive.runtime(), &runtime);
        interactive.spinner("Working").finish_and_clear();
    }

    #[test]
    fn prompting_helpers_fail_fast_without_interactive_terminal_unit() {
        let interactive = Interactive::new(runtime(false, false, None));

        for err in [
            interactive
                .confirm("Proceed?")
                .expect_err("confirm should fail"),
            interactive
                .password("Password")
                .expect_err("password should fail"),
            interactive
                .password_allow_empty("Password")
                .expect_err("password prompt should still require a TTY"),
        ] {
            assert!(
                err.to_string().contains("interactive terminal"),
                "unexpected error: {err}"
            );
        }
    }

    #[test]
    fn spinner_new_and_detect_paths_are_callable() {
        let interactive = Interactive::detect();
        interactive.spinner("Working").finish_and_clear();
        Spinner::new("Booting").finish_and_clear();
    }

    #[test]
    fn default_interactive_matches_detected_runtime_shape() {
        let detected = Interactive::detect();
        let defaulted = Interactive::default();

        assert_eq!(
            defaulted.runtime().stdin_is_tty,
            detected.runtime().stdin_is_tty
        );
        assert_eq!(
            defaulted.runtime().stderr_is_tty,
            detected.runtime().stderr_is_tty
        );
    }
}
