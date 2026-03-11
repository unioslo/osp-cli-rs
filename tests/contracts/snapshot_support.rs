use std::path::PathBuf;

#[macro_export]
macro_rules! assert_snapshot_text {
    ($name:expr, $text:expr $(,)?) => {{
        let sanitized = $crate::snapshot_support::sanitize_snapshot_text(
            ::std::convert::Into::<String>::into($text),
            &[],
        );
        let settings = $crate::snapshot_support::contract_snapshot_settings();
        settings.bind(|| insta::assert_snapshot!($name, sanitized));
    }};
}

#[macro_export]
macro_rules! assert_snapshot_text_with {
    ($name:expr, $text:expr, $replacements:expr $(,)?) => {{
        let sanitized = $crate::snapshot_support::sanitize_snapshot_text(
            ::std::convert::Into::<String>::into($text),
            $replacements,
        );
        let settings = $crate::snapshot_support::contract_snapshot_settings();
        settings.bind(|| insta::assert_snapshot!($name, sanitized));
    }};
}

#[macro_export]
macro_rules! assert_contract_snapshot {
    ($name:expr, $value:expr $(,)?) => {{
        let settings = $crate::snapshot_support::contract_snapshot_settings();
        settings.bind(|| insta::assert_snapshot!($name, $value));
    }};
}

pub(crate) fn contract_snapshot_settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(contract_snapshots_dir());
    settings
}

fn contract_snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("contracts")
        .join("snapshots")
}

pub(crate) fn sanitize_snapshot_text(mut text: String, replacements: &[(&str, &str)]) -> String {
    text = text.replace("\r\n", "\n");
    for (from, to) in replacements {
        text = text.replace(from, to);
    }
    text.lines()
        .map(sanitize_log_timestamp)
        .collect::<Vec<_>>()
        .join("\n")
}

fn sanitize_log_timestamp(line: &str) -> String {
    if let Some(idx) = line.find("Z ")
        && line.chars().next().is_some_and(|ch| ch.is_ascii_digit())
    {
        return format!("<TIMESTAMP>{}", &line[idx..]);
    }
    line.to_string()
}
