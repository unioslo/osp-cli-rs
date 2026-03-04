use std::io::IsTerminal;
use std::sync::Once;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

static INIT_LOGGING: Once = Once::new();

pub fn init_developer_logging(debug_count: u8) {
    INIT_LOGGING.call_once(|| {
        let default_level = map_debug_count(debug_count);
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(default_level.as_str()));

        let use_ansi = std::io::stderr().is_terminal();
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_target(true)
                    .with_ansi(use_ansi)
                    .compact(),
            )
            .try_init();
    });
}

fn map_debug_count(debug_count: u8) -> LevelFilter {
    match debug_count {
        0 => LevelFilter::WARN,
        1 => LevelFilter::INFO,
        2 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

trait AsStr {
    fn as_str(self) -> &'static str;
}

impl AsStr for LevelFilter {
    fn as_str(self) -> &'static str {
        match self {
            LevelFilter::OFF => "off",
            LevelFilter::ERROR => "error",
            LevelFilter::WARN => "warn",
            LevelFilter::INFO => "info",
            LevelFilter::DEBUG => "debug",
            LevelFilter::TRACE => "trace",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::map_debug_count;
    use tracing_subscriber::filter::LevelFilter;

    #[test]
    fn debug_count_maps_to_expected_levels() {
        assert_eq!(map_debug_count(0), LevelFilter::WARN);
        assert_eq!(map_debug_count(1), LevelFilter::INFO);
        assert_eq!(map_debug_count(2), LevelFilter::DEBUG);
        assert_eq!(map_debug_count(3), LevelFilter::TRACE);
        assert_eq!(map_debug_count(9), LevelFilter::TRACE);
    }
}
