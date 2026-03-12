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
//! - [`Cli`] for the binary-facing grammar
//! - [`InlineCommandCli`] for command text that omits the binary name
//! - [`parse_command_text_with_aliases`] when you need alias-aware command plus
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
use crate::config::{ConfigLayer, ConfigValue, ResolvedConfig, RuntimeLoadOptions};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::ui::chrome::{RuledSectionPolicy, SectionFrameStyle};
use crate::ui::theme::DEFAULT_THEME_NAME;
use crate::ui::{
    GuideDefaultFormat, HelpTableChrome, RenderSettings, StyleOverrides, TableBorderStyle,
    TableOverflow,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::ui::presentation::UiPresentation;

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
        RuntimeLoadOptions::new()
            .with_env(!self.no_env)
            .with_config_file(!self.no_config_file)
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
#[derive(Debug, Args)]
pub struct PluginCommandStateArgs {
    /// Command name to enable or disable.
    pub command: String,

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

/// Arguments for clearing persisted command state.
#[derive(Debug, Args)]
pub struct PluginCommandClearArgs {
    /// Command name whose state should be cleared.
    pub command: String,

    /// Clear global state instead of profile-scoped state.
    #[arg(long = "global", conflicts_with = "profile")]
    pub global: bool,

    /// Clear state for a named profile.
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

/// Arguments for selecting a provider implementation for a command.
#[derive(Debug, Args)]
pub struct PluginProviderSelectArgs {
    /// Command name whose provider should be selected.
    pub command: String,
    /// Plugin identifier to bind to the command.
    pub plugin_id: String,

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

/// Arguments for clearing a provider selection.
#[derive(Debug, Args)]
pub struct PluginProviderClearArgs {
    /// Command name whose provider binding should be removed.
    pub command: String,

    /// Clear the global binding instead of a profile-scoped binding.
    #[arg(long = "global", conflicts_with = "profile")]
    pub global: bool,

    /// Clear the binding for a named profile.
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
    /// Include source provenance for each returned key.
    #[arg(long = "sources")]
    pub sources: bool,

    /// Emit raw stored values without presentation formatting.
    #[arg(long = "raw")]
    pub raw: bool,
}

/// Arguments for `config get`.
#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    /// Config key to read.
    pub key: String,

    /// Include source provenance for the resolved key.
    #[arg(long = "sources")]
    pub sources: bool,

    /// Emit the raw stored value without presentation formatting.
    #[arg(long = "raw")]
    pub raw: bool,
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

    /// Remove the key from the global store instead of a profile-scoped store.
    #[arg(long = "global", conflicts_with_all = ["profile", "profile_all"])]
    pub global: bool,

    /// Remove the key from a single named profile.
    #[arg(long = "profile", conflicts_with = "profile_all")]
    pub profile: Option<String>,

    /// Remove the key from every known profile store.
    #[arg(long = "profile-all", conflicts_with = "profile")]
    pub profile_all: bool,

    /// Remove the key from a terminal-scoped store, or the current terminal when omitted.
    #[arg(
        long = "terminal",
        num_args = 0..=1,
        default_missing_value = "__current__"
    )]
    pub terminal: Option<String>,

    /// Remove the key only from the current in-memory session.
    #[arg(long = "session", conflicts_with_all = ["config_store", "secrets", "save"])]
    pub session: bool,

    /// Force the regular config store as the source to edit.
    #[arg(long = "config", conflicts_with_all = ["session", "secrets"])]
    pub config_store: bool,

    /// Force the secrets store as the source to edit.
    #[arg(long = "secrets", conflicts_with_all = ["session", "config_store"])]
    pub secrets: bool,

    /// Persist the change immediately after validation.
    #[arg(long = "save", conflicts_with_all = ["session", "config_store", "secrets"])]
    pub save: bool,

    /// Show the resolved removal plan without applying it.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

impl Cli {
    /// Returns the default render settings for this CLI invocation.
    pub fn render_settings(&self) -> RenderSettings {
        default_render_settings()
    }

    /// Applies config-backed render settings to an existing settings struct.
    pub fn seed_render_settings_from_config(
        &self,
        settings: &mut RenderSettings,
        config: &ResolvedConfig,
    ) {
        apply_render_settings_from_config(settings, config);
    }

    /// Returns the theme name selected by CLI override or resolved config.
    pub fn selected_theme_name(&self, config: &ResolvedConfig) -> String {
        self.theme
            .as_deref()
            .or_else(|| config.get_string("theme.name"))
            .unwrap_or(DEFAULT_THEME_NAME)
            .to_string()
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
        if self.incognito {
            layer.set("repl.history.enabled", false);
        }
        if let Some(theme) = self
            .theme
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            layer.set("theme.name", theme);
        }
        if self.gammel_og_bitter {
            layer.set("ui.presentation", UiPresentation::Austere.as_config_value());
        } else if let Some(presentation) = self.presentation {
            layer.set(
                "ui.presentation",
                UiPresentation::from(presentation).as_config_value(),
            );
        }
    }
}

pub(crate) fn default_render_settings() -> RenderSettings {
    RenderSettings::default()
}

pub(crate) fn apply_render_settings_from_config(
    settings: &mut RenderSettings,
    config: &ResolvedConfig,
) {
    if let Some(value) = config.get_string("ui.format")
        && let Some(parsed) = parse_output_format(value)
    {
        settings.format = parsed;
    }

    if let Some(value) = config.get_string("ui.mode")
        && let Some(parsed) = parse_render_mode(value)
    {
        settings.mode = parsed;
    }

    if let Some(value) = config.get_string("ui.unicode.mode")
        && let Some(parsed) = parse_unicode_mode(value)
    {
        settings.unicode = parsed;
    }

    if let Some(value) = config.get_string("ui.color.mode")
        && let Some(parsed) = parse_color_mode(value)
    {
        settings.color = parsed;
    }

    if let Some(value) = config.get_string("ui.chrome.frame")
        && let Some(parsed) = SectionFrameStyle::parse(value)
    {
        settings.chrome_frame = parsed;
    }

    if let Some(value) = config.get_string("ui.chrome.rule_policy")
        && let Some(parsed) = RuledSectionPolicy::parse(value)
    {
        settings.ruled_section_policy = parsed;
    }

    if let Some(value) = config.get_string("ui.guide.default_format")
        && let Some(parsed) = GuideDefaultFormat::parse(value)
    {
        settings.guide_default_format = parsed;
    }

    if settings.width.is_none() {
        match config.get("ui.width").map(ConfigValue::reveal) {
            Some(ConfigValue::Integer(width)) if *width > 0 => {
                settings.width = Some(*width as usize);
            }
            Some(ConfigValue::String(raw)) => {
                if let Ok(width) = raw.trim().parse::<usize>()
                    && width > 0
                {
                    settings.width = Some(width);
                }
            }
            _ => {}
        }
    }

    sync_render_settings_from_config(settings, config);
}

pub(crate) fn sync_render_settings_from_config(
    settings: &mut RenderSettings,
    config: &ResolvedConfig,
) {
    if let Some(value) = config_int(config, "ui.margin")
        && value >= 0
    {
        settings.margin = value as usize;
    }

    if let Some(value) = config_int(config, "ui.indent")
        && value > 0
    {
        settings.indent_size = value as usize;
    }

    if let Some(value) = config_int(config, "ui.short_list_max")
        && value > 0
    {
        settings.short_list_max = value as usize;
    }

    if let Some(value) = config_int(config, "ui.medium_list_max")
        && value > 0
    {
        settings.medium_list_max = value as usize;
    }

    if let Some(value) = config_int(config, "ui.grid_padding")
        && value > 0
    {
        settings.grid_padding = value as usize;
    }

    if let Some(value) = config_int(config, "ui.grid_columns") {
        settings.grid_columns = if value > 0 {
            Some(value as usize)
        } else {
            None
        };
    }

    if let Some(value) = config_int(config, "ui.column_weight")
        && value > 0
    {
        settings.column_weight = value as usize;
    }

    if let Some(value) = config_int(config, "ui.mreg.stack_min_col_width")
        && value > 0
    {
        settings.mreg_stack_min_col_width = value as usize;
    }

    if let Some(value) = config_int(config, "ui.mreg.stack_overflow_ratio")
        && value >= 100
    {
        settings.mreg_stack_overflow_ratio = value as usize;
    }

    if let Some(value) = config.get_string("ui.table.overflow")
        && let Some(parsed) = TableOverflow::parse(value)
    {
        settings.table_overflow = parsed;
    }

    if let Some(value) = config.get_string("ui.table.border")
        && let Some(parsed) = TableBorderStyle::parse(value)
    {
        settings.table_border = parsed;
    }

    if let Some(value) = config.get_string("ui.help.table_chrome")
        && let Some(parsed) = HelpTableChrome::parse(value)
    {
        settings.help_chrome.table_chrome = parsed;
    }

    settings.help_chrome.entry_indent = config_usize_override(config, "ui.help.entry_indent");
    settings.help_chrome.entry_gap = config_usize_override(config, "ui.help.entry_gap");
    settings.help_chrome.section_spacing = config_usize_override(config, "ui.help.section_spacing");

    settings.style_overrides = StyleOverrides {
        text: config_non_empty_string(config, "color.text"),
        key: config_non_empty_string(config, "color.key"),
        muted: config_non_empty_string(config, "color.text.muted"),
        table_header: config_non_empty_string(config, "color.table.header"),
        mreg_key: config_non_empty_string(config, "color.mreg.key"),
        value: config_non_empty_string(config, "color.value"),
        number: config_non_empty_string(config, "color.value.number"),
        bool_true: config_non_empty_string(config, "color.value.bool_true"),
        bool_false: config_non_empty_string(config, "color.value.bool_false"),
        null_value: config_non_empty_string(config, "color.value.null"),
        ipv4: config_non_empty_string(config, "color.value.ipv4"),
        ipv6: config_non_empty_string(config, "color.value.ipv6"),
        panel_border: config_non_empty_string(config, "color.panel.border")
            .or_else(|| config_non_empty_string(config, "color.border")),
        panel_title: config_non_empty_string(config, "color.panel.title"),
        code: config_non_empty_string(config, "color.code"),
        json_key: config_non_empty_string(config, "color.json.key"),
        message_error: config_non_empty_string(config, "color.message.error"),
        message_warning: config_non_empty_string(config, "color.message.warning"),
        message_success: config_non_empty_string(config, "color.message.success"),
        message_info: config_non_empty_string(config, "color.message.info"),
        message_trace: config_non_empty_string(config, "color.message.trace"),
    };
}

fn parse_output_format(value: &str) -> Option<OutputFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(OutputFormat::Auto),
        "guide" => Some(OutputFormat::Guide),
        "json" => Some(OutputFormat::Json),
        "table" => Some(OutputFormat::Table),
        "md" | "markdown" => Some(OutputFormat::Markdown),
        "mreg" => Some(OutputFormat::Mreg),
        "value" => Some(OutputFormat::Value),
        _ => None,
    }
}

fn parse_render_mode(value: &str) -> Option<RenderMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(RenderMode::Auto),
        "plain" => Some(RenderMode::Plain),
        "rich" => Some(RenderMode::Rich),
        _ => None,
    }
}

fn parse_color_mode(value: &str) -> Option<ColorMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ColorMode::Auto),
        "always" => Some(ColorMode::Always),
        "never" => Some(ColorMode::Never),
        _ => None,
    }
}

fn parse_unicode_mode(value: &str) -> Option<UnicodeMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(UnicodeMode::Auto),
        "always" => Some(UnicodeMode::Always),
        "never" => Some(UnicodeMode::Never),
        _ => None,
    }
}

fn config_int(config: &ResolvedConfig, key: &str) -> Option<i64> {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) => Some(*value),
        Some(ConfigValue::String(raw)) => raw.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn config_non_empty_string(config: &ResolvedConfig, key: &str) -> Option<String> {
    config
        .get_string(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn config_usize_override(config: &ResolvedConfig, key: &str) -> Option<usize> {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) if *value >= 0 => Some(*value as usize),
        Some(ConfigValue::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.eq_ignore_ascii_case("inherit") || trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<usize>().ok()
            }
        }
        _ => None,
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
        Cli, ColorMode, Commands, ConfigCommands, InlineCommandCli, OutputFormat, RenderMode,
        RuntimeLoadOptions, SectionFrameStyle, TableBorderStyle, TableOverflow, UnicodeMode,
        apply_render_settings_from_config, config_int, config_non_empty_string,
        config_usize_override, parse_color_mode, parse_inline_command_tokens, parse_output_format,
        parse_render_mode, parse_unicode_mode,
    };
    use crate::config::{ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions};
    use crate::ui::presentation::build_presentation_defaults_layer;
    use crate::ui::{GuideDefaultFormat, RenderSettings};
    use clap::Parser;

    fn resolved(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let options = ResolveOptions::default().with_terminal("cli");
        let base = resolver
            .resolve(options.clone())
            .expect("base test config should resolve");
        resolver.set_presentation(build_presentation_defaults_layer(&base));
        resolver
            .resolve(options)
            .expect("test config should resolve")
    }

    fn resolved_with_session(
        defaults_entries: &[(&str, &str)],
        session_entries: &[(&str, &str)],
    ) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in defaults_entries {
            defaults.set(*key, *value);
        }

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);

        let mut session = ConfigLayer::default();
        for (key, value) in session_entries {
            session.set(*key, *value);
        }
        resolver.set_session(session);

        let options = ResolveOptions::default().with_terminal("cli");
        let base = resolver
            .resolve(options.clone())
            .expect("base test config should resolve");
        resolver.set_presentation(build_presentation_defaults_layer(&base));
        resolver
            .resolve(options)
            .expect("test config should resolve")
    }

    #[test]
    fn parse_mode_and_config_helpers_normalize_strings_blanks_and_integers_unit() {
        assert_eq!(parse_output_format(" guide "), Some(OutputFormat::Guide));
        assert_eq!(
            parse_output_format(" markdown "),
            Some(OutputFormat::Markdown)
        );
        assert_eq!(parse_render_mode(" Rich "), Some(RenderMode::Rich));
        assert_eq!(parse_color_mode(" NEVER "), Some(ColorMode::Never));
        assert_eq!(parse_unicode_mode(" always "), Some(UnicodeMode::Always));
        assert_eq!(parse_output_format("yaml"), None);

        let config = resolved(&[
            ("ui.width", "120"),
            ("color.text", "  "),
            ("ui.margin", "3"),
        ]);

        assert_eq!(config_int(&config, "ui.width"), Some(120));
        assert_eq!(config_int(&config, "ui.margin"), Some(3));
        assert_eq!(config_non_empty_string(&config, "color.text"), None);
    }

    #[test]
    fn render_settings_apply_presentation_defaults_explicit_overrides_and_help_spacing_unit() {
        let config = resolved_with_session(
            &[("ui.width", "88")],
            &[
                ("ui.chrome.frame", "round"),
                ("ui.chrome.rule_policy", "stacked"),
                ("ui.table.border", "square"),
                ("ui.table.overflow", "wrap"),
            ],
        );
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(settings.width, Some(88));
        assert_eq!(settings.chrome_frame, SectionFrameStyle::Round);
        assert_eq!(
            settings.ruled_section_policy,
            crate::ui::RuledSectionPolicy::Shared
        );
        assert_eq!(settings.table_border, TableBorderStyle::Square);
        assert_eq!(settings.table_overflow, TableOverflow::Wrap);

        let config = resolved(&[("ui.presentation", "expressive")]);
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(settings.chrome_frame, SectionFrameStyle::TopBottom);
        assert_eq!(settings.table_border, TableBorderStyle::Round);

        let config = resolved_with_session(
            &[("ui.presentation", "expressive")],
            &[("ui.chrome.frame", "square"), ("ui.table.border", "none")],
        );
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(settings.chrome_frame, SectionFrameStyle::Square);
        assert_eq!(settings.table_border, TableBorderStyle::None);

        let config = resolved(&[("ui.guide.default_format", "inherit")]);
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(settings.guide_default_format, GuideDefaultFormat::Inherit);

        let config = resolved(&[
            ("ui.help.entry_indent", "4"),
            ("ui.help.entry_gap", "3"),
            ("ui.help.section_spacing", "inherit"),
        ]);
        let mut settings = RenderSettings::test_plain(OutputFormat::Guide);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(
            config_usize_override(&config, "ui.help.entry_indent"),
            Some(4)
        );
        assert_eq!(settings.help_chrome.entry_indent, Some(4));
        assert_eq!(settings.help_chrome.entry_gap, Some(3));
        assert_eq!(settings.help_chrome.section_spacing, None);
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
    fn cli_runtime_load_options_and_inline_parser_follow_disable_flags_unit() {
        let cli = Cli::parse_from(["osp", "--no-env", "--no-config-file", "theme", "list"]);
        assert_eq!(
            cli.runtime_load_options(),
            RuntimeLoadOptions::new()
                .with_env(false)
                .with_config_file(false)
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
}
