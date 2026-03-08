#[macro_export]
macro_rules! assert_snapshot_text {
    ($name:expr, $text:expr $(,)?) => {{
        let sanitized = $crate::snapshot_support::sanitize_snapshot_text(
            ::std::convert::Into::<String>::into($text),
            &[],
        );
        insta::assert_snapshot!($name, sanitized);
    }};
}

#[macro_export]
macro_rules! assert_snapshot_text_with {
    ($name:expr, $text:expr, $replacements:expr $(,)?) => {{
        let sanitized = $crate::snapshot_support::sanitize_snapshot_text(
            ::std::convert::Into::<String>::into($text),
            $replacements,
        );
        insta::assert_snapshot!($name, sanitized);
    }};
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
