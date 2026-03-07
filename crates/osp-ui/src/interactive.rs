use dialoguer::{Confirm, Password, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};
use std::env;
use std::io::{self, IsTerminal};
use std::time::Duration;

/// Blocking prompt helpers and transient status UI for interactive CLI hosts.
///
/// This module is intentionally small. It is not a second rendering system and
/// it should not own command-level policy. Callers decide when prompting is
/// appropriate, whether blank input is valid, and when a spinner should be
/// shown; `osp-ui` only provides the terminal primitives.
/// Interactive runtime hints used to decide whether live terminal UI is safe.
///
/// This mirrors the render/runtime split elsewhere in `osp-ui`: callers can
/// inject explicit values for tests or special hosts, while `detect()` remains
/// the boring default for normal CLI entrypoints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractiveRuntime {
    pub stdin_is_tty: bool,
    pub stderr_is_tty: bool,
    pub terminal: Option<String>,
}

impl InteractiveRuntime {
    /// Detect interactive terminal capabilities from the current process.
    pub fn detect() -> Self {
        Self {
            stdin_is_tty: io::stdin().is_terminal(),
            stderr_is_tty: io::stderr().is_terminal(),
            terminal: env::var("TERM").ok(),
        }
    }

    pub fn allows_prompting(&self) -> bool {
        self.stdin_is_tty && self.stderr_is_tty
    }

    pub fn allows_live_output(&self) -> bool {
        self.stderr_is_tty && !matches!(self.terminal.as_deref(), Some("dumb"))
    }
}

pub type InteractiveResult<T> = io::Result<T>;

#[derive(Debug, Clone)]
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

    pub fn new(runtime: InteractiveRuntime) -> Self {
        Self { runtime }
    }

    pub fn runtime(&self) -> &InteractiveRuntime {
        &self.runtime
    }

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

    pub fn password_allow_empty(&self, prompt: &str) -> InteractiveResult<String> {
        self.password_with_options(prompt, true)
    }

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
}

#[cfg(test)]
mod tests {
    use super::{Interactive, InteractiveRuntime, Spinner};

    #[test]
    fn runtime_blocks_live_output_for_dumb_term() {
        let runtime = InteractiveRuntime {
            stdin_is_tty: true,
            stderr_is_tty: true,
            terminal: Some("dumb".to_string()),
        };

        assert!(!runtime.allows_live_output());
        assert!(runtime.allows_prompting());
    }

    #[test]
    fn runtime_blocks_prompting_without_ttys() {
        let runtime = InteractiveRuntime {
            stdin_is_tty: false,
            stderr_is_tty: true,
            terminal: Some("xterm-256color".to_string()),
        };

        assert!(!runtime.allows_prompting());
    }

    #[test]
    fn runtime_blocks_live_output_without_stderr_tty() {
        let runtime = InteractiveRuntime {
            stdin_is_tty: true,
            stderr_is_tty: false,
            terminal: Some("xterm-256color".to_string()),
        };

        assert!(!runtime.allows_live_output());
        assert!(!runtime.allows_prompting());
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
        let live_runtime = InteractiveRuntime {
            stdin_is_tty: true,
            stderr_is_tty: true,
            terminal: Some("xterm-256color".to_string()),
        };
        let muted_runtime = InteractiveRuntime {
            stdin_is_tty: true,
            stderr_is_tty: true,
            terminal: Some("dumb".to_string()),
        };

        let live = Spinner::with_runtime(&live_runtime, "Working");
        live.set_message("Still working");
        live.finish_with_message("Done");

        let muted = Spinner::with_runtime(&muted_runtime, "Muted");
        muted.finish_with_message("Still muted");
    }

    #[test]
    fn confirm_fails_fast_without_interactive_terminal() {
        let interactive = Interactive::new(InteractiveRuntime {
            stdin_is_tty: false,
            stderr_is_tty: false,
            terminal: None,
        });

        let err = interactive
            .confirm("Proceed?")
            .expect_err("confirm should fail");
        assert!(
            err.to_string().contains("interactive terminal"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn interactive_runtime_accessor_and_spinner_follow_runtime() {
        let runtime = InteractiveRuntime {
            stdin_is_tty: true,
            stderr_is_tty: true,
            terminal: Some("xterm-256color".to_string()),
        };
        let interactive = Interactive::new(runtime.clone());

        assert_eq!(interactive.runtime(), &runtime);
        interactive.spinner("Working").finish_and_clear();
    }

    #[test]
    fn password_fails_fast_without_interactive_terminal() {
        let interactive = Interactive::new(InteractiveRuntime {
            stdin_is_tty: false,
            stderr_is_tty: false,
            terminal: None,
        });

        let err = interactive
            .password("Password")
            .expect_err("password should fail");
        assert!(
            err.to_string().contains("interactive terminal"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn password_allow_empty_fails_fast_without_interactive_terminal() {
        let interactive = Interactive::new(InteractiveRuntime {
            stdin_is_tty: false,
            stderr_is_tty: false,
            terminal: None,
        });

        let err = interactive
            .password_allow_empty("Password")
            .expect_err("password prompt should still require a TTY");
        assert!(
            err.to_string().contains("interactive terminal"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn runtime_allows_live_output_when_term_is_missing_but_stderr_is_tty() {
        let runtime = InteractiveRuntime {
            stdin_is_tty: true,
            stderr_is_tty: true,
            terminal: None,
        };

        assert!(runtime.allows_prompting());
        assert!(runtime.allows_live_output());
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

    #[test]
    fn runtime_without_stdin_tty_can_still_allow_live_output() {
        let runtime = InteractiveRuntime {
            stdin_is_tty: false,
            stderr_is_tty: true,
            terminal: Some("xterm-256color".to_string()),
        };

        assert!(!runtime.allows_prompting());
        assert!(runtime.allows_live_output());
    }
}
