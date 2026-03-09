use std::{collections::BTreeMap, path::PathBuf};

use crate::config::{
    ChainedLoader, ConfigLayer, EnvSecretsLoader, EnvVarLoader, LoaderPipeline, ResolvedConfig,
    SecretsTomlLoader, StaticLayerLoader, TomlFileLoader,
};

pub const DEFAULT_PROFILE_NAME: &str = "default";
pub const DEFAULT_REPL_HISTORY_MAX_ENTRIES: i64 = 1000;
pub const DEFAULT_REPL_HISTORY_ENABLED: bool = true;
pub const DEFAULT_REPL_HISTORY_DEDUPE: bool = true;
pub const DEFAULT_REPL_HISTORY_PROFILE_SCOPED: bool = true;
pub const DEFAULT_SESSION_CACHE_MAX_RESULTS: i64 = 64;
pub const DEFAULT_DEBUG_LEVEL: i64 = 0;
pub const DEFAULT_LOG_FILE_ENABLED: bool = false;
pub const DEFAULT_LOG_FILE_LEVEL: &str = "warn";
pub const DEFAULT_UI_WIDTH: i64 = 72;
pub const DEFAULT_UI_MARGIN: i64 = 0;
pub const DEFAULT_UI_INDENT: i64 = 2;
pub const DEFAULT_UI_PRESENTATION: &str = "expressive";
pub const DEFAULT_UI_HELP_LAYOUT: &str = "full";
pub const DEFAULT_UI_GUIDE_DEFAULT_FORMAT: &str = "guide";
pub const DEFAULT_UI_MESSAGES_LAYOUT: &str = "grouped";
pub const DEFAULT_UI_CHROME_FRAME: &str = "top";
pub const DEFAULT_UI_TABLE_BORDER: &str = "square";
pub const DEFAULT_REPL_INTRO: &str = "full";
pub const DEFAULT_UI_SHORT_LIST_MAX: i64 = 1;
pub const DEFAULT_UI_MEDIUM_LIST_MAX: i64 = 5;
pub const DEFAULT_UI_GRID_PADDING: i64 = 4;
pub const DEFAULT_UI_COLUMN_WEIGHT: i64 = 3;
pub const DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH: i64 = 10;
pub const DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO: i64 = 200;
pub const DEFAULT_UI_TABLE_OVERFLOW: &str = "clip";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeLoadOptions {
    pub include_env: bool,
    pub include_config_file: bool,
}

impl Default for RuntimeLoadOptions {
    fn default() -> Self {
        Self {
            include_env: true,
            include_config_file: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub active_profile: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            active_profile: DEFAULT_PROFILE_NAME.to_string(),
        }
    }
}

impl RuntimeConfig {
    pub fn from_resolved(resolved: &ResolvedConfig) -> Self {
        Self {
            active_profile: resolved.active_profile().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigPaths {
    pub config_file: Option<PathBuf>,
    pub secrets_file: Option<PathBuf>,
}

impl RuntimeConfigPaths {
    pub fn discover() -> Self {
        let paths = Self::from_env(&RuntimeEnvironment::capture());
        tracing::debug!(
            config_file = ?paths.config_file.as_ref().map(|path| path.display().to_string()),
            secrets_file = ?paths.secrets_file.as_ref().map(|path| path.display().to_string()),
            "discovered runtime config paths"
        );
        paths
    }

    fn from_env(env: &RuntimeEnvironment) -> Self {
        Self {
            config_file: env
                .path_override("OSP_CONFIG_FILE")
                .or_else(|| env.config_path("config.toml")),
            secrets_file: env
                .path_override("OSP_SECRETS_FILE")
                .or_else(|| env.config_path("secrets.toml")),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeDefaults {
    layer: ConfigLayer,
}

impl RuntimeDefaults {
    pub fn from_process_env(default_theme_name: &str, default_repl_prompt: &str) -> Self {
        Self::from_env(
            &RuntimeEnvironment::capture(),
            default_theme_name,
            default_repl_prompt,
        )
    }

    fn from_env(
        env: &RuntimeEnvironment,
        default_theme_name: &str,
        default_repl_prompt: &str,
    ) -> Self {
        let mut layer = ConfigLayer::default();

        macro_rules! set_defaults {
            ($($key:literal => $value:expr),* $(,)?) => {
                $(layer.set($key, $value);)*
            };
        }

        set_defaults! {
            "profile.default" => DEFAULT_PROFILE_NAME.to_string(),
            "theme.name" => default_theme_name.to_string(),
            "user.name" => env.user_name(),
            "domain" => env.domain_name(),
            "repl.prompt" => default_repl_prompt.to_string(),
            "repl.input_mode" => "auto".to_string(),
            "repl.simple_prompt" => false,
            "repl.shell_indicator" => "[{shell}]".to_string(),
            "repl.intro" => DEFAULT_REPL_INTRO.to_string(),
            "repl.history.path" => env.repl_history_path(),
            "repl.history.max_entries" => DEFAULT_REPL_HISTORY_MAX_ENTRIES,
            "repl.history.enabled" => DEFAULT_REPL_HISTORY_ENABLED,
            "repl.history.dedupe" => DEFAULT_REPL_HISTORY_DEDUPE,
            "repl.history.profile_scoped" => DEFAULT_REPL_HISTORY_PROFILE_SCOPED,
            "session.cache.max_results" => DEFAULT_SESSION_CACHE_MAX_RESULTS,
            "debug.level" => DEFAULT_DEBUG_LEVEL,
            "log.file.enabled" => DEFAULT_LOG_FILE_ENABLED,
            "log.file.path" => env.log_file_path(),
            "log.file.level" => DEFAULT_LOG_FILE_LEVEL.to_string(),
            "ui.width" => DEFAULT_UI_WIDTH,
            "ui.margin" => DEFAULT_UI_MARGIN,
            "ui.indent" => DEFAULT_UI_INDENT,
            "ui.presentation" => DEFAULT_UI_PRESENTATION.to_string(),
            "ui.help.layout" => DEFAULT_UI_HELP_LAYOUT.to_string(),
            "ui.guide.default_format" => DEFAULT_UI_GUIDE_DEFAULT_FORMAT.to_string(),
            "ui.messages.layout" => DEFAULT_UI_MESSAGES_LAYOUT.to_string(),
            "ui.chrome.frame" => DEFAULT_UI_CHROME_FRAME.to_string(),
            "ui.table.overflow" => DEFAULT_UI_TABLE_OVERFLOW.to_string(),
            "ui.table.border" => DEFAULT_UI_TABLE_BORDER.to_string(),
            "ui.short_list_max" => DEFAULT_UI_SHORT_LIST_MAX,
            "ui.medium_list_max" => DEFAULT_UI_MEDIUM_LIST_MAX,
            "ui.grid_padding" => DEFAULT_UI_GRID_PADDING,
            "ui.column_weight" => DEFAULT_UI_COLUMN_WEIGHT,
            "ui.mreg.stack_min_col_width" => DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH,
            "ui.mreg.stack_overflow_ratio" => DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO,
            "extensions.plugins.timeout_ms" => 10_000,
            "extensions.plugins.discovery.path" => false,
        }

        let theme_path = env.theme_paths();
        if !theme_path.is_empty() {
            layer.set("theme.path", theme_path);
        }

        for key in [
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
        ] {
            layer.set(key, String::new());
        }

        Self { layer }
    }

    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.layer
            .entries()
            .iter()
            .find(|entry| entry.key == key && entry.scope == crate::config::Scope::global())
            .and_then(|entry| match entry.value.reveal() {
                crate::config::ConfigValue::String(value) => Some(value.as_str()),
                _ => None,
            })
    }

    pub fn to_layer(&self) -> ConfigLayer {
        self.layer.clone()
    }
}

pub fn build_runtime_pipeline(
    defaults: ConfigLayer,
    presentation: Option<ConfigLayer>,
    paths: &RuntimeConfigPaths,
    load: RuntimeLoadOptions,
    cli: Option<ConfigLayer>,
    session: Option<ConfigLayer>,
) -> LoaderPipeline {
    tracing::debug!(
        include_env = load.include_env,
        include_config_file = load.include_config_file,
        config_file = ?paths.config_file.as_ref().map(|path| path.display().to_string()),
        secrets_file = ?paths.secrets_file.as_ref().map(|path| path.display().to_string()),
        has_presentation_layer = presentation.is_some(),
        has_cli_layer = cli.is_some(),
        has_session_layer = session.is_some(),
        defaults_entries = defaults.entries().len(),
        "building runtime loader pipeline"
    );
    let mut pipeline = LoaderPipeline::new(StaticLayerLoader::new(defaults));

    if let Some(presentation_layer) = presentation {
        pipeline = pipeline.with_presentation(StaticLayerLoader::new(presentation_layer));
    }

    if load.include_env {
        pipeline = pipeline.with_env(EnvVarLoader::from_process_env());
    }

    if load.include_config_file
        && let Some(path) = &paths.config_file
    {
        pipeline = pipeline.with_file(TomlFileLoader::new(path.clone()).optional());
    }

    if let Some(path) = &paths.secrets_file {
        let mut secret_chain = ChainedLoader::new(SecretsTomlLoader::new(path.clone()).optional());
        if load.include_env {
            secret_chain = secret_chain.with(EnvSecretsLoader::from_process_env());
        }
        pipeline = pipeline.with_secrets(secret_chain);
    } else if load.include_env {
        pipeline = pipeline.with_secrets(ChainedLoader::new(EnvSecretsLoader::from_process_env()));
    }

    if let Some(cli_layer) = cli {
        pipeline = pipeline.with_cli(StaticLayerLoader::new(cli_layer));
    }
    if let Some(session_layer) = session {
        pipeline = pipeline.with_session(StaticLayerLoader::new(session_layer));
    }

    pipeline
}

pub fn default_config_root_dir() -> Option<PathBuf> {
    RuntimeEnvironment::capture().config_root_dir()
}

pub fn default_cache_root_dir() -> Option<PathBuf> {
    RuntimeEnvironment::capture().cache_root_dir()
}

pub fn default_state_root_dir() -> Option<PathBuf> {
    RuntimeEnvironment::capture().state_root_dir()
}

#[derive(Debug, Clone, Default)]
struct RuntimeEnvironment {
    vars: BTreeMap<String, String>,
}

impl RuntimeEnvironment {
    fn capture() -> Self {
        Self::from_pairs(std::env::vars())
    }

    fn from_pairs<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self {
            vars: vars
                .into_iter()
                .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().to_string()))
                .collect(),
        }
    }

    fn config_root_dir(&self) -> Option<PathBuf> {
        self.xdg_root_dir("XDG_CONFIG_HOME", &[".config"])
    }

    fn cache_root_dir(&self) -> Option<PathBuf> {
        self.xdg_root_dir("XDG_CACHE_HOME", &[".cache"])
    }

    fn state_root_dir(&self) -> Option<PathBuf> {
        self.xdg_root_dir("XDG_STATE_HOME", &[".local", "state"])
    }

    fn config_path(&self, leaf: &str) -> Option<PathBuf> {
        self.config_root_dir().map(|root| join_path(root, &[leaf]))
    }

    fn theme_paths(&self) -> Vec<String> {
        self.config_root_dir()
            .map(|root| join_path(root, &["themes"]).to_string_lossy().to_string())
            .into_iter()
            .collect()
    }

    fn user_name(&self) -> String {
        self.get_nonempty("USER")
            .or_else(|| self.get_nonempty("USERNAME"))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "anonymous".to_string())
    }

    fn domain_name(&self) -> String {
        self.get_nonempty("HOSTNAME")
            .or_else(|| self.get_nonempty("COMPUTERNAME"))
            .unwrap_or("localhost")
            .split_once('.')
            .map(|(_, domain)| domain.to_string())
            .filter(|domain| !domain.trim().is_empty())
            .unwrap_or_else(|| "local".to_string())
    }

    fn repl_history_path(&self) -> String {
        join_path(
            self.state_root_dir_or_temp(),
            &["history", "${user.name}@${profile.active}.history"],
        )
        .display()
        .to_string()
    }

    fn log_file_path(&self) -> String {
        join_path(self.state_root_dir_or_temp(), &["osp.log"])
            .display()
            .to_string()
    }

    fn path_override(&self, key: &str) -> Option<PathBuf> {
        self.get_nonempty(key).map(PathBuf::from)
    }

    fn state_root_dir_or_temp(&self) -> PathBuf {
        self.state_root_dir().unwrap_or_else(|| {
            let mut path = std::env::temp_dir();
            path.push("osp");
            path
        })
    }

    fn xdg_root_dir(&self, xdg_var: &str, home_suffix: &[&str]) -> Option<PathBuf> {
        if let Some(path) = self.get_nonempty(xdg_var) {
            return Some(join_path(PathBuf::from(path), &["osp"]));
        }

        let home = self.get_nonempty("HOME")?;
        Some(join_path(PathBuf::from(home), home_suffix).join("osp"))
    }

    fn get_nonempty(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

fn join_path(mut root: PathBuf, segments: &[&str]) -> PathBuf {
    for segment in segments {
        root.push(segment);
    }
    root
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{DEFAULT_PROFILE_NAME, RuntimeConfigPaths, RuntimeDefaults, RuntimeEnvironment};
    use crate::config::{ConfigLayer, ConfigValue, Scope};

    fn find_value<'a>(layer: &'a ConfigLayer, key: &str) -> Option<&'a ConfigValue> {
        layer
            .entries()
            .iter()
            .find(|entry| entry.key == key && entry.scope == Scope::global())
            .map(|entry| &entry.value)
    }

    #[test]
    fn runtime_defaults_seed_expected_keys() {
        let defaults =
            RuntimeDefaults::from_env(&RuntimeEnvironment::default(), "nord", "osp> ").to_layer();

        assert_eq!(
            find_value(&defaults, "profile.default"),
            Some(&ConfigValue::String(DEFAULT_PROFILE_NAME.to_string()))
        );
        assert_eq!(
            find_value(&defaults, "theme.name"),
            Some(&ConfigValue::String("nord".to_string()))
        );
        assert_eq!(
            find_value(&defaults, "repl.prompt"),
            Some(&ConfigValue::String("osp> ".to_string()))
        );
        assert_eq!(
            find_value(&defaults, "repl.intro"),
            Some(&ConfigValue::String(super::DEFAULT_REPL_INTRO.to_string()))
        );
        assert_eq!(
            find_value(&defaults, "repl.history.max_entries"),
            Some(&ConfigValue::Integer(
                super::DEFAULT_REPL_HISTORY_MAX_ENTRIES
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.width"),
            Some(&ConfigValue::Integer(super::DEFAULT_UI_WIDTH))
        );
        assert_eq!(
            find_value(&defaults, "ui.presentation"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_PRESENTATION.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.help.layout"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_HELP_LAYOUT.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.messages.layout"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_MESSAGES_LAYOUT.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.chrome.frame"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_CHROME_FRAME.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.table.border"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_TABLE_BORDER.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "color.prompt.text"),
            Some(&ConfigValue::String(String::new()))
        );
    }

    #[test]
    fn runtime_defaults_history_path_keeps_placeholders() {
        let defaults =
            RuntimeDefaults::from_env(&RuntimeEnvironment::default(), "nord", "osp> ").to_layer();
        let path = match find_value(&defaults, "repl.history.path") {
            Some(ConfigValue::String(value)) => value.as_str(),
            other => panic!("unexpected history path value: {other:?}"),
        };

        assert!(path.contains("${user.name}@${profile.active}.history"));
    }

    #[test]
    fn runtime_config_paths_prefer_explicit_file_overrides() {
        let env = RuntimeEnvironment::from_pairs([
            ("OSP_CONFIG_FILE", "/tmp/custom-config.toml"),
            ("OSP_SECRETS_FILE", "/tmp/custom-secrets.toml"),
            ("XDG_CONFIG_HOME", "/ignored"),
        ]);

        let paths = RuntimeConfigPaths::from_env(&env);

        assert_eq!(
            paths.config_file,
            Some(PathBuf::from("/tmp/custom-config.toml"))
        );
        assert_eq!(
            paths.secrets_file,
            Some(PathBuf::from("/tmp/custom-secrets.toml"))
        );
    }

    #[test]
    fn runtime_config_paths_fall_back_to_xdg_root() {
        let env = RuntimeEnvironment::from_pairs([("XDG_CONFIG_HOME", "/var/tmp/xdg-config")]);

        let paths = RuntimeConfigPaths::from_env(&env);

        assert_eq!(
            paths.config_file,
            Some(PathBuf::from("/var/tmp/xdg-config/osp/config.toml"))
        );
        assert_eq!(
            paths.secrets_file,
            Some(PathBuf::from("/var/tmp/xdg-config/osp/secrets.toml"))
        );
    }

    #[test]
    fn runtime_environment_uses_home_when_xdg_is_missing() {
        let env = RuntimeEnvironment::from_pairs([("HOME", "/home/tester")]);

        assert_eq!(
            env.config_root_dir(),
            Some(PathBuf::from("/home/tester/.config/osp"))
        );
        assert_eq!(
            env.cache_root_dir(),
            Some(PathBuf::from("/home/tester/.cache/osp"))
        );
        assert_eq!(
            env.state_root_dir(),
            Some(PathBuf::from("/home/tester/.local/state/osp"))
        );
    }

    #[test]
    fn runtime_environment_state_artifacts_fall_back_to_temp_root() {
        let env = RuntimeEnvironment::default();
        let mut expected_root = std::env::temp_dir();
        expected_root.push("osp");

        assert_eq!(
            env.repl_history_path(),
            expected_root
                .join("history")
                .join("${user.name}@${profile.active}.history")
                .display()
                .to_string()
        );
        assert_eq!(
            env.log_file_path(),
            expected_root.join("osp.log").display().to_string()
        );
    }
}
