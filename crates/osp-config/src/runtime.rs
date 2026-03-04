use std::path::PathBuf;

use crate::{
    ChainedLoader, ConfigLayer, EnvSecretsLoader, EnvVarLoader, LoaderPipeline, ResolvedConfig,
    SecretsTomlLoader, StaticLayerLoader, TomlFileLoader,
};

pub const DEFAULT_PROFILE_NAME: &str = "default";
pub const DEFAULT_REPL_HISTORY_MAX_ENTRIES: i64 = 1000;
pub const DEFAULT_SESSION_CACHE_MAX_RESULTS: i64 = 64;
pub const DEFAULT_DEBUG_LEVEL: i64 = 0;
pub const DEFAULT_LOG_FILE_ENABLED: bool = false;
pub const DEFAULT_LOG_FILE_LEVEL: &str = "warn";
pub const DEFAULT_UI_WIDTH: i64 = 72;
pub const DEFAULT_UI_MESSAGES_BOXED: bool = true;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub default_profile: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_profile: DEFAULT_PROFILE_NAME.to_string(),
        }
    }
}

impl RuntimeConfig {
    pub fn from_resolved(resolved: &ResolvedConfig) -> Self {
        let default_profile = resolved
            .get_string("profile.default")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| resolved.active_profile().to_string());
        Self { default_profile }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigPaths {
    pub config_file: Option<PathBuf>,
    pub secrets_file: Option<PathBuf>,
}

impl RuntimeConfigPaths {
    pub fn discover() -> Self {
        let config_file = std::env::var("OSP_CONFIG_FILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                default_config_root_dir().map(|mut path| {
                    path.push("config.toml");
                    path
                })
            });
        let secrets_file = std::env::var("OSP_SECRETS_FILE")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                default_config_root_dir().map(|mut path| {
                    path.push("secrets.toml");
                    path
                })
            });

        Self {
            config_file,
            secrets_file,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeDefaults {
    pub profile_default: String,
    pub theme_name: String,
    pub user_name: String,
    pub domain: String,
    pub repl_prompt: String,
    pub repl_simple_prompt: bool,
    pub repl_shell_indicator: String,
    pub repl_intro: bool,
    pub repl_history_path: String,
    pub repl_history_max_entries: i64,
    pub session_cache_max_results: i64,
    pub debug_level: i64,
    pub log_file_enabled: bool,
    pub log_file_path: String,
    pub log_file_level: String,
    pub ui_width: i64,
    pub ui_messages_boxed: bool,
    pub color_prompt_text: String,
    pub color_prompt_command: String,
}

impl RuntimeDefaults {
    pub fn from_process_env(default_theme_name: &str, default_repl_prompt: &str) -> Self {
        Self {
            profile_default: DEFAULT_PROFILE_NAME.to_string(),
            theme_name: default_theme_name.to_string(),
            user_name: default_user_name(),
            domain: default_domain_name(),
            repl_prompt: default_repl_prompt.to_string(),
            repl_simple_prompt: false,
            repl_shell_indicator: "[{shell}]".to_string(),
            repl_intro: true,
            repl_history_path: default_repl_history_path(),
            repl_history_max_entries: DEFAULT_REPL_HISTORY_MAX_ENTRIES,
            session_cache_max_results: DEFAULT_SESSION_CACHE_MAX_RESULTS,
            debug_level: DEFAULT_DEBUG_LEVEL,
            log_file_enabled: DEFAULT_LOG_FILE_ENABLED,
            log_file_path: default_log_file_path(),
            log_file_level: DEFAULT_LOG_FILE_LEVEL.to_string(),
            ui_width: DEFAULT_UI_WIDTH,
            ui_messages_boxed: DEFAULT_UI_MESSAGES_BOXED,
            color_prompt_text: String::new(),
            color_prompt_command: String::new(),
        }
    }

    pub fn to_layer(&self) -> ConfigLayer {
        let mut layer = ConfigLayer::default();
        layer.set("profile.default", self.profile_default.clone());
        layer.set("theme.name", self.theme_name.clone());
        layer.set("user.name", self.user_name.clone());
        layer.set("domain", self.domain.clone());
        layer.set("repl.prompt", self.repl_prompt.clone());
        layer.set("repl.simple_prompt", self.repl_simple_prompt);
        layer.set("repl.shell_indicator", self.repl_shell_indicator.clone());
        layer.set("repl.intro", self.repl_intro);
        layer.set("repl.history.path", self.repl_history_path.clone());
        layer.set("repl.history.max_entries", self.repl_history_max_entries);
        layer.set("session.cache.max_results", self.session_cache_max_results);
        layer.set("debug.level", self.debug_level);
        layer.set("log.file.enabled", self.log_file_enabled);
        layer.set("log.file.path", self.log_file_path.clone());
        layer.set("log.file.level", self.log_file_level.clone());
        layer.set("ui.width", self.ui_width);
        layer.set("ui.messages.boxed", self.ui_messages_boxed);
        layer.set("color.prompt.text", self.color_prompt_text.clone());
        layer.set("color.prompt.command", self.color_prompt_command.clone());
        layer
    }
}

pub fn build_runtime_pipeline(
    defaults: ConfigLayer,
    paths: &RuntimeConfigPaths,
    cli: Option<ConfigLayer>,
    session: Option<ConfigLayer>,
) -> LoaderPipeline {
    let mut pipeline = LoaderPipeline::new(StaticLayerLoader::new(defaults))
        .with_env(EnvVarLoader::from_process_env());

    if let Some(path) = &paths.config_file {
        pipeline = pipeline.with_file(TomlFileLoader::new(path.clone()).optional());
    }

    if let Some(path) = &paths.secrets_file {
        let secret_chain = ChainedLoader::new(SecretsTomlLoader::new(path.clone()).optional())
            .with(EnvSecretsLoader::from_process_env());
        pipeline = pipeline.with_secrets(secret_chain);
    } else {
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
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        let mut root = PathBuf::from(path);
        root.push("osp");
        return Some(root);
    }

    let home = std::env::var("HOME").ok()?;
    let mut root = PathBuf::from(home);
    root.push(".config");
    root.push("osp");
    Some(root)
}

fn default_user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "anonymous".to_string())
}

pub fn default_cache_root_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("XDG_CACHE_HOME") {
        let mut root = PathBuf::from(path);
        root.push("osp");
        return Some(root);
    }

    let home = std::env::var("HOME").ok()?;
    let mut root = PathBuf::from(home);
    root.push(".cache");
    root.push("osp");
    Some(root)
}

fn default_domain_name() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "localhost".to_string());
    host.split_once('.')
        .map(|(_, domain)| domain.to_string())
        .filter(|domain| !domain.trim().is_empty())
        .unwrap_or_else(|| "local".to_string())
}

fn default_repl_history_path() -> String {
    let mut path = default_state_root_dir().unwrap_or_else(|| {
        let mut fallback = std::env::temp_dir();
        fallback.push("osp");
        fallback
    });
    path.push("history.txt");
    path.display().to_string()
}

fn default_log_file_path() -> String {
    let mut path = default_state_root_dir().unwrap_or_else(|| {
        let mut fallback = std::env::temp_dir();
        fallback.push("osp");
        fallback
    });
    path.push("osp.log");
    path.display().to_string()
}

pub fn default_state_root_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("XDG_STATE_HOME") {
        let mut root = PathBuf::from(path);
        root.push("osp");
        return Some(root);
    }

    let home = std::env::var("HOME").ok()?;
    let mut root = PathBuf::from(home);
    root.push(".local");
    root.push("state");
    root.push("osp");
    Some(root)
}
