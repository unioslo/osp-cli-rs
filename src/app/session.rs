//! Session-scoped host state for one logical app run.
//!
//! This module exists to hold mutable state that should survive across commands
//! within the same session, but should not be promoted to global runtime
//! state.
//!
//! High-level flow:
//!
//! - track prompt timing plus last-success/last-failure REPL details
//! - maintain the REPL scope stack
//! - bundle session state that host code needs to carry between dispatches
//!
//! Contract:
//!
//! - session data here is narrower-lived than the runtime state in
//!   [`super::runtime`]
//! - long-lived environment/config/plugin bootstrap state should not drift into
//!   this module
//!
//! Public API shape:
//!
//! - use [`AppSession::default`] plus the `with_*` chainers for session-scoped
//!   REPL state
//! - use [`AppStateBuilder`] when you need a fully assembled runtime/session
//!   snapshot outside the full CLI bootstrap
//! - these types are host machinery, not lightweight semantic DTOs

use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::config::ConfigLayer;
use crate::native::{NativeCommandRegistry, NativeSessionContext};
use crate::plugin::PluginManager;
use crate::repl::HistoryShellContext;

use super::command_output::CommandOutput;
use super::runtime::{AppClients, AppRuntime, LaunchContext, RuntimeContext, UiState};
use super::timing::TimingSummary;

#[derive(Debug, Clone, Copy, Default)]
/// Timing badge rendered in the prompt for the most recent command.
pub(crate) struct DebugTimingBadge {
    /// Prompt detail level used when rendering the badge.
    pub level: u8,
    pub(crate) summary: TimingSummary,
}

/// Shared prompt-timing storage that dispatch code can update and prompt
/// rendering can read.
#[derive(Clone, Default, Debug)]
pub(crate) struct DebugTimingState {
    inner: Arc<RwLock<Option<DebugTimingBadge>>>,
}

impl DebugTimingState {
    /// Stores the current timing badge.
    pub fn set(&self, badge: DebugTimingBadge) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = Some(badge);
        }
    }

    /// Clears any stored timing badge.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = None;
        }
    }

    /// Returns the current timing badge, if one is available.
    pub fn badge(&self) -> Option<DebugTimingBadge> {
        self.inner.read().map(|value| *value).unwrap_or(None)
    }
}

/// One entered command scope inside the interactive REPL shell stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplScopeFrame {
    command: String,
}

impl ReplScopeFrame {
    /// Creates a frame for the given command name.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::ReplScopeFrame;
    ///
    /// let frame = ReplScopeFrame::new("theme");
    /// assert_eq!(frame.command(), "theme");
    /// ```
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    /// Returns the command name associated with this scope frame.
    pub fn command(&self) -> &str {
        self.command.as_str()
    }
}

/// The REPL's current command shell, used for shell-style scoped interaction.
///
/// This is what lets the REPL stay "inside" a command family while still
/// rendering scope labels, help targets, and history prefixes consistently.
/// Shells are root commands, so there is at most one: entering another shell
/// switches to it rather than nesting.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReplScopeStack {
    frame: Option<ReplScopeFrame>,
}

impl ReplScopeStack {
    /// Returns `true` when the REPL is at the top-level scope.
    pub fn is_root(&self) -> bool {
        self.frame.is_none()
    }

    /// Enters a command shell, leaving any current one.
    pub fn enter(&mut self, command: impl Into<String>) {
        self.frame = Some(ReplScopeFrame::new(command));
    }

    /// Leaves the current command shell.
    pub fn leave(&mut self) -> Option<ReplScopeFrame> {
        self.frame.take()
    }

    /// Returns the command path represented by the current stack.
    pub fn commands(&self) -> Vec<String> {
        self.frame
            .iter()
            .map(|frame| frame.command.clone())
            .collect()
    }

    /// Returns whether the given command is the current shell.
    pub fn contains_command(&self, command: &str) -> bool {
        self.frame
            .iter()
            .any(|frame| frame.command.eq_ignore_ascii_case(command))
    }

    /// Returns a human-readable label for the current scope path.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// assert_eq!(scope.display_label(), None);
    ///
    /// scope.enter("orch");
    /// scope.enter("nh");
    /// assert_eq!(scope.display_label(), Some("nh".to_string()));
    /// ```
    pub fn display_label(&self) -> Option<String> {
        self.frame.as_ref().map(|frame| frame.command.clone())
    }

    /// Returns the history prefix used for shell-backed history entries.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// scope.enter("theme");
    /// scope.enter("orch");
    ///
    /// assert_eq!(scope.history_prefix(), "orch ");
    /// ```
    pub fn history_prefix(&self) -> String {
        self.frame
            .as_ref()
            .map(|frame| format!("{} ", frame.command))
            .unwrap_or_default()
    }

    /// Returns the active history scope prefix, if the REPL is inside a shell.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// assert_eq!(scope.history_scope_prefix(), None);
    ///
    /// scope.enter("theme");
    /// assert_eq!(scope.history_scope_prefix(), Some("theme ".to_string()));
    /// ```
    pub fn history_scope_prefix(&self) -> Option<String> {
        let prefix = self.history_prefix();
        if prefix.is_empty() {
            None
        } else {
            Some(prefix)
        }
    }

    /// Returns the user-facing label for the current history scope.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// assert_eq!(scope.history_scope_label(), "root history");
    ///
    /// scope.enter("theme");
    /// scope.enter("orch");
    /// assert_eq!(scope.history_scope_label(), "orch shell history");
    /// ```
    pub fn history_scope_label(&self) -> String {
        self.display_label()
            .map(|label| format!("{label} shell history"))
            .unwrap_or_else(|| "root history".to_string())
    }

    /// Prepends the active scope path unless the tokens are already scoped.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// scope.enter("theme");
    ///
    /// assert_eq!(
    ///     scope.prefixed_tokens(&["use".to_string(), "dracula".to_string()]),
    ///     vec!["theme".to_string(), "use".to_string(), "dracula".to_string()]
    /// );
    /// ```
    pub fn prefixed_tokens(&self, tokens: &[String]) -> Vec<String> {
        let prefix = self.commands();
        if prefix.is_empty() || tokens.starts_with(&prefix) {
            return tokens.to_vec();
        }
        let mut full = prefix;
        full.extend_from_slice(tokens);
        full
    }

    /// Returns help tokens for the current scope.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// scope.enter("theme");
    ///
    /// assert_eq!(scope.help_tokens(), vec!["theme".to_string(), "--help".to_string()]);
    /// ```
    pub fn help_tokens(&self) -> Vec<String> {
        let mut tokens = self.commands();
        if !tokens.is_empty() {
            tokens.push("--help".to_string());
        }
        tokens
    }
}

/// Session-scoped REPL state and prompt metadata.
#[non_exhaustive]
#[must_use]
#[derive(Clone)]
pub(crate) struct AppSession {
    /// Whether history capture is enabled for this session.
    pub history_enabled: bool,
    pub(crate) recording: crate::repl::recording::Recording,
    /// Shell-scoped history prefix state shared with the history store.
    pub history_shell: HistoryShellContext,
    /// Shared prompt timing badge state.
    pub prompt_timing: DebugTimingState,
    /// Session-scoped context exposed by native commands to the REPL host.
    pub native_context: NativeSessionContext,
    /// Structured follow-up commands, independent of resource context.
    pub repl_feedback: crate::repl::ReplFeedback,
    pub(crate) startup_prompt_timing_pending: bool,
    /// Current command shell within the REPL.
    pub scope: ReplScopeStack,
    /// Raw output plus pipeline stages from the most recent successful REPL command.
    pub(crate) last_success: Option<LastSuccess>,
    pub(crate) command_formats: crate::repl::command_formats::CommandFormats,
    /// Last staged command and invocation settings for target-specific rendering.
    pub(crate) last_output: Option<(Arc<super::CommandOutput>, crate::ui::RenderSettings)>,
    /// Summary of the most recent failed REPL command.
    pub last_failure: Option<LastFailure>,
    /// Session-scoped config overrides layered above persisted config.
    pub config_overrides: ConfigLayer,
}

/// The last failed REPL command and its original diagnostic evidence.
#[derive(Debug, Clone)]
pub(crate) struct LastFailure {
    /// Command label identifying the failure without retaining its arguments.
    pub command_line: String,
    /// Short failure summary suitable for prompts or status output.
    pub summary: String,
    report: std::sync::Arc<miette::Report>,
    settings: Box<crate::ui::RenderSettings>,
    backtrace: Option<std::sync::Arc<std::backtrace::Backtrace>>,
    pub(crate) command_evidence: String,
}

impl LastFailure {
    /// Records supplied summary and detail text without duplicating the detail ladder.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn new(command_line: String, summary: String, detail: String) -> Self {
        Self {
            command_line,
            summary,
            report: std::sync::Arc::new(miette::miette!("{detail}")),
            settings: Box::new(crate::ui::RenderSettings::test_plain(
                crate::core::output::OutputFormat::Table,
            )),
            backtrace: None,
            command_evidence: String::new(),
        }
    }

    pub(crate) fn from_report(
        command_line: String,
        report: std::sync::Arc<miette::Report>,
        settings: crate::ui::RenderSettings,
    ) -> Self {
        let summary = super::host::render_report_message(
            &report,
            crate::core::MessageLevel::Success,
            &settings,
        );
        let backtrace =
            super::host::capture_missing_report_backtrace(&report).map(std::sync::Arc::new);
        let report = if super::host::find_error_in_chain::<
            super::command_output::CommandBackendDetails,
        >(&report)
        .is_some()
            || super::host::find_error_in_chain::<crate::config::ConfigError>(&report).is_some()
        {
            std::sync::Arc::new(miette::miette!("{}", summary))
        } else {
            report
        };
        let root = command_line.split_whitespace().next().unwrap_or_default();
        let command_line = if command_line.split_whitespace().nth(1).is_some() {
            format!("{root} …")
        } else {
            root.to_owned()
        };
        Self {
            command_line,
            summary,
            command_evidence: super::host::report_evidence(
                &report,
                crate::core::MessageLevel::Evidence,
            )
            .0,
            report,
            settings: Box::new(settings),
            backtrace,
        }
    }

    /// Renders diagnostic detail with the terminal settings captured at failure time.
    pub fn detail(&self, level: crate::core::MessageLevel) -> String {
        super::host::render_report_message(&self.report, level, &self.settings)
    }
    /// Renders the backtrace captured when the failure was recorded, if available.
    pub fn backtrace(&self) -> Option<String> {
        super::host::render_backtrace(&self.report, &self.settings).or_else(|| {
            self.backtrace
                .as_ref()
                .map(|trace| format!("Stack backtrace:\n{trace}"))
        })
    }
}

#[derive(Debug, Clone)]
/// Stored output contract for replaying the last successful REPL result.
pub(crate) struct LastSuccess {
    pub(crate) output: std::sync::Arc<CommandOutput>,
    pub(crate) stages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplExitTransition {
    ExitRoot,
    LeftShell { frame: ReplScopeFrame },
}

impl AppSession {
    /// Enables or disables history capture for this session.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn with_history_enabled(mut self, history_enabled: bool) -> Self {
        self.history_enabled = history_enabled;
        self
    }

    /// Replaces the shell-scoped history context shared with the history store.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn with_history_shell(mut self, history_shell: HistoryShellContext) -> Self {
        self.history_shell = history_shell;
        self
    }

    /// Replaces the session-scoped config overrides layered above persisted config.
    pub fn with_config_overrides(mut self, config_overrides: ConfigLayer) -> Self {
        self.config_overrides = config_overrides;
        self
    }

    /// Enters a REPL shell, leaving any current one, and synchronizes history context.
    pub fn enter_repl_scope(&mut self, command: impl Into<String>) {
        self.scope.enter(command);
        self.sync_history_shell_context();
    }

    /// Leaves the current REPL shell scope and synchronizes history context.
    pub fn leave_repl_scope(&mut self) -> Option<ReplScopeFrame> {
        let frame = self.scope.leave()?;
        self.sync_history_shell_context();
        Some(frame)
    }

    /// Applies a user `exit` request against the current REPL scope.
    pub(crate) fn request_repl_exit(&mut self) -> ReplExitTransition {
        if self.scope.is_root() {
            self.sync_history_shell_context();
            ReplExitTransition::ExitRoot
        } else {
            match self.leave_repl_scope() {
                Some(frame) => ReplExitTransition::LeftShell { frame },
                None => ReplExitTransition::ExitRoot,
            }
        }
    }

    /// Finalizes a completed REPL line by synchronizing derived session state.
    pub(crate) fn finish_repl_line(&self) {
        self.sync_history_shell_context();
    }

    /// Stores the last successful output so the REPL can replay it later.
    pub(crate) fn record_success_output(
        &mut self,
        command_line: &str,
        output: CommandOutput,
        stages: &[String],
    ) {
        let command_line = command_line.trim().to_string();
        if command_line.is_empty() {
            return;
        }
        self.last_success = Some(LastSuccess {
            output: std::sync::Arc::new(output),
            stages: stages
                .iter()
                .filter(|stage| {
                    !matches!(
                        stage.trim().to_ascii_uppercase().as_str(),
                        "Y" | "LESS" | "EDITOR"
                    )
                })
                .cloned()
                .collect(),
        });
    }

    /// Records details about the latest failed command.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn record_failure(
        &mut self,
        command_line: &str,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let command_line = command_line.trim().to_string();
        if command_line.is_empty() {
            return;
        }
        let summary = summary.into();
        let detail = detail.into();
        self.last_failure = Some(LastFailure::new(command_line, summary, detail));
    }

    /// Retains the original report so follow-up diagnostics render on demand.
    pub(crate) fn record_failure_report(
        &mut self,
        command_line: &str,
        report: std::sync::Arc<miette::Report>,
        settings: crate::ui::RenderSettings,
    ) {
        let command_line = command_line.trim().to_string();
        if command_line.is_empty() {
            return;
        }
        self.last_failure = Some(LastFailure::from_report(command_line, report, settings));
    }

    /// Returns the last successful REPL output contract, if available.
    pub(crate) fn last_success(&self) -> Option<&LastSuccess> {
        self.last_success.as_ref()
    }

    /// Updates the prompt timing badge for the most recent command.
    pub fn record_prompt_timing(
        &self,
        level: u8,
        total: Duration,
        parse: Option<Duration>,
        execute: Option<Duration>,
        render: Option<Duration>,
    ) {
        if level == 0 {
            self.prompt_timing.clear();
            return;
        }

        self.prompt_timing.set(DebugTimingBadge {
            level,
            summary: TimingSummary {
                total,
                parse,
                execute,
                render,
            },
        });
    }

    /// Seeds the initial prompt timing badge emitted during startup.
    pub fn seed_startup_prompt_timing(&mut self, level: u8, total: Duration) {
        if !self.startup_prompt_timing_pending {
            return;
        }
        self.startup_prompt_timing_pending = false;
        if level == 0 {
            return;
        }

        self.prompt_timing.set(DebugTimingBadge {
            level,
            summary: TimingSummary {
                total,
                parse: None,
                execute: None,
                render: None,
            },
        });
    }

    /// Synchronizes history context with the current REPL scope.
    pub fn sync_history_shell_context(&self) {
        self.history_shell.set_prefix(self.scope.history_prefix());
    }
}

impl Default for AppSession {
    fn default() -> Self {
        Self {
            history_enabled: true,
            recording: Default::default(),
            history_shell: HistoryShellContext::default(),
            prompt_timing: DebugTimingState::default(),
            native_context: NativeSessionContext::default(),
            repl_feedback: Default::default(),
            startup_prompt_timing_pending: true,
            scope: ReplScopeStack::default(),
            last_success: None,
            command_formats: Default::default(),
            last_output: None,
            last_failure: None,
            config_overrides: ConfigLayer::default(),
        }
    }
}

/// Aggregate application state shared between runtime and session logic.
#[non_exhaustive]
#[must_use]
pub(crate) struct AppState {
    /// Runtime-scoped services and resolved config state.
    pub runtime: AppRuntime,
    /// Session-scoped REPL state and prompt metadata.
    pub session: AppSession,
    /// Shared client registries used during command execution.
    pub clients: AppClients,
}

impl AppState {
    pub(crate) fn reload_config_after_access_hook(&mut self) -> miette::Result<()> {
        if self.runtime.config_reload_requested {
            crate::app::assembly::rebuild_repl_in_place(self)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
impl AppState {
    /// Builds a full application-state snapshot by deriving UI state from the
    /// resolved config and runtime context.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::app::{AppState, RuntimeContext, TerminalKind};
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("repl.history.path", "/tmp/osp-doc-history.jsonl");
    /// defaults.set("theme.path", Vec::<String>::new());
    /// defaults.set("ui.verbosity", "warning");
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let config = resolver.resolve(ResolveOptions::new().with_terminal("repl")).unwrap();
    ///
    /// let state = AppState::from_resolved_config(
    ///     RuntimeContext::new(None, TerminalKind::Repl, None),
    ///     config,
    /// )
    /// .unwrap();
    ///
    /// assert_eq!(state.runtime.config_state().resolved().active_profile(), "default");
    /// assert_eq!(state.runtime.ui().verbosity.as_str(), "warning");
    /// assert!(state.clients.plugins().explicit_dirs().is_empty());
    /// ```
    pub fn from_resolved_config(
        context: RuntimeContext,
        config: crate::config::ResolvedConfig,
    ) -> miette::Result<Self> {
        AppStateBuilder::from_resolved_config(context, config).map(AppStateBuilder::build)
    }

    /// Synchronizes the history shell context with the current session scope.
    pub fn sync_history_shell_context(&self) {
        self.session.sync_history_shell_context();
    }

    /// Records a failed REPL command and its associated messages.
    pub fn record_repl_failure(
        &mut self,
        command_line: &str,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.session.record_failure(command_line, summary, detail);
    }

    /// Returns details about the most recent failed REPL command.
    pub fn last_repl_failure(&self) -> Option<LastFailure> {
        self.session.last_failure.clone()
    }
}

/// Builder for [`AppState`].
///
/// This is the canonical manual-construction factory for runtime/session/client
/// state when callers need a snapshot without going through full CLI bootstrap.
///
/// Use [`AppStateBuilder::from_resolved_config`] for the normal config-driven
/// path, then override specific pieces such as the session or plugin manager as
/// needed. Derived UI state always comes from the resolved configuration.
///
/// # Examples
///
/// ```text
/// use osp_cli::app::{AppSession, AppStateBuilder, RuntimeContext, TerminalKind};
/// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
///
/// let mut defaults = ConfigLayer::default();
/// defaults.set("profile.default", "default");
/// defaults.set("repl.history.path", "/tmp/osp-doc-history.jsonl");
/// defaults.set("theme.path", Vec::<String>::new());
///
/// let mut resolver = ConfigResolver::default();
/// resolver.set_defaults(defaults);
/// let config = resolver.resolve(ResolveOptions::new().with_terminal("repl")).unwrap();
///
/// let state = AppStateBuilder::from_resolved_config(
///     RuntimeContext::new(None, TerminalKind::Repl, None),
///     config,
/// )?
/// .with_session(AppSession::default().with_history_enabled(false))
/// .build();
///
/// assert!(!state.session.history_enabled);
/// # Ok::<(), miette::Report>(())
/// ```
#[must_use]
pub(crate) struct AppStateBuilder {
    pub(crate) context: RuntimeContext,
    pub(crate) config: crate::config::ResolvedConfig,
    pub(crate) ui: UiState,
    pub(crate) launch: LaunchContext,
    pub(crate) plugins: Option<PluginManager>,
    pub(crate) native_commands: NativeCommandRegistry,
    pub(crate) session: Option<AppSession>,
    pub(crate) themes: Option<crate::ui::theme_catalog::ThemeCatalog>,
    pub(crate) definition: std::sync::Arc<super::AppDefinition>,
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
impl AppStateBuilder {
    /// Starts building an application-state snapshot from the resolved config
    /// and UI state the caller wants to expose.
    ///
    /// This is the manual-construction path. Prefer
    /// [`AppStateBuilder::from_resolved_config`] when the builder should derive
    /// UI defaults from config and runtime context first.
    #[cfg(test)]
    pub(crate) fn new(
        context: RuntimeContext,
        config: crate::config::ResolvedConfig,
        ui: UiState,
    ) -> Self {
        Self {
            context,
            config,
            ui,
            launch: LaunchContext::default(),
            plugins: None,
            native_commands: NativeCommandRegistry::default(),
            session: None,
            themes: None,
            definition: Default::default(),
        }
    }

    /// Starts a builder by deriving UI state from the resolved config and
    /// runtime context.
    ///
    /// This is the canonical embedder entrypoint when you want one coherent
    /// host snapshot and only plan to override selected pieces before build.
    pub fn from_resolved_config(
        context: RuntimeContext,
        config: crate::config::ResolvedConfig,
    ) -> miette::Result<Self> {
        let (ui, themes) =
            super::assembly::build_host(super::assembly::HostBuildRequest::Presentation {
                context: &context,
                config: Some(&config),
                seed: super::assembly::RenderSettingsSeed::DefaultAuto,
                theme: None,
                tolerant: false,
            })?
            .presentation();
        let session = AppSession::default();
        crate::ui::theme_catalog::log_theme_issues(&themes.issues);
        Ok(Self {
            context,
            config,
            ui,
            launch: LaunchContext::default(),
            plugins: None,
            native_commands: NativeCommandRegistry::default(),
            session: Some(session),
            themes: Some(themes),
            definition: Default::default(),
        })
    }

    /// Replaces the launch-time provenance used for cache and plugin setup.
    ///
    /// If omitted, the builder keeps [`LaunchContext::default`].
    pub fn with_launch(mut self, launch: LaunchContext) -> Self {
        self.launch = launch;
        self
    }

    /// Replaces the plugin manager used when assembling shared clients.
    ///
    /// If omitted, the builder derives the plugin manager from the resolved
    /// config plus the current launch context during [`AppStateBuilder::build`].
    #[cfg(test)]
    pub(crate) fn with_plugins(mut self, plugins: PluginManager) -> Self {
        self.plugins = Some(plugins);
        self
    }

    /// Replaces the native command registry used when assembling shared
    /// clients.
    ///
    /// If omitted, the builder keeps [`NativeCommandRegistry::default`].
    pub fn with_native_commands(mut self, native_commands: NativeCommandRegistry) -> Self {
        self.native_commands = native_commands;
        self
    }

    /// Replaces the session snapshot carried by the built app state.
    ///
    /// If omitted, the builder uses the derived/default session for the
    /// current config.
    pub fn with_session(mut self, session: AppSession) -> Self {
        self.session = Some(session);
        self
    }

    /// Builds the configured [`AppState`].
    ///
    /// This assembles one coherent runtime/session/client snapshot, deriving any
    /// omitted pieces such as themes, plugin manager, and default session before
    /// returning the final value.
    pub fn build(self) -> AppState {
        super::assembly::assemble_state(self, false)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {

    #[test]
    fn host_state_example_1() {
        use crate::app::ReplScopeFrame;

        let frame = ReplScopeFrame::new("theme");
        assert_eq!(frame.command(), "theme");
    }

    #[test]
    fn host_state_example_2() {
        use crate::app::ReplScopeStack;

        let mut scope = ReplScopeStack::default();
        assert_eq!(scope.display_label(), None);

        scope.enter("orch");
        scope.enter("nh");
        assert_eq!(scope.display_label(), Some("nh".to_string()));
    }

    #[test]
    fn host_state_example_3() {
        use crate::app::ReplScopeStack;

        let mut scope = ReplScopeStack::default();
        scope.enter("theme");
        scope.enter("orch");

        assert_eq!(scope.history_prefix(), "orch ");
    }

    #[test]
    fn host_state_example_4() {
        use crate::app::ReplScopeStack;

        let mut scope = ReplScopeStack::default();
        assert_eq!(scope.history_scope_prefix(), None);

        scope.enter("theme");
        assert_eq!(scope.history_scope_prefix(), Some("theme ".to_string()));
    }

    #[test]
    fn host_state_example_5() {
        use crate::app::ReplScopeStack;

        let mut scope = ReplScopeStack::default();
        assert_eq!(scope.history_scope_label(), "root history");

        scope.enter("theme");
        scope.enter("orch");
        assert_eq!(scope.history_scope_label(), "orch shell history");
    }

    #[test]
    fn host_state_example_6() {
        use crate::app::ReplScopeStack;

        let mut scope = ReplScopeStack::default();
        scope.enter("theme");

        assert_eq!(
            scope.prefixed_tokens(&["use".to_string(), "dracula".to_string()]),
            vec![
                "theme".to_string(),
                "use".to_string(),
                "dracula".to_string()
            ]
        );
    }

    #[test]
    fn host_state_example_7() {
        use crate::app::ReplScopeStack;

        let mut scope = ReplScopeStack::default();
        scope.enter("theme");

        assert_eq!(
            scope.help_tokens(),
            vec!["theme".to_string(), "--help".to_string()]
        );
    }

    #[test]
    fn host_state_example_8() {
        (|| {
            use crate::app::{AppSession, AppStateBuilder, RuntimeContext, TerminalKind};
            use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};

            let mut defaults = ConfigLayer::default();
            defaults.set("profile.default", "default");
            defaults.set("repl.history.path", "/tmp/osp-doc-history.jsonl");
            defaults.set("theme.path", Vec::<String>::new());

            let mut resolver = ConfigResolver::default();
            resolver.set_defaults(defaults);
            let config = resolver
                .resolve(ResolveOptions::new().with_terminal("repl"))
                .unwrap();

            let state = AppStateBuilder::from_resolved_config(
                RuntimeContext::new(None, TerminalKind::Repl, None),
                config,
            )?
            .with_session(AppSession::default().with_history_enabled(false))
            .build();

            assert!(!state.session.history_enabled);
            Ok::<(), miette::Report>(())
        })()
        .unwrap();
    }

    #[test]
    fn internal_example_1() {
        use crate::app::{AppState, RuntimeContext, TerminalKind};
        use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};

        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("repl.history.path", "/tmp/osp-doc-history.jsonl");
        defaults.set("theme.path", Vec::<String>::new());
        defaults.set("ui.verbosity", "warning");

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::new().with_terminal("repl"))
            .unwrap();

        let state = AppState::from_resolved_config(
            RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
        )
        .unwrap();

        assert_eq!(
            state.runtime.config_state().resolved().active_profile(),
            "default"
        );
        assert_eq!(state.runtime.ui().verbosity.as_str(), "warning");
        assert!(state.clients.plugins().explicit_dirs().is_empty());
    }

    use std::time::Duration;

    use super::{AppSession, ReplExitTransition};
    use crate::config::ConfigLayer;

    #[test]
    fn request_repl_exit_tracks_root_and_shell_transitions_unit() {
        let mut root = AppSession::default();
        assert!(matches!(
            root.request_repl_exit(),
            ReplExitTransition::ExitRoot
        ));

        // Entering another shell switches; exit then returns to root.
        let mut shell = AppSession::default();
        shell.enter_repl_scope("ldap");
        shell.enter_repl_scope("orch");
        assert_eq!(shell.scope.commands(), vec!["orch".to_string()]);
        assert!(matches!(
            shell.request_repl_exit(),
            ReplExitTransition::LeftShell { frame } if frame.command() == "orch"
        ));
        assert!(shell.scope.is_root());
    }

    #[test]
    fn rebuild_state_round_trip_preserves_replay_and_scope_unit() {
        let mut session = AppSession::default().with_history_enabled(false);
        let mut overrides = ConfigLayer::default();
        overrides.set("ui.format", "json");
        session = session.with_config_overrides(overrides);
        session.enter_repl_scope("ldap");
        session.record_prompt_timing(2, Duration::from_secs(3), None, None, None);
        session.startup_prompt_timing_pending = false;

        session.record_success_output(
            "list users",
            crate::app::CommandOutput::json(serde_json::json!([{ "name": "alice" }])),
            &["P name".to_string()],
        );
        session.record_failure("list users", "Command failed", "detail");
        let restored = session.clone();
        restored.sync_history_shell_context();

        assert!(!restored.history_enabled);
        assert_eq!(restored.scope.commands(), vec!["ldap".to_string()]);
        assert_eq!(restored.history_shell.prefix(), Some("ldap ".to_string()));
        assert!(restored.last_success().is_some());
        assert_eq!(
            restored
                .last_failure
                .as_ref()
                .map(|failure| failure.summary.as_str()),
            Some("Command failed")
        );
        assert_eq!(
            restored.prompt_timing.badge().map(|badge| badge.level),
            Some(2)
        );
        assert!(!restored.startup_prompt_timing_pending);
        assert_eq!(restored.config_overrides.entries().len(), 1);
        assert_eq!(restored.config_overrides.entries()[0].key, "ui.format");
        assert_eq!(
            restored.config_overrides.entries()[0].value.to_string(),
            "json"
        );
    }
}
