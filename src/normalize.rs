pub(crate) fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(crate) fn normalize_optional_identifier(value: Option<String>) -> Option<String> {
    value
        .map(|value| normalize_identifier(&value))
        .filter(|value| !value.is_empty())
}
