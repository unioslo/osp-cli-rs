//! The CLI module exists to define the public command-line grammar of `osp`.
//!
//! This module owns the public command-line grammar for `osp`: top-level
//! commands, shared flags, inline parsing helpers, and the bridge from CLI
//! arguments into render/config runtime settings. It does not execute commands;
//! that handoff happens in [`crate::app`].
//!
//! Read this module when you need to answer:
//!
//! - what is a valid `osp ...` command line?
//! - which flags are persistent config versus one-shot invocation settings?
//! - how does the REPL reuse the same grammar without reusing process argv?
//!
//! Broad-strokes flow:
//!
//! ```text
//! process argv or REPL line
//!      │
//!      ▼
//! [ Cli / InlineCommandCli ]
//! clap grammar for builtins and shared flags
//!      │
//!      ├── [ invocation ] one-shot execution/render flags (`--format`, `-v`,
//!      │                  `--cache`, `--plugin-provider`, ...)
//!      ├── [ pipeline ]   alias-aware command token parsing plus DSL stages
//!      └── [ commands ]   built-in command handlers once parsing is complete
//!      │
//!      ▼
//! [ app ] host orchestration and final dispatch
//! ```
//!
//! Most callers only need a few entry points:
//!
//! - [`crate::cli::Cli`] for the binary-facing grammar
//! - [`crate::cli::InlineCommandCli`] for command text that omits the binary name
//! - [`crate::cli::parse_command_text_with_aliases`] when you need alias-aware command plus
//!   DSL parsing
//!
//! The split here is deliberate. One-shot flags that affect rendering or
//! dispatch should be modeled here so CLI, REPL, tests, and embedders all see
//! the same contract. Do not let individual command handlers invent their own
//! side-channel parsing rules or hidden output flags.
//!
//! Contract:
//!
//! - this module defines what users are allowed to type
//! - it may translate flags into config/render settings
//! - it should not dispatch commands, query external systems, or own REPL
//!   editor behavior

pub(crate) mod commands;
pub(crate) mod invocation;
pub(crate) mod pipeline;
pub(crate) mod rows;
use crate::config::{ConfigLayer, RuntimeLoadOptions};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::ui::UiPresentation;

pub use pipeline::{
    ParsedCommandLine, is_cli_help_stage, parse_command_text_with_aliases,
    parse_command_tokens_with_aliases, validate_cli_dsl_stages,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum PresentationArg {
    Expressive,
    Compact,
    #[value(alias = "gammel-og-bitter")]
    Austere,
}

impl From<PresentationArg> for UiPresentation {
    fn from(value: PresentationArg) -> Self {
        match value {
            PresentationArg::Expressive => UiPresentation::Expressive,
            PresentationArg::Compact => UiPresentation::Compact,
            PresentationArg::Austere => UiPresentation::Austere,
        }
    }
}

/// Top-level CLI parser for the `osp` command.
#[derive(Debug, Parser)]
#[command(
    name = "osp",
    version = env!("CARGO_PKG_VERSION"),
    about = "OSP CLI",
    after_help = "Use `osp plugins commands` to list plugin-provided commands."
)]
pub struct Cli {
    /// Override the effective user name for this invocation.
    #[arg(short = 'u', long = "user")]
    pub user: Option<String>,

    /// Disable persistent REPL history and other identity-linked behavior.
    #[arg(short = 'i', long = "incognito", global = true)]
    pub incognito: bool,

    /// Select the active config profile for the invocation.
    #[arg(long = "profile", global = true)]
    pub profile: Option<String>,

    /// Skip environment-derived config sources.
    #[arg(long = "no-env", global = true)]
    pub no_env: bool,

    /// Skip config-file-derived sources.
    #[arg(long = "no-config-file", alias = "no-config", global = true)]
    pub no_config_file: bool,

    /// Use only built-in defaults and explicit in-memory overrides.
    ///
    /// This is stricter than combining `--no-env` and `--no-config-file`: it
    /// also disables env/path bootstrap discovery through `HOME`, `XDG_*`,
    /// `OSP_CONFIG_FILE`, and `OSP_SECRETS_FILE`.
    #[arg(long = "defaults-only", global = true)]
    pub defaults_only: bool,

    /// Add one or more plugin discovery directories.
    #[arg(long = "plugin-dir", global = true)]
    pub plugin_dirs: Vec<PathBuf>,

    /// Override the selected output theme.
    #[arg(long = "theme", global = true)]
    pub theme: Option<String>,

    #[arg(long = "presentation", alias = "app-style", global = true)]
    presentation: Option<PresentationArg>,

    #[arg(
        long = "gammel-og-bitter",
        conflicts_with = "presentation",
        global = true
    )]
    gammel_og_bitter: bool,

    /// Top-level built-in or plugin command selection.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    /// Returns the runtime source-loading options implied by global CLI flags.
    pub fn runtime_load_options(&self) -> RuntimeLoadOptions {
        runtime_load_options_from_flags(self.no_env, self.no_config_file, self.defaults_only)
    }
}

/// Top-level commands accepted by `osp`.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Inspect and manage discovered plugins.
    Plugins(PluginsArgs),
    /// Run local diagnostics and health checks.
    Doctor(DoctorArgs),
    /// Inspect and change output themes.
    Theme(ThemeArgs),
    /// Inspect and mutate CLI configuration.
    Config(ConfigArgs),
    /// Manage persisted REPL history.
    History(HistoryArgs),
    #[command(hide = true)]
    /// Render the legacy intro/help experience.
    Intro(IntroArgs),
    #[command(hide = true)]
    /// Access hidden REPL debugging and support commands.
    Repl(ReplArgs),
    #[command(external_subcommand)]
    /// Dispatch an external or plugin-provided command line.
    External(Vec<String>),
}

/// Parser used for inline command execution without the binary name prefix.
#[derive(Debug, Parser)]
#[command(name = "osp", no_binary_name = true)]
pub struct InlineCommandCli {
    /// Parsed command payload, if any.
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Hidden REPL-only command namespace.
#[derive(Debug, Args)]
pub struct ReplArgs {
    /// Hidden REPL subcommand to run.
    #[command(subcommand)]
    pub command: ReplCommands,
}

/// Hidden REPL debugging commands.
#[derive(Debug, Subcommand)]
pub enum ReplCommands {
    #[command(name = "debug-complete", hide = true)]
    /// Trace completion candidates for a partially typed line.
    DebugComplete(DebugCompleteArgs),
    #[command(name = "debug-highlight", hide = true)]
    /// Trace syntax-highlighting output for a line.
    DebugHighlight(DebugHighlightArgs),
}

/// Popup menu target to inspect through the hidden REPL debug surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DebugMenuArg {
    /// Trace the normal completion popup.
    Completion,
    /// Trace the history-search popup used by `Ctrl-R`.
    History,
}

/// Arguments for REPL completion debugging.
#[derive(Debug, Args)]
pub struct DebugCompleteArgs {
    /// Input line to complete.
    #[arg(long)]
    pub line: String,

    /// Selects which REPL popup menu to debug.
    #[arg(long = "menu", value_enum, default_value_t = DebugMenuArg::Completion)]
    pub menu: DebugMenuArg,

    /// Cursor position within `line`; defaults to the end of the line.
    #[arg(long)]
    pub cursor: Option<usize>,

    /// Virtual menu width to use when rendering completion output.
    #[arg(long, default_value_t = 80)]
    pub width: u16,

    /// Virtual menu height to use when rendering completion output.
    #[arg(long, default_value_t = 24)]
    pub height: u16,

    /// Optional completion trace steps to enable.
    #[arg(long = "step")]
    pub steps: Vec<String>,

    /// Enable ANSI styling in the rendered completion menu.
    #[arg(long = "menu-ansi", default_value_t = false)]
    pub menu_ansi: bool,

    /// Enable Unicode box-drawing in the rendered completion menu.
    #[arg(long = "menu-unicode", default_value_t = false)]
    pub menu_unicode: bool,
}

/// Arguments for REPL highlighting debugging.
#[derive(Debug, Args)]
pub struct DebugHighlightArgs {
    /// Input line to highlight.
    #[arg(long)]
    pub line: String,
}

/// Top-level plugin command arguments.
#[derive(Debug, Args)]
pub struct PluginsArgs {
    /// Plugin management action to perform.
    #[command(subcommand)]
    pub command: PluginsCommands,
}

/// Top-level doctor command arguments.
#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Optional narrowed diagnostic target.
    #[command(subcommand)]
    pub command: Option<DoctorCommands>,
}

/// Built-in diagnostic groups exposed through `osp doctor`.
#[derive(Debug, Subcommand)]
pub enum DoctorCommands {
    /// Run every available built-in diagnostic.
    All,
    /// Validate resolved configuration state.
    Config,
    /// Show the last run metadata when available.
    Last,
    /// Validate plugin discovery and state.
    Plugins,
    /// Validate theme resolution and rendering support.
    Theme,
}

/// Built-in plugin management subcommands.
#[derive(Debug, Subcommand)]
pub enum PluginsCommands {
    /// List discovered plugins.
    List,
    /// List commands exported by plugins.
    Commands,
    /// Show plugin-declared configuration metadata.
    Config(PluginConfigArgs),
    /// Force a fresh plugin discovery pass.
    Refresh,
    /// Enable a plugin-backed command.
    Enable(PluginCommandStateArgs),
    /// Disable a plugin-backed command.
    Disable(PluginCommandStateArgs),
    /// Clear persisted state for a command.
    ClearState(PluginCommandClearArgs),
    /// Select the provider implementation used for a command.
    SelectProvider(PluginProviderSelectArgs),
    /// Clear an explicit provider selection for a command.
    ClearProvider(PluginProviderClearArgs),
    /// Run plugin-specific diagnostics.
    Doctor,
}

/// Top-level theme command arguments.
#[derive(Debug, Args)]
pub struct ThemeArgs {
    /// Theme action to perform.
    #[command(subcommand)]
    pub command: ThemeCommands,
}

/// Theme inspection and selection commands.
#[derive(Debug, Subcommand)]
pub enum ThemeCommands {
    /// List available themes.
    List,
    /// Show details for a specific theme.
    Show(ThemeShowArgs),
    /// Persist or apply a selected theme.
    Use(ThemeUseArgs),
}

/// Arguments for `theme show`.
#[derive(Debug, Args)]
pub struct ThemeShowArgs {
    /// Theme name to inspect; defaults to the active theme.
    pub name: Option<String>,
}

/// Arguments for `theme use`.
#[derive(Debug, Args)]
pub struct ThemeUseArgs {
    /// Theme name to activate.
    pub name: String,
}

/// Shared arguments for enabling or disabling a plugin command.
#[derive(Debug, Args, Clone, Default)]
pub struct PluginScopeArgs {
    /// Apply the change globally instead of to a profile.
    #[arg(long = "global", conflicts_with = "profile")]
    pub global: bool,

    /// Apply the change to a named profile.
    #[arg(long = "profile")]
    pub profile: Option<String>,

    /// Target a specific terminal context, or the current one when omitted.
    #[arg(
        long = "terminal",
        num_args = 0..=1,
        default_missing_value = "__current__"
    )]
    pub terminal: Option<String>,
}

/// Shared config write scope arguments.
#[derive(Debug, Args, Clone, Default)]
pub struct ConfigScopeArgs {
    /// Write to the global store instead of a profile-scoped store.
    #[arg(long = "global", conflicts_with_all = ["profile", "profile_all"])]
    pub global: bool,

    /// Write to a single named profile.
    #[arg(long = "profile", conflicts_with = "profile_all")]
    pub profile: Option<String>,

    /// Write to every known profile store.
    #[arg(long = "profile-all", conflicts_with = "profile")]
    pub profile_all: bool,

    /// Write to a terminal-scoped store, or the current terminal when omitted.
    #[arg(
        long = "terminal",
        num_args = 0..=1,
        default_missing_value = "__current__"
    )]
    pub terminal: Option<String>,
}

/// Shared config store-selection arguments.
#[derive(Debug, Args, Clone, Default)]
pub struct ConfigStoreArgs {
    /// Apply the change only to the current in-memory session.
    #[arg(long = "session", conflicts_with_all = ["config_store", "secrets", "save"])]
    pub session: bool,

    /// Force the regular config store as the destination.
    #[arg(long = "config", conflicts_with_all = ["session", "secrets"])]
    pub config_store: bool,

    /// Force the secrets store as the destination.
    #[arg(long = "secrets", conflicts_with_all = ["session", "config_store"])]
    pub secrets: bool,

    /// Persist the change immediately after validation.
    #[arg(long = "save", conflicts_with_all = ["session", "config_store", "secrets"])]
    pub save: bool,
}

/// Shared plugin command target arguments.
#[derive(Debug, Args, Clone)]
pub struct PluginCommandTargetArgs {
    /// Command name to mutate.
    pub command: String,

    /// Shared global/profile/terminal targeting flags.
    #[command(flatten)]
    pub scope: PluginScopeArgs,
}

/// Shared config read-output arguments.
#[derive(Debug, Args, Clone, Default)]
pub struct ConfigReadOutputArgs {
    /// Include source provenance for returned keys.
    #[arg(long = "sources")]
    pub sources: bool,

    /// Emit raw stored values without presentation formatting.
    #[arg(long = "raw")]
    pub raw: bool,
}

/// Shared arguments for enabling or disabling a plugin command.
#[derive(Debug, Args)]
pub struct PluginCommandStateArgs {
    /// Shared command name plus global/profile/terminal targeting flags.
    #[command(flatten)]
    pub target: PluginCommandTargetArgs,
}

/// Arguments for clearing persisted command state.
#[derive(Debug, Args)]
pub struct PluginCommandClearArgs {
    /// Shared command name plus global/profile/terminal targeting flags.
    #[command(flatten)]
    pub target: PluginCommandTargetArgs,
}

/// Arguments for selecting a provider implementation for a command.
#[derive(Debug, Args)]
pub struct PluginProviderSelectArgs {
    /// Shared command name plus global/profile/terminal targeting flags.
    #[command(flatten)]
    pub target: PluginCommandTargetArgs,

    /// Plugin identifier to bind to the command.
    pub plugin_id: String,
}

/// Arguments for clearing a provider selection.
#[derive(Debug, Args)]
pub struct PluginProviderClearArgs {
    /// Shared command name plus global/profile/terminal targeting flags.
    #[command(flatten)]
    pub target: PluginCommandTargetArgs,
}

/// Arguments for `plugins config`.
#[derive(Debug, Args)]
pub struct PluginConfigArgs {
    /// Plugin identifier whose config schema should be shown.
    pub plugin_id: String,
}

/// Top-level config command arguments.
#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// Config action to perform.
    #[command(subcommand)]
    pub command: ConfigCommands,
}

/// Top-level history command arguments.
#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// History action to perform.
    #[command(subcommand)]
    pub command: HistoryCommands,
}

/// Hidden intro command arguments.
#[derive(Debug, Args, Clone, Default)]
pub struct IntroArgs {}

/// History management commands.
#[derive(Debug, Subcommand)]
pub enum HistoryCommands {
    /// List persisted history entries.
    List,
    /// Retain only the newest `keep` entries.
    Prune(HistoryPruneArgs),
    /// Remove all persisted history entries.
    Clear,
}

/// Arguments for `history prune`.
#[derive(Debug, Args)]
pub struct HistoryPruneArgs {
    /// Number of recent entries to keep.
    pub keep: usize,
}

/// Configuration inspection and mutation commands.
#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Show the resolved configuration view.
    Show(ConfigShowArgs),
    /// Read a single resolved config key.
    Get(ConfigGetArgs),
    /// Explain how a config key was resolved.
    Explain(ConfigExplainArgs),
    /// Set a config key in one or more writable stores.
    Set(ConfigSetArgs),
    /// Remove a config key from one or more writable stores.
    Unset(ConfigUnsetArgs),
    #[command(alias = "diagnostics")]
    /// Run config-specific diagnostics.
    Doctor,
}

/// Arguments for `config show`.
#[derive(Debug, Args)]
pub struct ConfigShowArgs {
    /// Shared source/raw output flags for config reads.
    #[command(flatten)]
    pub output: ConfigReadOutputArgs,
}

/// Arguments for `config get`.
#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    /// Config key to read.
    pub key: String,

    /// Shared source/raw output flags for config reads.
    #[command(flatten)]
    pub output: ConfigReadOutputArgs,
}

/// Arguments for `config explain`.
#[derive(Debug, Args)]
pub struct ConfigExplainArgs {
    /// Config key to explain.
    pub key: String,

    /// Reveal secret values in the explanation output.
    #[arg(long = "show-secrets")]
    pub show_secrets: bool,
}

/// Arguments for `config set`.
#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    /// Config key to write.
    pub key: String,
    /// Config value to write.
    pub value: String,

    /// Shared global/profile/terminal targeting flags.
    #[command(flatten)]
    pub scope: ConfigScopeArgs,

    /// Shared config/session/secrets store-selection flags.
    #[command(flatten)]
    pub store: ConfigStoreArgs,

    /// Show the resolved write plan without applying it.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Skip interactive confirmation prompts.
    #[arg(long = "yes")]
    pub yes: bool,

    /// Show an explanation of the resolved write targets.
    #[arg(long = "explain")]
    pub explain: bool,
}

/// Arguments for `config unset`.
#[derive(Debug, Args)]
pub struct ConfigUnsetArgs {
    /// Config key to remove.
    pub key: String,

    /// Shared global/profile/terminal targeting flags.
    #[command(flatten)]
    pub scope: ConfigScopeArgs,

    /// Shared config/session/secrets store-selection flags.
    #[command(flatten)]
    pub store: ConfigStoreArgs,

    /// Show the resolved removal plan without applying it.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

impl Cli {
    pub(crate) fn append_static_session_overrides(&self, layer: &mut ConfigLayer) {
        if let Some(user) = self
            .user
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            layer.set("user.name", user);
        }
        if self.incognito {
            layer.set("repl.history.enabled", false);
        }
        append_appearance_overrides(
            layer,
            self.theme.as_deref(),
            if self.gammel_og_bitter {
                Some(UiPresentation::Austere)
            } else {
                self.presentation.map(UiPresentation::from)
            },
        );
    }
}

pub(crate) fn append_appearance_overrides(
    layer: &mut ConfigLayer,
    theme: Option<&str>,
    presentation: Option<UiPresentation>,
) {
    if let Some(theme) = theme.map(str::trim).filter(|value| !value.is_empty()) {
        layer.set("theme.name", theme);
    }
    if let Some(presentation) = presentation {
        layer.set("ui.presentation", presentation.as_config_value());
    }
}

pub(crate) fn runtime_load_options_from_flags(
    no_env: bool,
    no_config_file: bool,
    defaults_only: bool,
) -> RuntimeLoadOptions {
    if defaults_only {
        RuntimeLoadOptions::defaults_only()
    } else {
        RuntimeLoadOptions::new()
            .with_env(!no_env)
            .with_config_file(!no_config_file)
    }
}

/// Parses inline command tokens with the same clap model as the top-level CLI.
///
/// This is the REPL-facing path for turning already-tokenized input into a
/// concrete builtin command, and it returns `Ok(None)` when no subcommand has
/// been selected yet.
///
/// # Examples
///
/// ```
/// use osp_cli::cli::{Commands, ThemeCommands, parse_inline_command_tokens};
///
/// let tokens = vec![
///     "theme".to_string(),
///     "show".to_string(),
///     "dracula".to_string(),
/// ];
///
/// let command = parse_inline_command_tokens(&tokens).unwrap().unwrap();
/// match command {
///     Commands::Theme(args) => match args.command {
///         ThemeCommands::Show(show) => {
///             assert_eq!(show.name.as_deref(), Some("dracula"));
///         }
///         other => panic!("unexpected theme command: {other:?}"),
///     },
///     other => panic!("unexpected command: {other:?}"),
/// }
/// ```
pub fn parse_inline_command_tokens(tokens: &[String]) -> Result<Option<Commands>, clap::Error> {
    InlineCommandCli::try_parse_from(tokens.iter().map(String::as_str)).map(|parsed| parsed.command)
}

#[cfg(test)]
mod tests {
    use super::{
        Cli, Commands, ConfigCommands, InlineCommandCli, RuntimeLoadOptions,
        append_appearance_overrides, parse_inline_command_tokens,
    };
    use crate::config::{ConfigLayer, ConfigValue};
    use clap::Parser;

    #[test]
    fn parse_inline_command_tokens_accepts_builtin_and_external_commands_unit() {
        let builtin = parse_inline_command_tokens(&["config".to_string(), "doctor".to_string()])
            .expect("builtin command should parse");
        assert!(matches!(
            builtin,
            Some(Commands::Config(args)) if matches!(args.command, ConfigCommands::Doctor)
        ));

        let external = parse_inline_command_tokens(&["ldap".to_string(), "user".to_string()])
            .expect("external command should parse");
        assert!(
            matches!(external, Some(Commands::External(tokens)) if tokens == vec!["ldap", "user"])
        );
    }

    #[test]
    fn cli_runtime_load_options_and_inline_parser_follow_disable_flags_unit() {
        let cli = Cli::parse_from(["osp", "--no-env", "--no-config-file", "theme", "list"]);
        assert_eq!(
            cli.runtime_load_options(),
            RuntimeLoadOptions::new()
                .with_env(false)
                .with_config_file(false)
        );

        let cli = Cli::parse_from(["osp", "--defaults-only", "theme", "list"]);
        assert_eq!(
            cli.runtime_load_options(),
            RuntimeLoadOptions::defaults_only()
        );

        let inline = InlineCommandCli::try_parse_from(["theme", "list"])
            .expect("inline command should parse");
        assert!(matches!(inline.command, Some(Commands::Theme(_))));
    }

    #[test]
    fn app_style_alias_maps_to_presentation_unit() {
        let cli = Cli::parse_from(["osp", "--app-style", "austere"]);
        let mut layer = ConfigLayer::default();
        cli.append_static_session_overrides(&mut layer);
        assert_eq!(
            layer
                .entries()
                .iter()
                .find(|entry| entry.key == "ui.presentation")
                .map(|entry| &entry.value),
            Some(&ConfigValue::from("austere"))
        );
    }

    #[test]
    fn appearance_overrides_trim_theme_and_apply_presentation_unit() {
        let mut layer = ConfigLayer::default();
        append_appearance_overrides(
            &mut layer,
            Some(" nord "),
            Some(crate::ui::UiPresentation::Compact),
        );

        assert_eq!(
            layer
                .entries()
                .iter()
                .find(|entry| entry.key == "theme.name")
                .map(|entry| &entry.value),
            Some(&ConfigValue::from("nord"))
        );
        assert_eq!(
            layer
                .entries()
                .iter()
                .find(|entry| entry.key == "ui.presentation")
                .map(|entry| &entry.value),
            Some(&ConfigValue::from("compact"))
        );
    }
}
