pub(crate) mod commands;

use crate::osp_config::{ConfigLayer, ConfigValue, ResolvedConfig, RuntimeLoadOptions};
use crate::osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::osp_ui::chrome::SectionFrameStyle;
use crate::osp_ui::theme::DEFAULT_THEME_NAME;
use crate::osp_ui::{
    RenderRuntime, RenderSettings, StyleOverrides, TableBorderStyle, TableOverflow,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use crate::osp_cli::ui_presentation::{
    UiPresentation, apply_presentation_to_render_settings, is_builtin_default,
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

#[derive(Debug, Parser)]
#[command(
    name = "osp",
    version = env!("CARGO_PKG_VERSION"),
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

    #[arg(long = "no-env", global = true)]
    pub no_env: bool,

    #[arg(long = "no-config-file", alias = "no-config", global = true)]
    pub no_config_file: bool,

    #[arg(long = "plugin-dir", global = true)]
    pub plugin_dirs: Vec<PathBuf>,

    #[arg(long = "theme", global = true)]
    pub theme: Option<String>,

    #[arg(long = "presentation", global = true)]
    presentation: Option<PresentationArg>,

    #[arg(
        long = "gammel-og-bitter",
        conflicts_with = "presentation",
        global = true
    )]
    gammel_og_bitter: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

impl Cli {
    pub fn runtime_load_options(&self) -> RuntimeLoadOptions {
        RuntimeLoadOptions {
            include_env: !self.no_env,
            include_config_file: !self.no_config_file,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Plugins(PluginsArgs),
    Doctor(DoctorArgs),
    Theme(ThemeArgs),
    Config(ConfigArgs),
    History(HistoryArgs),
    #[command(hide = true)]
    Repl(ReplArgs),
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Debug, Parser)]
#[command(name = "osp", no_binary_name = true)]
pub struct InlineCommandCli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Args)]
pub struct ReplArgs {
    #[command(subcommand)]
    pub command: ReplCommands,
}

#[derive(Debug, Subcommand)]
pub enum ReplCommands {
    #[command(name = "debug-complete", hide = true)]
    DebugComplete(DebugCompleteArgs),
    #[command(name = "debug-highlight", hide = true)]
    DebugHighlight(DebugHighlightArgs),
}

#[derive(Debug, Args)]
pub struct DebugCompleteArgs {
    #[arg(long)]
    pub line: String,

    #[arg(long)]
    pub cursor: Option<usize>,

    #[arg(long, default_value_t = 80)]
    pub width: u16,

    #[arg(long, default_value_t = 24)]
    pub height: u16,

    #[arg(long = "step")]
    pub steps: Vec<String>,

    #[arg(long = "menu-ansi", default_value_t = false)]
    pub menu_ansi: bool,

    #[arg(long = "menu-unicode", default_value_t = false)]
    pub menu_unicode: bool,
}

#[derive(Debug, Args)]
pub struct DebugHighlightArgs {
    #[arg(long)]
    pub line: String,
}

#[derive(Debug, Args)]
pub struct PluginsArgs {
    #[command(subcommand)]
    pub command: PluginsCommands,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[command(subcommand)]
    pub command: Option<DoctorCommands>,
}

#[derive(Debug, Subcommand)]
pub enum DoctorCommands {
    All,
    Config,
    Last,
    Plugins,
    Theme,
}

#[derive(Debug, Subcommand)]
pub enum PluginsCommands {
    List,
    Commands,
    Config(PluginConfigArgs),
    Refresh,
    Enable(PluginToggleArgs),
    Disable(PluginToggleArgs),
    SelectProvider(PluginProviderSelectArgs),
    ClearProvider(PluginProviderClearArgs),
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
pub struct PluginProviderSelectArgs {
    pub command: String,
    pub plugin_id: String,
}

#[derive(Debug, Args)]
pub struct PluginProviderClearArgs {
    pub command: String,
}

#[derive(Debug, Args)]
pub struct PluginConfigArgs {
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
    Unset(ConfigUnsetArgs),
    #[command(alias = "diagnostics")]
    Doctor,
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

#[derive(Debug, Args)]
pub struct ConfigUnsetArgs {
    pub key: String,

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
}

impl Cli {
    pub fn render_settings(&self) -> RenderSettings {
        default_render_settings()
    }

    pub fn seed_render_settings_from_config(
        &self,
        settings: &mut RenderSettings,
        config: &ResolvedConfig,
    ) {
        apply_render_settings_from_config(settings, config);
    }

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
    RenderSettings {
        format: OutputFormat::Auto,
        mode: RenderMode::Auto,
        color: ColorMode::Auto,
        unicode: UnicodeMode::Auto,
        width: None,
        margin: 0,
        indent_size: 2,
        short_list_max: 1,
        medium_list_max: 5,
        grid_padding: 4,
        grid_columns: None,
        column_weight: 3,
        table_overflow: TableOverflow::Clip,
        table_border: TableBorderStyle::Square,
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        theme: None,
        style_overrides: StyleOverrides::default(),
        chrome_frame: SectionFrameStyle::Top,
        runtime: RenderRuntime::default(),
    }
}

pub(crate) fn apply_render_settings_from_config(
    settings: &mut RenderSettings,
    config: &ResolvedConfig,
) {
    apply_presentation_to_render_settings(settings, config);

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

    if !is_builtin_default(config, "ui.chrome.frame")
        && let Some(value) = config.get_string("ui.chrome.frame")
        && let Some(parsed) = SectionFrameStyle::parse(value)
    {
        settings.chrome_frame = parsed;
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

    if !is_builtin_default(config, "ui.table.border")
        && let Some(value) = config.get_string("ui.table.border")
        && let Some(parsed) = TableBorderStyle::parse(value)
    {
        settings.table_border = parsed;
    }

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

#[cfg(test)]
mod tests {
    use super::{
        Cli, ColorMode, Commands, ConfigCommands, InlineCommandCli, OutputFormat, RenderMode,
        RuntimeLoadOptions, SectionFrameStyle, TableBorderStyle, TableOverflow, UnicodeMode,
        apply_render_settings_from_config, config_int, config_non_empty_string, parse_color_mode,
        parse_inline_command_tokens, parse_output_format, parse_render_mode, parse_unicode_mode,
    };
    use crate::osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::osp_ui::RenderSettings;
    use clap::Parser;

    fn resolved(entries: &[(&str, &str)]) -> crate::osp_config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver
            .resolve(ResolveOptions::default().with_terminal("cli"))
            .expect("test config should resolve")
    }

    fn resolved_with_session(
        defaults_entries: &[(&str, &str)],
        session_entries: &[(&str, &str)],
    ) -> crate::osp_config::ResolvedConfig {
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

        resolver
            .resolve(ResolveOptions::default().with_terminal("cli"))
            .expect("test config should resolve")
    }

    #[test]
    fn parse_mode_helpers_accept_aliases_and_trim_input_unit() {
        assert_eq!(
            parse_output_format(" markdown "),
            Some(OutputFormat::Markdown)
        );
        assert_eq!(parse_render_mode(" Rich "), Some(RenderMode::Rich));
        assert_eq!(parse_color_mode(" NEVER "), Some(ColorMode::Never));
        assert_eq!(parse_unicode_mode(" always "), Some(UnicodeMode::Always));
        assert_eq!(parse_output_format("yaml"), None);
    }

    #[test]
    fn config_helpers_ignore_blank_strings_and_parse_integers_unit() {
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
    fn render_settings_apply_low_level_ui_overrides_unit() {
        let config = resolved_with_session(
            &[("ui.width", "88")],
            &[
                ("ui.chrome.frame", "round"),
                ("ui.table.border", "square"),
                ("ui.table.overflow", "wrap"),
            ],
        );
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(settings.width, Some(88));
        assert_eq!(settings.chrome_frame, SectionFrameStyle::Round);
        assert_eq!(settings.table_border, TableBorderStyle::Square);
        assert_eq!(settings.table_overflow, TableOverflow::Wrap);
    }

    #[test]
    fn presentation_seeds_runtime_chrome_and_table_defaults_unit() {
        let config = resolved(&[("ui.presentation", "expressive")]);
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(settings.chrome_frame, SectionFrameStyle::TopBottom);
        assert_eq!(settings.table_border, TableBorderStyle::Round);
    }

    #[test]
    fn explicit_low_level_overrides_beat_presentation_defaults_unit() {
        let config = resolved_with_session(
            &[("ui.presentation", "expressive")],
            &[("ui.chrome.frame", "square"), ("ui.table.border", "none")],
        );
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_settings_from_config(&mut settings, &config);

        assert_eq!(settings.chrome_frame, SectionFrameStyle::Square);
        assert_eq!(settings.table_border, TableBorderStyle::None);
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
    fn cli_runtime_load_options_follow_disable_flags_unit() {
        let cli = Cli::parse_from(["osp", "--no-env", "--no-config-file", "theme", "list"]);
        assert_eq!(
            cli.runtime_load_options(),
            RuntimeLoadOptions {
                include_env: false,
                include_config_file: false,
            }
        );

        let inline = InlineCommandCli::try_parse_from(["theme", "list"])
            .expect("inline command should parse");
        assert!(matches!(inline.command, Some(Commands::Theme(_))));
    }
}
