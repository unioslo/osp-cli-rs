pub(crate) mod commands;

use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use osp_config::{ConfigValue, ResolvedConfig};
use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_ui::theme::DEFAULT_THEME_NAME;
use osp_ui::{RenderSettings, StyleOverrides};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum OutputFormatArg {
    Auto,
    Json,
    Table,
    Md,
    Mreg,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum RenderModeArg {
    Auto,
    Plain,
    Rich,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ColorModeArg {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum UnicodeModeArg {
    Auto,
    Always,
    Never,
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(value: OutputFormatArg) -> Self {
        match value {
            OutputFormatArg::Auto => OutputFormat::Auto,
            OutputFormatArg::Json => OutputFormat::Json,
            OutputFormatArg::Table => OutputFormat::Table,
            OutputFormatArg::Md => OutputFormat::Markdown,
            OutputFormatArg::Mreg => OutputFormat::Mreg,
            OutputFormatArg::Value => OutputFormat::Value,
        }
    }
}

impl From<RenderModeArg> for RenderMode {
    fn from(value: RenderModeArg) -> Self {
        match value {
            RenderModeArg::Auto => RenderMode::Auto,
            RenderModeArg::Plain => RenderMode::Plain,
            RenderModeArg::Rich => RenderMode::Rich,
        }
    }
}

impl From<ColorModeArg> for ColorMode {
    fn from(value: ColorModeArg) -> Self {
        match value {
            ColorModeArg::Auto => ColorMode::Auto,
            ColorModeArg::Always => ColorMode::Always,
            ColorModeArg::Never => ColorMode::Never,
        }
    }
}

impl From<UnicodeModeArg> for UnicodeMode {
    fn from(value: UnicodeModeArg) -> Self {
        match value {
            UnicodeModeArg::Auto => UnicodeMode::Auto,
            UnicodeModeArg::Always => UnicodeMode::Always,
            UnicodeModeArg::Never => UnicodeMode::Never,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "osp",
    about = "OSP CLI",
    after_help = "Use `osp plugins commands` to list plugin-provided commands."
)]
pub struct Cli {
    #[arg(short = 'u', long = "user")]
    pub user: Option<String>,

    #[arg(short = 'i', long = "incognito", global = true)]
    pub incognito: bool,

    #[arg(long = "profile", global = true)]
    pub profile: Option<String>,

    #[arg(long = "format", default_value = "auto", global = true)]
    format: OutputFormatArg,

    #[arg(long = "mode", default_value = "auto", global = true)]
    mode: RenderModeArg,

    #[arg(long = "color", default_value = "auto", global = true)]
    color: ColorModeArg,

    #[arg(long = "unicode", default_value = "auto", global = true)]
    unicode: UnicodeModeArg,

    #[arg(long = "json", global = true)]
    json_legacy: bool,

    #[arg(long = "ascii", global = true)]
    ascii_legacy: bool,

    #[arg(short = 'v', long = "verbose", action = ArgAction::Count, global = true)]
    pub verbose: u8,

    #[arg(short = 'q', long = "quiet", action = ArgAction::Count, global = true)]
    pub quiet: u8,

    #[arg(short = 'd', long = "debug", action = ArgAction::Count, global = true)]
    pub debug: u8,

    #[arg(long = "plugin-dir", global = true)]
    pub plugin_dirs: Vec<PathBuf>,

    #[arg(long = "theme", global = true)]
    pub theme: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Plugins(PluginsArgs),
    Theme(ThemeArgs),
    Config(ConfigArgs),
    History(HistoryArgs),
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Debug, Parser)]
#[command(name = "osp", no_binary_name = true)]
pub struct InlineCommandCli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Parser)]
#[command(name = "osp", no_binary_name = true)]
pub struct ReplCli {
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

    #[arg(short = 'q', long = "quiet", action = ArgAction::Count)]
    pub quiet: u8,

    #[arg(short = 'd', long = "debug", action = ArgAction::Count)]
    pub debug: u8,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Args)]
pub struct PluginsArgs {
    #[command(subcommand)]
    pub command: PluginsCommands,
}

#[derive(Debug, Subcommand)]
pub enum PluginsCommands {
    List,
    Commands,
    Enable(PluginToggleArgs),
    Disable(PluginToggleArgs),
    Doctor,
}

#[derive(Debug, Args)]
pub struct ThemeArgs {
    #[command(subcommand)]
    pub command: ThemeCommands,
}

#[derive(Debug, Subcommand)]
pub enum ThemeCommands {
    List,
    Show(ThemeShowArgs),
    Use(ThemeUseArgs),
}

#[derive(Debug, Args)]
pub struct ThemeShowArgs {
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct ThemeUseArgs {
    pub name: String,
}

#[derive(Debug, Args)]
pub struct PluginToggleArgs {
    pub plugin_id: String,
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    #[command(subcommand)]
    pub command: HistoryCommands,
}

#[derive(Debug, Subcommand)]
pub enum HistoryCommands {
    List,
    Prune(HistoryPruneArgs),
    Clear,
}

#[derive(Debug, Args)]
pub struct HistoryPruneArgs {
    pub keep: usize,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    Show(ConfigShowArgs),
    Get(ConfigGetArgs),
    Explain(ConfigExplainArgs),
    Set(ConfigSetArgs),
    Diagnostics,
}

#[derive(Debug, Args)]
pub struct ConfigShowArgs {
    #[arg(long = "sources")]
    pub sources: bool,

    #[arg(long = "raw")]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct ConfigGetArgs {
    pub key: String,

    #[arg(long = "sources")]
    pub sources: bool,

    #[arg(long = "raw")]
    pub raw: bool,
}

#[derive(Debug, Args)]
pub struct ConfigExplainArgs {
    pub key: String,

    #[arg(long = "show-secrets")]
    pub show_secrets: bool,
}

#[derive(Debug, Args)]
pub struct ConfigSetArgs {
    pub key: String,
    pub value: String,

    #[arg(long = "global", conflicts_with_all = ["profile", "profile_all"])]
    pub global: bool,

    #[arg(long = "profile", conflicts_with = "profile_all")]
    pub profile: Option<String>,

    #[arg(long = "profile-all", conflicts_with = "profile")]
    pub profile_all: bool,

    #[arg(
        long = "terminal",
        num_args = 0..=1,
        default_missing_value = "__current__"
    )]
    pub terminal: Option<String>,

    #[arg(long = "session", conflicts_with_all = ["config_store", "secrets", "save"])]
    pub session: bool,

    #[arg(long = "config", conflicts_with_all = ["session", "secrets"])]
    pub config_store: bool,

    #[arg(long = "secrets", conflicts_with_all = ["session", "config_store"])]
    pub secrets: bool,

    #[arg(long = "save", conflicts_with_all = ["session", "config_store", "secrets"])]
    pub save: bool,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "yes")]
    pub yes: bool,

    #[arg(long = "explain")]
    pub explain: bool,
}

impl Cli {
    pub fn render_settings(&self) -> RenderSettings {
        let format = if self.json_legacy {
            OutputFormat::Json
        } else {
            self.format.into()
        };
        let unicode = if self.ascii_legacy {
            UnicodeMode::Never
        } else {
            self.unicode.into()
        };

        RenderSettings {
            format,
            mode: self.mode.into(),
            color: self.color.into(),
            unicode,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: StyleOverrides::default(),
        }
    }

    pub fn seed_render_settings_from_config(
        &self,
        settings: &mut RenderSettings,
        config: &ResolvedConfig,
    ) {
        if !self.json_legacy
            && matches!(self.format, OutputFormatArg::Auto)
            && let Some(value) = config.get_string("ui.format")
            && let Some(parsed) = parse_output_format(value)
        {
            settings.format = parsed;
        }

        if matches!(self.mode, RenderModeArg::Auto)
            && let Some(value) = config.get_string("ui.mode")
            && let Some(parsed) = parse_render_mode(value)
        {
            settings.mode = parsed;
        }

        if !self.ascii_legacy
            && matches!(self.unicode, UnicodeModeArg::Auto)
            && let Some(value) = config.get_string("ui.unicode.mode")
            && let Some(parsed) = parse_unicode_mode(value)
        {
            settings.unicode = parsed;
        }

        if matches!(self.color, ColorModeArg::Auto)
            && let Some(value) = config.get_string("ui.color.mode")
            && let Some(parsed) = parse_color_mode(value)
        {
            settings.color = parsed;
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

        settings.style_overrides = StyleOverrides {
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
        };
    }

    pub fn selected_theme_name(&self, config: &ResolvedConfig) -> String {
        self.theme
            .as_deref()
            .or_else(|| config.get_string("theme.name"))
            .unwrap_or(DEFAULT_THEME_NAME)
            .to_string()
    }
}

fn parse_output_format(value: &str) -> Option<OutputFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(OutputFormat::Auto),
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

pub fn parse_inline_command_tokens(tokens: &[String]) -> Result<Option<Commands>, clap::Error> {
    InlineCommandCli::try_parse_from(tokens.iter().map(String::as_str)).map(|parsed| parsed.command)
}

pub fn parse_repl_tokens(tokens: &[String]) -> Result<ReplCli, clap::Error> {
    ReplCli::try_parse_from(tokens.iter().map(String::as_str))
}
