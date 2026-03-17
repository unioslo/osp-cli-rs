//! Builtin config defaults live in this file.
//!
//! Do not split builtin defaults across modules unless this file becomes
//! impossible to understand.
//!
//! Keep different kinds of defaults in separate sections here instead:
//!
//! - semantic runtime fallbacks used directly by callers
//! - literal builtin config defaults
//! - computed defaults derived from runtime inputs

use super::ConfigLayer;
use super::runtime::RuntimeEnvironment;

/// Default logical profile name used when no profile override is active.
pub const DEFAULT_PROFILE_NAME: &str = "default";
/// Default maximum number of REPL history entries to keep.
pub const DEFAULT_REPL_HISTORY_MAX_ENTRIES: i64 = 1000;
/// Default toggle for persistent REPL history.
pub const DEFAULT_REPL_HISTORY_ENABLED: bool = true;
/// Default toggle for deduplicating REPL history entries.
pub const DEFAULT_REPL_HISTORY_DEDUPE: bool = true;
/// Default toggle for profile-scoped REPL history storage.
pub const DEFAULT_REPL_HISTORY_PROFILE_SCOPED: bool = true;
/// Default maximum number of rows shown in the REPL history search menu.
pub const DEFAULT_REPL_HISTORY_MENU_ROWS: i64 = 5;
/// Default upper bound for cached session results.
pub const DEFAULT_SESSION_CACHE_MAX_RESULTS: i64 = 64;
/// Default debug verbosity level.
pub const DEFAULT_DEBUG_LEVEL: i64 = 0;
/// Default toggle for file logging.
pub const DEFAULT_LOG_FILE_ENABLED: bool = false;
/// Default log level used for file logging.
pub const DEFAULT_LOG_FILE_LEVEL: &str = "warn";
/// Default render width hint.
pub const DEFAULT_UI_WIDTH: i64 = 72;
/// Default left margin for rendered output.
pub const DEFAULT_UI_MARGIN: i64 = 0;
/// Default indentation width for nested output.
pub const DEFAULT_UI_INDENT: i64 = 2;
/// Default presentation preset name.
pub const DEFAULT_UI_PRESENTATION: &str = "expressive";
/// Default semantic guide-format preference.
pub const DEFAULT_UI_GUIDE_DEFAULT_FORMAT: &str = "guide";
/// Default grouped-message layout mode.
pub const DEFAULT_UI_MESSAGES_LAYOUT: &str = "grouped";
/// Default section chrome frame style.
pub const DEFAULT_UI_CHROME_FRAME: &str = "top";
/// Default rule-sharing policy for sibling section chrome.
pub const DEFAULT_UI_CHROME_RULE_POLICY: &str = "shared";
/// Default table border style.
pub const DEFAULT_UI_TABLE_BORDER: &str = "square";
/// Default REPL intro mode.
pub const DEFAULT_REPL_INTRO: &str = "full";
/// Default threshold for rendering short lists compactly.
pub const DEFAULT_UI_SHORT_LIST_MAX: i64 = 1;
/// Default threshold for rendering medium lists before expanding further.
pub const DEFAULT_UI_MEDIUM_LIST_MAX: i64 = 5;
/// Default grid column padding.
pub const DEFAULT_UI_GRID_PADDING: i64 = 4;
/// Default adaptive grid column weight.
pub const DEFAULT_UI_COLUMN_WEIGHT: i64 = 3;
/// Default minimum width before MREG output stacks columns.
pub const DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH: i64 = 10;
/// Default threshold for stacked MREG overflow behavior.
pub const DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO: i64 = 200;
/// Default table overflow strategy.
pub const DEFAULT_UI_TABLE_OVERFLOW: &str = "clip";

const DEFAULT_EXTENSIONS_PLUGINS_TIMEOUT_MS: i64 =
    crate::plugin::DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS as i64;

const EMPTY_STYLE_OVERRIDE_KEYS: &[&str] = &[
    "color.text",
    "color.text.muted",
    "color.key",
    "color.border",
    "color.prompt.text",
    "color.prompt.command",
    "color.table.header",
    "color.mreg.key",
    "color.value",
    "color.value.number",
    "color.value.bool_true",
    "color.value.bool_false",
    "color.value.null",
    "color.value.ipv4",
    "color.value.ipv6",
    "color.panel.border",
    "color.panel.title",
    "color.code",
    "color.json.key",
];

const LITERAL_DEFAULTS: &[LiteralDefault] = &[
    LiteralDefault::string("profile.default", DEFAULT_PROFILE_NAME),
    LiteralDefault::string("repl.input_mode", "auto"),
    LiteralDefault::bool("repl.simple_prompt", false),
    LiteralDefault::string("repl.shell_indicator", "[{shell}]"),
    LiteralDefault::string("repl.intro", DEFAULT_REPL_INTRO),
    LiteralDefault::int("repl.history.max_entries", DEFAULT_REPL_HISTORY_MAX_ENTRIES),
    LiteralDefault::bool("repl.history.enabled", DEFAULT_REPL_HISTORY_ENABLED),
    LiteralDefault::bool("repl.history.dedupe", DEFAULT_REPL_HISTORY_DEDUPE),
    LiteralDefault::bool(
        "repl.history.profile_scoped",
        DEFAULT_REPL_HISTORY_PROFILE_SCOPED,
    ),
    LiteralDefault::int("repl.history.menu_rows", DEFAULT_REPL_HISTORY_MENU_ROWS),
    LiteralDefault::int(
        "session.cache.max_results",
        DEFAULT_SESSION_CACHE_MAX_RESULTS,
    ),
    LiteralDefault::int("debug.level", DEFAULT_DEBUG_LEVEL),
    LiteralDefault::bool("log.file.enabled", DEFAULT_LOG_FILE_ENABLED),
    LiteralDefault::string("log.file.level", DEFAULT_LOG_FILE_LEVEL),
    LiteralDefault::int("ui.width", DEFAULT_UI_WIDTH),
    LiteralDefault::int("ui.margin", DEFAULT_UI_MARGIN),
    LiteralDefault::int("ui.indent", DEFAULT_UI_INDENT),
    LiteralDefault::string("ui.presentation", DEFAULT_UI_PRESENTATION),
    LiteralDefault::string("ui.help.level", "inherit"),
    LiteralDefault::string("ui.guide.default_format", DEFAULT_UI_GUIDE_DEFAULT_FORMAT),
    LiteralDefault::string("ui.messages.layout", DEFAULT_UI_MESSAGES_LAYOUT),
    LiteralDefault::string("ui.message.verbosity", "success"),
    LiteralDefault::string("ui.chrome.frame", DEFAULT_UI_CHROME_FRAME),
    LiteralDefault::string("ui.chrome.rule_policy", DEFAULT_UI_CHROME_RULE_POLICY),
    LiteralDefault::string("ui.table.overflow", DEFAULT_UI_TABLE_OVERFLOW),
    LiteralDefault::string("ui.table.border", DEFAULT_UI_TABLE_BORDER),
    LiteralDefault::string("ui.help.table_chrome", "none"),
    LiteralDefault::string("ui.help.entry_indent", "inherit"),
    LiteralDefault::string("ui.help.entry_gap", "inherit"),
    LiteralDefault::string("ui.help.section_spacing", "inherit"),
    LiteralDefault::int("ui.short_list_max", DEFAULT_UI_SHORT_LIST_MAX),
    LiteralDefault::int("ui.medium_list_max", DEFAULT_UI_MEDIUM_LIST_MAX),
    LiteralDefault::int("ui.grid_padding", DEFAULT_UI_GRID_PADDING),
    LiteralDefault::int("ui.column_weight", DEFAULT_UI_COLUMN_WEIGHT),
    LiteralDefault::int(
        "ui.mreg.stack_min_col_width",
        DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH,
    ),
    LiteralDefault::int(
        "ui.mreg.stack_overflow_ratio",
        DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO,
    ),
    LiteralDefault::int(
        "extensions.plugins.timeout_ms",
        DEFAULT_EXTENSIONS_PLUGINS_TIMEOUT_MS,
    ),
    LiteralDefault::bool("extensions.plugins.discovery.path", false),
];

#[derive(Clone, Copy)]
enum LiteralDefaultValue {
    String(&'static str),
    Bool(bool),
    Integer(i64),
}

#[derive(Clone, Copy)]
struct LiteralDefault {
    key: &'static str,
    value: LiteralDefaultValue,
}

impl LiteralDefault {
    const fn string(key: &'static str, value: &'static str) -> Self {
        Self {
            key,
            value: LiteralDefaultValue::String(value),
        }
    }

    const fn bool(key: &'static str, value: bool) -> Self {
        Self {
            key,
            value: LiteralDefaultValue::Bool(value),
        }
    }

    const fn int(key: &'static str, value: i64) -> Self {
        Self {
            key,
            value: LiteralDefaultValue::Integer(value),
        }
    }

    fn seed(self, layer: &mut ConfigLayer) {
        match self.value {
            LiteralDefaultValue::String(value) => layer.set(self.key, value),
            LiteralDefaultValue::Bool(value) => layer.set(self.key, value),
            LiteralDefaultValue::Integer(value) => layer.set(self.key, value),
        }
    }
}

pub(super) fn build_builtin_defaults(
    env: &RuntimeEnvironment,
    default_theme_name: &str,
    default_repl_prompt: &str,
) -> ConfigLayer {
    let mut layer = ConfigLayer::default();
    seed_literal_defaults(&mut layer);
    seed_computed_defaults(&mut layer, env, default_theme_name, default_repl_prompt);
    layer
}

fn seed_literal_defaults(layer: &mut ConfigLayer) {
    for default in LITERAL_DEFAULTS {
        default.seed(layer);
    }

    for key in EMPTY_STYLE_OVERRIDE_KEYS {
        layer.set(*key, String::new());
    }
}

fn seed_computed_defaults(
    layer: &mut ConfigLayer,
    env: &RuntimeEnvironment,
    default_theme_name: &str,
    default_repl_prompt: &str,
) {
    layer.set("theme.name", default_theme_name);
    layer.set("user.name", env.user_name());
    layer.set("domain", env.domain_name());
    layer.set("repl.prompt", default_repl_prompt);
    layer.set("repl.history.path", env.repl_history_path());
    layer.set("log.file.path", env.log_file_path());

    let theme_path = env.theme_paths();
    if !theme_path.is_empty() {
        layer.set("theme.path", theme_path);
    }
}
