use std::fs::OpenOptions;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

static INIT_LOGGING: Once = Once::new();

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

pub fn init_developer_logging(config: DeveloperLoggingConfig) {
    INIT_LOGGING.call_once(|| {
        let stderr_level = map_debug_count(config.debug_count);
        let use_ansi = std::io::stderr().is_terminal();

        let stderr_layer = fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_ansi(use_ansi)
            .compact()
            .with_filter(stderr_level);

        if let Some(file) = config.file {
            match file_writer(&file.path) {
                Ok(writer) => {
                    let file_layer = fmt::layer()
                        .with_writer(move || writer.clone())
                        .with_target(true)
                        .with_ansi(false)
                        .compact()
                        .with_filter(file.level);

                    if let Err(err) = tracing_subscriber::registry()
                        .with(stderr_layer)
                        .with(file_layer)
                        .try_init()
                        && config.debug_count >= 2
                    {
                        eprintln!("logging already initialized: {err}");
                    }
                }
                Err(err) => {
                    if let Err(err) = tracing_subscriber::registry().with(stderr_layer).try_init()
                        && config.debug_count >= 2
                    {
                        eprintln!("logging already initialized: {err}");
                    }
                    eprintln!("failed to initialize file logging: {err}");
                }
            }
        } else if let Err(err) = tracing_subscriber::registry().with(stderr_layer).try_init()
            && config.debug_count >= 2
        {
            eprintln!("logging already initialized: {err}");
        }
    });
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

#[derive(Clone)]
struct SharedFileWriter {
    file: Arc<Mutex<std::fs::File>>,
}

impl Write for SharedFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("log file mutex poisoned"))?;
        file.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("log file mutex poisoned"))?;
        file.flush()
    }
}

fn file_writer(path: &Path) -> Result<SharedFileWriter, String> {
    let (directory, _) = split_file_path(path)?;
    std::fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| err.to_string())?;
    Ok(SharedFileWriter {
        file: Arc::new(Mutex::new(file)),
    })
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

#[cfg(test)]
mod tests {
    use super::{map_debug_count, parse_level_filter, split_file_path};
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
}
