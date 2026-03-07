use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;

static LOGGING_STATE: OnceLock<Option<LoggingState>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct FileLoggingConfig {
    pub path: PathBuf,
    pub level: LevelFilter,
}

#[derive(Debug, Clone)]
pub struct DeveloperLoggingConfig {
    pub debug_count: u8,
    pub file: Option<FileLoggingConfig>,
}

pub fn bootstrap_logging_config(args: &[OsString]) -> DeveloperLoggingConfig {
    DeveloperLoggingConfig {
        debug_count: scan_debug_count(args),
        file: None,
    }
}

pub fn init_developer_logging(config: DeveloperLoggingConfig) {
    if let Some(state) = LOGGING_STATE
        .get_or_init(|| LoggingState::initialize(&config))
        .as_ref()
    {
        state.apply(&config);
    }
}

pub fn parse_level_filter(value: &str) -> Option<LevelFilter> {
    match value.trim().to_ascii_lowercase().as_str() {
        "error" => Some(LevelFilter::ERROR),
        "warn" | "warning" => Some(LevelFilter::WARN),
        "info" => Some(LevelFilter::INFO),
        "debug" => Some(LevelFilter::DEBUG),
        "trace" => Some(LevelFilter::TRACE),
        _ => None,
    }
}

struct LoggingState {
    stderr_handle: Box<dyn ReloadFilterHandle>,
    file_handle: Box<dyn ReloadFilterHandle>,
    file_writer: DynamicFileWriter,
}

trait ReloadFilterHandle: Send + Sync {
    fn reload(&self, level: LevelFilter) -> Result<(), String>;
}

impl<S> ReloadFilterHandle for reload::Handle<LevelFilter, S>
where
    S: tracing::Subscriber + Send + Sync + 'static,
{
    fn reload(&self, level: LevelFilter) -> Result<(), String> {
        reload::Handle::reload(self, level).map_err(|err| err.to_string())
    }
}

impl LoggingState {
    fn initialize(config: &DeveloperLoggingConfig) -> Option<Self> {
        let file_writer = DynamicFileWriter::default();
        let file_level = match file_writer.configure(config.file.as_ref()) {
            Ok(level) => level,
            Err(err) => {
                eprintln!("failed to initialize file logging: {err}");
                LevelFilter::OFF
            }
        };

        let use_ansi = std::io::stderr().is_terminal();
        let (stderr_filter, stderr_handle) =
            reload::Layer::new(map_debug_count(config.debug_count));
        let stderr_layer = fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_ansi(use_ansi)
            .compact()
            .with_filter(stderr_filter);
        let (file_filter, file_handle) = reload::Layer::new(file_level);
        let file_writer_for_layer = file_writer.clone();
        let file_layer = fmt::layer()
            .with_writer(move || file_writer_for_layer.clone())
            .with_target(true)
            .with_ansi(false)
            .compact()
            .with_filter(file_filter);

        if let Err(err) = registry().with(stderr_layer).with(file_layer).try_init() {
            if config.debug_count >= 2 {
                eprintln!("logging already initialized: {err}");
            }
            return None;
        }

        Some(Self {
            stderr_handle: Box::new(stderr_handle),
            file_handle: Box::new(file_handle),
            file_writer,
        })
    }

    fn apply(&self, config: &DeveloperLoggingConfig) {
        if let Err(err) = self
            .stderr_handle
            .reload(map_debug_count(config.debug_count))
            && config.debug_count >= 2
        {
            eprintln!("failed to reload stderr logging: {err}");
        }

        let file_level = match self.file_writer.configure(config.file.as_ref()) {
            Ok(level) => level,
            Err(err) => {
                self.file_writer.clear();
                eprintln!("failed to initialize file logging: {err}");
                LevelFilter::OFF
            }
        };

        if let Err(err) = self.file_handle.reload(file_level)
            && config.debug_count >= 2
        {
            eprintln!("failed to reload file logging: {err}");
        }
    }
}

#[derive(Clone, Default)]
struct DynamicFileWriter {
    state: Arc<Mutex<DynamicFileState>>,
}

#[derive(Default)]
struct DynamicFileState {
    file: Option<std::fs::File>,
}

impl DynamicFileWriter {
    fn configure(&self, file: Option<&FileLoggingConfig>) -> Result<LevelFilter, String> {
        let opened = match file {
            Some(file) => Some(open_log_file(&file.path)?),
            None => None,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| "log file mutex poisoned".to_string())?;
        state.file = opened;
        Ok(file.map(|value| value.level).unwrap_or(LevelFilter::OFF))
    }

    fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.file = None;
        }
    }
}

impl Write for DynamicFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("log file mutex poisoned"))?;
        if let Some(file) = state.file.as_mut() {
            file.write(buf)
        } else {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| std::io::Error::other("log file mutex poisoned"))?;
        if let Some(file) = state.file.as_mut() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

fn open_log_file(path: &Path) -> Result<std::fs::File, String> {
    let (directory, _) = split_file_path(path)?;
    std::fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| err.to_string())
}

fn split_file_path(path: &Path) -> Result<(PathBuf, String), String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("invalid log file path: {}", path.display()))?;

    let directory = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    Ok((directory, file_name))
}

fn map_debug_count(debug_count: u8) -> LevelFilter {
    match debug_count {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        2 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

fn scan_debug_count(args: &[OsString]) -> u8 {
    let mut debug = 0u8;

    for token in args.iter().skip(1) {
        let Some(value) = token.to_str() else {
            continue;
        };

        match value {
            "--" => break,
            "--debug" => {
                debug = debug.saturating_add(1);
                continue;
            }
            _ => {}
        }

        if value.starts_with('-') && !value.starts_with("--") {
            for ch in value.chars().skip(1) {
                if ch == 'd' {
                    debug = debug.saturating_add(1);
                }
            }
        }
    }

    debug
}

#[cfg(test)]
mod tests {
    use super::{bootstrap_logging_config, map_debug_count, parse_level_filter, split_file_path};
    use std::ffi::OsString;
    use std::path::Path;
    use tracing_subscriber::filter::LevelFilter;

    #[test]
    fn debug_count_maps_to_expected_levels() {
        assert_eq!(map_debug_count(0), LevelFilter::WARN);
        assert_eq!(map_debug_count(1), LevelFilter::INFO);
        assert_eq!(map_debug_count(2), LevelFilter::DEBUG);
        assert_eq!(map_debug_count(3), LevelFilter::TRACE);
        assert_eq!(map_debug_count(9), LevelFilter::TRACE);
    }

    #[test]
    fn parse_level_filter_recognizes_values() {
        assert_eq!(parse_level_filter("warn"), Some(LevelFilter::WARN));
        assert_eq!(parse_level_filter("WARNING"), Some(LevelFilter::WARN));
        assert_eq!(parse_level_filter("info"), Some(LevelFilter::INFO));
        assert_eq!(parse_level_filter("bad"), None);
    }

    #[test]
    fn split_file_path_requires_file_name() {
        assert!(split_file_path(Path::new("")).is_err());
    }

    #[test]
    fn bootstrap_logging_config_counts_debug_flags() {
        let config = bootstrap_logging_config(&[
            OsString::from("osp"),
            OsString::from("-dd"),
            OsString::from("plugins"),
            OsString::from("--debug"),
            OsString::from("list"),
        ]);

        assert_eq!(config.debug_count, 3);
        assert!(config.file.is_none());
    }
}
