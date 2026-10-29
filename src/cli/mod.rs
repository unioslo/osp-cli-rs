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
//!      │                  rendering options, ...)
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
//! - `InlineCommandCli` for command text that omits the binary name
//! - `parse_command_text_with_aliases` when you need alias-aware command plus
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
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::ui::UiPresentation;

pub(crate) use pipeline::{ParsedCommandLine, is_cli_help_stage, validate_cli_dsl_stages};
#[cfg(test)]
pub(crate) use pipeline::{parse_command_text_with_aliases, parse_command_tokens_with_aliases};

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
#[derive(Parser)]
#[command(
    name = "osp",
    version = env!("CARGO_PKG_VERSION"),
    about = "OSP CLI",
    after_help = "Run `osp COMMAND --help` for a command family, or `osp` alone for the interactive shell. Pipe output through the OSP DSL with `|`; use `| H` for in-band DSL help."
)]
#[non_exhaustive]
pub struct Cli {
    #[arg(skip)]
    pub(crate) product_overrides: ConfigLayer,
    #[arg(skip)]
    pub(crate) product_action: Option<String>,
    /// Override the effective user name for this invocation.
    #[arg(short = 'u', long = "user")]
    pub user: Option<String>,

    /// Disable persistent REPL history and other identity-linked behavior.
    #[arg(long = "incognito", global = true)]
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

    /// Disable colors and pleasantries (equivalent to `--presentation austere`).
    #[arg(
        long = "gammel-og-bitter",
        conflicts_with = "presentation",
        global = true
    )]
    gammel_og_bitter: bool,

    /// Top-level built-in or plugin command selection.
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

impl Cli {
    /// Reports whether a built-in or external command was selected.
    pub fn has_command(&self) -> bool {
        self.command.is_some()
    }

    /// Returns the runtime source-loading options implied by global CLI flags.
    pub fn runtime_load_options(&self) -> RuntimeLoadOptions {
        runtime_load_options_from_flags(self.no_env, self.no_config_file, self.defaults_only)
    }
}

/// Top-level commands accepted by `osp`.
#[derive(Subcommand)]
pub(crate) enum Commands {
    #[command(hide = true)]
    /// Inspect, set or clear entities remembered in this REPL session.
    Context(ContextArgs),
    /// Inspect and manage plugins.
    Plugins(PluginsArgs),
    /// Run diagnostics checks.
    Doctor(DoctorArgs),
    /// Inspect and apply themes.
    Theme(ThemeArgs),
    /// Inspect and edit runtime config.
    Config(ConfigArgs),
    /// Create and manage command aliases.
    Alias(AliasArgs),
    /// Preview local terminal feedback (no service requests).
    Notifications(NotificationsArgs),
    /// Inspect or prune REPL history.
    History(HistoryArgs),
    /// Generate static shell completion for built-in commands and flags.
    Completions(CompletionsArgs),
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

/// Local terminal notification controls.
#[derive(Debug, Clone, Args)]
pub(crate) struct NotificationsArgs {
    /// The local feedback operation.
    #[command(subcommand)]
    pub command: NotificationsCommands,
}

/// Explicit local notification operations.
#[derive(Debug, Clone, Subcommand)]
pub(crate) enum NotificationsCommands {
    /// Request the same terminal bell as enabled event feedback.
    Preview,
}

impl Commands {
    // Completion scripts run before host dispatch; session commands need REPL state.
    pub(crate) fn validate_dispatch_surface(&self, is_repl: bool) -> miette::Result<()> {
        match self {
            Self::Context(_) if !is_repl => Err(miette::miette!(
                "`context` is available only in the REPL session"
            )),
            Self::Completions(_) => Err(miette::miette!(
                "`completions` is available only as a one-shot CLI command"
            )),
            _ => Ok(()),
        }
    }
}

/// Inspect, set or clear entities remembered in the current REPL session.
#[derive(Debug, Args)]
pub(crate) struct ContextArgs {
    /// Context operation to run.
    #[command(subcommand)]
    pub command: Option<ContextCommands>,
}

/// Operations for REPL session context.
#[derive(Debug, Subcommand)]
pub(crate) enum ContextCommands {
    /// Remove all remembered values, or only the given kind.
    #[command(display_order = 10)]
    Clear(ContextClearArgs),
    /// Remember a value for an entity kind.
    Set(ContextSetArgs),
}

/// Selects context values to clear.
#[derive(Debug, Args)]
pub(crate) struct ContextClearArgs {
    /// Entity kind to remove, such as `account` or `host`.
    pub kind: Option<String>,
}

/// A named entity to remember.
#[derive(Debug, Args)]
pub(crate) struct ContextSetArgs {
    /// Shared entity kind.
    pub kind: String,
    /// Entity name or identifier.
    pub value: String,
}

/// Arguments for static shell-completion generation.
#[derive(Debug, Args)]
#[command(
    after_help = "Static scripts cover built-in commands and flags. Commands loaded dynamically from native integrations or plugins remain available through the interactive REPL completion engine."
)]
pub(crate) struct CompletionsArgs {
    /// Shell whose completion script should be generated.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

pub(crate) fn render_shell_completions(args: CompletionsArgs, full: &clap::Command) -> String {
    // Hidden commands are REPL-only or internal, so the scripts leave them out.
    let mut command = clap::Command::new("osp")
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .version(env!("CARGO_PKG_VERSION"))
        .args(full.get_arguments().cloned())
        .subcommands(
            full.get_subcommands()
                .filter(|subcommand| !subcommand.is_hide_set())
                .cloned(),
        );
    let mut output = Vec::new();
    clap_complete::generate(args.shell, &mut command, "osp", &mut output);
    String::from_utf8(output).unwrap_or_default()
}

/// Parser used for inline command execution without the binary name prefix.
#[derive(Parser)]
#[command(name = "osp", about = "OSP CLI", no_binary_name = true)]
pub(crate) struct InlineCommandCli {
    /// Parsed command payload, if any.
    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

/// Hidden REPL-only command namespace.
#[derive(Debug, Args)]
pub(crate) struct ReplArgs {
    /// Hidden REPL subcommand to run.
    #[command(subcommand)]
    pub command: ReplCommands,
}

/// Hidden REPL debugging commands.
#[derive(Debug, Subcommand)]
pub(crate) enum ReplCommands {
    #[command(name = "debug-complete", hide = true)]
    /// Trace completion candidates for a partially typed line.
    DebugComplete(DebugCompleteArgs),
    #[command(name = "debug-highlight", hide = true)]
    /// Trace syntax-highlighting output for a line.
    DebugHighlight(DebugHighlightArgs),
}

/// Popup menu target to inspect through the hidden REPL debug surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum DebugMenuArg {
    /// Trace the normal completion popup.
    Completion,
}

/// Arguments for REPL completion debugging.
#[derive(Debug, Args)]
pub(crate) struct DebugCompleteArgs {
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
pub(crate) struct DebugHighlightArgs {
    /// Input line to highlight.
    #[arg(long)]
    pub line: String,
}

/// Top-level plugin command arguments.
#[derive(Debug, Args)]
pub(crate) struct PluginsArgs {
    /// Plugin management action to perform.
    #[command(subcommand)]
    pub command: PluginsCommands,
}

impl Commands {
    pub(crate) fn with_app_doctor(self) -> Self {
        match self {
            Self::Doctor(DoctorArgs {
                command: Some(DoctorCommands::External(args)),
            }) => Self::External(std::iter::once("doctor".to_owned()).chain(args).collect()),
            command => command,
        }
    }
}

/// Top-level doctor command arguments.
#[derive(Debug, Args)]
pub(crate) struct DoctorArgs {
    /// Optional narrowed diagnostic target.
    #[command(subcommand)]
    pub command: Option<DoctorCommands>,
}

/// Built-in diagnostic groups exposed through `osp doctor`.
#[derive(Debug, Subcommand)]
pub(crate) enum DoctorCommands {
    /// Run all visible diagnostics.
    #[command(display_order = 10)]
    All,
    /// Show config diagnostics.
    #[command(display_order = 11)]
    Config,
    /// Show the last REPL failure; combine with -v/-vv/-vvv for more detail.
    #[command(display_order = 12)]
    Last,
    /// Run plugin diagnostics.
    #[command(display_order = 13)]
    Plugins,
    /// Show theme diagnostics.
    #[command(display_order = 14)]
    Theme,
    /// App-registered diagnostics, dispatched through the native command registry.
    #[command(external_subcommand)]
    External(Vec<String>),
}

/// Built-in plugin management subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum PluginsCommands {
    /// List plugins.
    List,
    /// Drop cached describe metadata.
    Refresh,
    /// Enable a plugin.
    Enable {
        /// Plugin name.
        name: String,
    },
    /// Disable a plugin.
    Disable {
        /// Plugin name.
        name: String,
    },
}

/// Top-level theme command arguments.
#[derive(Debug, Args)]
pub(crate) struct ThemeArgs {
    /// Theme action to perform.
    #[command(subcommand)]
    pub command: ThemeCommands,
}

/// Theme inspection and selection commands.
#[derive(Debug, Subcommand)]
pub(crate) enum ThemeCommands {
    /// List available themes.
    #[command(display_order = 10)]
    List,
    /// Preview themes interactively, or apply a named theme for this session.
    #[command(display_order = 11)]
    Preview(ThemePreviewArgs),
    /// Set active theme.
    #[command(display_order = 12)]
    Use(ThemeUseArgs),
    /// Reset saved theme selection and colour overrides to product defaults.
    #[command(display_order = 13)]
    Reset,
}

/// Arguments for `theme preview`.
#[derive(Debug, Args)]
pub(crate) struct ThemePreviewArgs {
    /// Theme to apply for this process; omit to choose and save interactively.
    pub name: Option<String>,
}

/// Arguments for `theme use`.
#[derive(Debug, Args)]
pub(crate) struct ThemeUseArgs {
    /// Theme name to activate.
    pub name: String,
}

/// Shared config write scope arguments.
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct ConfigScopeArgs {
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
pub(crate) struct ConfigStoreArgs {
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
    #[arg(long = "permanent", alias = "save", conflicts_with_all = ["session", "config_store", "secrets"])]
    pub save: bool,
}

/// Shared config read-output arguments.
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct ConfigReadOutputArgs {
    /// Include source provenance for returned keys.
    #[arg(long = "sources")]
    pub sources: bool,

    /// Emit raw stored values without presentation formatting.
    #[arg(long = "raw")]
    pub raw: bool,
}

/// Top-level config command arguments.
#[derive(Args)]
pub(crate) struct ConfigArgs {
    /// Config action to perform.
    #[command(subcommand)]
    pub command: ConfigCommands,
}

/// Top-level alias command arguments.
#[derive(Debug, Args)]
#[command(
    after_help = "Examples:\n  osp alias add lsng 'ldap netgroup ${1} --value | P members'\n  osp alias list\n  osp alias remove lsng --save"
)]
pub(crate) struct AliasArgs {
    /// Alias action to perform.
    #[command(subcommand)]
    pub command: AliasCommands,
}

/// Commands for the friendly `alias.*` config wrapper.
#[derive(Debug, Subcommand)]
pub(crate) enum AliasCommands {
    /// List aliases available in the active profile and terminal.
    #[command(display_order = 10)]
    List(AliasListArgs),
    /// Add or replace an alias.
    #[command(display_order = 11)]
    Add(AliasAddArgs),
    /// Remove an alias from the selected config scope.
    #[command(display_order = 12)]
    Remove(AliasRemoveArgs),
}

/// Arguments for `alias list`.
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct AliasListArgs {
    /// Include the winning config source and scope for each alias.
    #[arg(long)]
    pub sources: bool,
}

/// Arguments for `alias add`.
#[derive(Debug, Args)]
pub(crate) struct AliasAddArgs {
    /// Short command name, for example `lsng`.
    pub name: String,

    /// Command template; quote templates containing spaces or `${1}` placeholders.
    pub template: String,

    /// Shared global/profile/terminal targeting flags.
    #[command(flatten)]
    pub scope: ConfigScopeArgs,

    /// Shared config/session store-selection flags.
    #[command(flatten)]
    pub store: ConfigStoreArgs,

    /// Validate and show the write without applying it.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

/// Arguments for `alias remove`.
#[derive(Debug, Args)]
pub(crate) struct AliasRemoveArgs {
    /// Short command name to remove.
    pub name: String,

    /// Shared global/profile/terminal targeting flags.
    #[command(flatten)]
    pub scope: ConfigScopeArgs,

    /// Shared config/session store-selection flags.
    #[command(flatten)]
    pub store: ConfigStoreArgs,

    /// Show the removal without applying it.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

/// Top-level history command arguments.
#[derive(Debug, Args)]
pub(crate) struct HistoryArgs {
    /// History action to perform.
    #[command(subcommand)]
    pub command: HistoryCommands,
}

/// Hidden intro command arguments.
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct IntroArgs {}

/// History management commands.
#[derive(Debug, Subcommand)]
pub(crate) enum HistoryCommands {
    /// List recent history.
    #[command(display_order = 10)]
    List(HistoryListArgs),
    /// Keep last N entries.
    #[command(display_order = 11)]
    Prune(HistoryPruneArgs),
    /// Delete history lines matching a prefix or glob.
    #[command(display_order = 12)]
    Clear(HistoryClearArgs),
}

/// Arguments for `history clear`.
#[derive(Debug, Args, Default)]
pub(crate) struct HistoryClearArgs {
    /// Match the displayed command from its start; * and ? are wildcards.
    pub pattern: String,
    /// List matching lines without deleting them.
    #[arg(long)]
    pub preview: bool,
    /// Allow deleting more than 100 lines.
    #[arg(long)]
    pub force: bool,
}

/// Arguments for `history list`.
#[derive(Debug, Args, Clone, Default)]
pub(crate) struct HistoryListArgs {
    /// Maximum number of recent matching entries to show.
    #[arg(long, value_name = "COUNT", default_value = "20")]
    pub limit: Option<std::num::NonZeroUsize>,

    /// Inclusive lower bound, for example 2026-08-11 or 2026-08-11T10:00:00Z.
    #[arg(
        long,
        value_name = "TIMESTAMP",
        help = "Inclusive lower bound. Accepts YYYY-MM-DD, YYYY-MM-DD HH:MM[:SS], or RFC3339-like input such as 2026-08-11T10:00:00Z"
    )]
    pub since: Option<String>,

    /// Inclusive upper bound, for example 2026-08-11T11:00:00Z or with +02:00.
    #[arg(
        long,
        value_name = "TIMESTAMP",
        help = "Inclusive upper bound. Accepts YYYY-MM-DD, YYYY-MM-DD HH:MM[:SS], or RFC3339-like input such as 2026-08-11T12:00:00+02:00"
    )]
    pub until: Option<String>,
}

/// Arguments for `history prune`.
#[derive(Debug, Args)]
pub(crate) struct HistoryPruneArgs {
    /// Number of recent entries to keep.
    pub keep: usize,
}

/// Configuration inspection and mutation commands.
#[derive(Subcommand)]
pub(crate) enum ConfigCommands {
    /// Edit configuration interactively; Save uses config set's default target.
    Edit(ConfigEditArgs),
    /// Show the resolved configuration view.
    #[command(display_order = 10)]
    Show(ConfigShowArgs),
    /// Read a single resolved config key.
    #[command(display_order = 11)]
    Get(ConfigGetArgs),
    /// Explain how a config key was resolved.
    #[command(display_order = 12)]
    Explain(ConfigExplainArgs),
    /// Set a config key in one or more writable stores.
    #[command(display_order = 13)]
    Set(ConfigSetArgs),
    /// Remove a config key from one or more writable stores.
    #[command(display_order = 14)]
    Unset(ConfigUnsetArgs),
    #[command(alias = "diagnostics")]
    /// Run config-specific diagnostics.
    #[command(display_order = 15)]
    Doctor,
}

/// Arguments for `config edit`.
#[derive(Debug, Args)]
pub(crate) struct ConfigEditArgs {
    /// Initially selected config key.
    pub key: Option<String>,
}

/// Arguments for `config show`.
#[derive(Debug, Args)]
pub(crate) struct ConfigShowArgs {
    /// Shared source/raw output flags for config reads.
    #[command(flatten)]
    pub output: ConfigReadOutputArgs,
}

/// Arguments for `config get`.
#[derive(Debug, Args)]
pub(crate) struct ConfigGetArgs {
    /// Config key to read.
    pub key: String,

    /// Shared source/raw output flags for config reads.
    #[command(flatten)]
    pub output: ConfigReadOutputArgs,
}

/// Arguments for `config explain`.
#[derive(Debug, Args)]
pub(crate) struct ConfigExplainArgs {
    /// Config key to explain.
    pub key: String,

    /// Reveal secret values in the explanation output.
    #[arg(long = "show-secrets")]
    pub show_secrets: bool,
}

/// Arguments for `config set`.
#[derive(Args)]
pub(crate) struct ConfigSetArgs {
    /// Config key to write.
    pub key: String,
    /// Config value to write. Multiple words are joined with spaces; secret keys prompt if omitted.
    #[arg(num_args = 1.., conflicts_with = "from_file")]
    pub value: Vec<String>,

    /// Account whose display name to set; defaults to the current user.
    #[arg(short = 'u', long = "user", value_name = "USERNAME")]
    pub display_user: Option<String>,

    /// Read the value once from a UTF-8 file, or - for stdin. Strip trailing newlines only.
    #[arg(long, value_name = "PATH", conflicts_with = "value")]
    pub from_file: Option<std::path::PathBuf>,

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
pub(crate) struct ConfigUnsetArgs {
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
    /// Resolves startup configuration without constructing command clients.
    pub fn resolve_config(
        &self,
        product_defaults: ConfigLayer,
        terminal: &str,
    ) -> miette::Result<crate::config::ResolvedConfig> {
        crate::app::bootstrap::resolve_runtime_config(
            crate::app::RuntimeConfigRequest::new(self.profile.clone(), Some(terminal))
                .with_runtime_load(self.runtime_load_options())
                .with_product_defaults(product_defaults)
                .with_session_layer(crate::app::build_cli_session_layer(self)),
        )
    }

    pub(crate) fn append_static_session_overrides(&self, layer: &mut ConfigLayer) {
        if let Some(user) = self
            .user
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            layer.set("user.name", user);
        }
        layer.extend_from_layer(&self.product_overrides);
        if self.incognito {
            layer.set("repl.history.path", "");
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
/// ```text
/// use osp_cli::cli::{Commands, ThemeCommands, parse_inline_command_tokens};
///
/// let tokens = vec![
///     "theme".to_string(),
///     "use".to_string(),
///     "dracula".to_string(),
/// ];
///
/// let command = parse_inline_command_tokens(&tokens).unwrap().unwrap();
/// match command {
///     Commands::Theme(args) => match args.command {
///         ThemeCommands::Use(selected) => {
///             assert_eq!(selected.name, "dracula");
///         }
///         _ => panic!("unexpected theme command"),
///     },
///     _ => panic!("unexpected command"),
/// }
/// ```
pub(crate) fn parse_inline_command_tokens(
    tokens: &[String],
) -> Result<Option<Commands>, clap::Error> {
    InlineCommandCli::try_parse_from(tokens.iter().map(String::as_str))
        .map(|parsed| parsed.command.map(Commands::with_app_doctor))
}

pub(crate) fn passive_command(command: clap::Command) -> clap::Command {
    let mut command = command.disable_help_subcommand(true);
    // Materialize generated help flags before turning their actions passive.
    command.build();
    command
        .ignore_errors(true)
        .mut_args(|arg| match arg.get_action() {
            clap::ArgAction::Help | clap::ArgAction::HelpShort | clap::ArgAction::HelpLong => {
                arg.action(clap::ArgAction::SetTrue)
            }
            _ => arg,
        })
        .mut_subcommands(passive_command)
}

pub(crate) fn help_parse_tokens(tokens: &[String]) -> &[String] {
    // Clap answers at the first help flag, so later words cannot pick the target.
    let options_end = tokens
        .iter()
        .position(|token| token == "--")
        .unwrap_or(tokens.len());
    tokens[..options_end]
        .iter()
        .position(|token| token == "-h" || token == "--help")
        .map_or(tokens, |index| &tokens[..=index])
}

/// Select the help target with clap's parser, then read that command's metadata.
pub(crate) fn command_help_document(
    mut command: clap::Command,
    tokens: &[String],
    scope: &crate::core::CommandScope,
) -> (crate::core::document::Document, bool, bool) {
    let name = if command.get_name() == "osp" {
        ""
    } else {
        command.get_name()
    };
    let bin_name = scope.command(name);
    command = command.clone().bin_name(if bin_name.is_empty() {
        name.to_string()
    } else {
        bin_name
    });
    let tokens = help_parse_tokens(tokens);
    command.build();
    let parser = passive_command(command.clone());
    let matches = parser.try_get_matches_from(tokens);
    let mut selected = &command;
    let mut root = true;
    let mut help_subcommand = false;
    if let Ok(matches) = &matches {
        let mut current = matches;
        while let Some((name, child_matches)) = current.subcommand() {
            current = child_matches;
            if name == "help" {
                help_subcommand = true;
                if let Ok(Some(targets)) = current.try_get_many::<String>("subcommand") {
                    for name in targets {
                        if let Some(child) = selected.find_subcommand(name) {
                            selected = child;
                            root = false;
                        }
                    }
                }
            } else if let Some(child) = selected.find_subcommand(name) {
                selected = child;
                root = false;
            }
        }
    }
    let long = help_subcommand || tokens.iter().any(|token| token == "--help");
    (
        crate::core::document::Document::from_clap_command(selected, long),
        root,
        long,
    )
}

#[derive(clap::Parser)]
#[command(name = "help", about = "Show REPL help")]
pub(crate) struct ReplHelpCommand {
    /// List all visible command paths.
    #[arg(long)]
    pub(crate) all: bool,
}

pub(crate) fn is_help_all(tokens: &[String]) -> bool {
    tokens.first().is_some_and(|word| word == "help")
        && ReplHelpCommand::try_parse_from(tokens).is_ok_and(|help| help.all)
}

pub(crate) fn repl_only_commands() -> [clap::Command; 6] {
    use clap::{Arg, ArgAction, Command, ValueHint};
    [
        <ReplHelpCommand as clap::CommandFactory>::command(),
        Command::new("last")
            .about("Replay the last successful result")
            .arg(
                Arg::new("raw")
                    .long("raw")
                    .action(ArgAction::SetTrue)
                    .help("Show the pre-pipeline result"),
            ),
        <crate::repl::dispatch::builtins::EachCommand as clap::CommandFactory>::command(),
        Command::new("source")
            .about("Run commands from files")
            .arg(
                Arg::new("file")
                    .value_name("FILE")
                    .help("Command file")
                    .num_args(0..)
                    .value_hint(ValueHint::FilePath),
            )
            .arg(
                Arg::new("ignore-errors")
                    .long("ignore-errors")
                    .action(ArgAction::SetTrue)
                    .help("Continue after a command fails"),
            ),
        <crate::repl::recording::RecordingCommand as clap::CommandFactory>::command(),
        Command::new("exit")
            .visible_alias("quit")
            .about("Exit REPL"),
    ]
}

pub(crate) fn builtin_command_names() -> Vec<String> {
    InlineCommandCli::command()
        .get_subcommands()
        .map(|command| command.get_name().to_string())
        .collect()
}

#[expect(clippy::expect_used, reason = "callers name declared builtins")]
pub(crate) fn builtin_command(name: &str) -> clap::Command {
    InlineCommandCli::command()
        .find_subcommand(name)
        .expect("known builtin command")
        .clone()
}

pub(crate) fn inline_help_document(
    tokens: &[String],
    scope: &crate::core::CommandScope,
) -> (crate::core::document::Document, bool, bool) {
    command_help_document(InlineCommandCli::command(), tokens, scope)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    mod pipeline_parse;

    mod aliases;

    #[test]
    fn internal_example_1() {
        use crate::cli::{Commands, ThemeCommands, parse_inline_command_tokens};

        let tokens = vec![
            "theme".to_string(),
            "use".to_string(),
            "dracula".to_string(),
        ];

        let command = parse_inline_command_tokens(&tokens).unwrap().unwrap();
        match command {
            Commands::Theme(args) => match args.command {
                ThemeCommands::Use(selected) => {
                    assert_eq!(selected.name, "dracula");
                }
                _ => panic!("unexpected theme command"),
            },
            _ => panic!("unexpected command"),
        }
    }

    use super::{
        AliasCommands, Cli, Commands, ConfigCommands, InlineCommandCli, RuntimeLoadOptions,
        append_appearance_overrides, parse_inline_command_tokens,
    };
    use crate::config::{ConfigLayer, ConfigValue};
    use clap::{CommandFactory, Parser};

    #[test]
    fn legacy_austere_alias_has_user_facing_help_unit() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("Disable colors and pleasantries"));
    }

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
    fn alias_commands_parse_a_small_pedagogical_surface_unit() {
        let add = Cli::try_parse_from([
            "osp",
            "alias",
            "add",
            "lsng",
            "ldap netgroup ${1} --value | P members",
            "--profile",
            "ops",
            "--save",
        ])
        .expect("alias add should parse");
        assert!(matches!(
            add.command,
            Some(Commands::Alias(args))
                if matches!(&args.command, AliasCommands::Add(add)
                    if add.name == "lsng"
                        && add.template == "ldap netgroup ${1} --value | P members"
                        && add.scope.profile.as_deref() == Some("ops")
                        && add.store.save)
        ));

        let remove = Cli::try_parse_from(["osp", "alias", "remove", "lsng", "--global"])
            .expect("alias remove should parse");
        assert!(matches!(
            remove.command,
            Some(Commands::Alias(args))
                if matches!(&args.command, AliasCommands::Remove(remove)
                    if remove.name == "lsng" && remove.scope.global)
        ));

        let help = Cli::command()
            .find_subcommand("alias")
            .expect("alias command should exist")
            .clone()
            .render_long_help()
            .to_string();
        assert!(help.contains("alias add lsng"));
        assert!(help.contains("${1}"));
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
