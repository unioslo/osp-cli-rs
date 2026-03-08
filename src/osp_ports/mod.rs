use crate::osp_core::row::Row;
use anyhow::{Result, anyhow};
use serde_json::Value;

pub trait LdapDirectory {
    fn user(
        &self,
        uid: &str,
        filter: Option<&str>,
        attributes: Option<&[String]>,
    ) -> Result<Vec<Row>>;

    fn netgroup(
        &self,
        name: &str,
        filter: Option<&str>,
        attributes: Option<&[String]>,
    ) -> Result<Vec<Row>>;
}

pub fn parse_attributes(raw: Option<&str>) -> Result<Option<Vec<String>>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let attrs = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<String>>();
    if attrs.is_empty() {
        return Err(anyhow!("--attributes must include at least one key"));
    }
    Ok(Some(attrs))
}

pub fn apply_filter_and_projection(
    rows: Vec<Row>,
    filter: Option<&str>,
    attributes: Option<&[String]>,
) -> Vec<Row> {
    let filtered = match filter {
        Some(spec) => rows
            .into_iter()
            .filter(|row| row_matches_filter(row, spec))
            .collect::<Vec<Row>>(),
        None => rows,
    };

    match attributes {
        Some(attrs) => filtered
            .into_iter()
            .map(|row| project_attributes(&row, attrs))
            .collect::<Vec<Row>>(),
        None => filtered,
    }
}

fn row_matches_filter(row: &Row, spec: &str) -> bool {
    let spec = spec.trim();
    if spec.is_empty() {
        return true;
    }

    if let Some((key, value)) = spec.split_once('=') {
        return field_equals(row, key.trim(), value.trim());
    }

    let query = spec.to_ascii_lowercase();
    let serial = Value::Object(row.clone()).to_string().to_ascii_lowercase();
    serial.contains(&query)
}

fn field_equals(row: &Row, key: &str, expected: &str) -> bool {
    let Some(value) = row.get(key) else {
        return false;
    };
    value_matches(value, expected)
}

fn value_matches(value: &Value, expected: &str) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| value_matches(item, expected)),
        Value::String(s) => string_matches(s, expected),
        other => string_matches(&other.to_string(), expected),
    }
}

fn string_matches(actual: &str, expected: &str) -> bool {
    if expected.contains('*') {
        return wildcard_match(expected, actual);
    }
    actual.eq_ignore_ascii_case(expected)
}

fn project_attributes(row: &Row, attrs: &[String]) -> Row {
    let mut selected = Row::new();
    for key in attrs {
        if let Some(value) = row.get(key) {
            selected.insert(key.clone(), value.clone());
        }
    }
    selected
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let escaped = regex::escape(pattern).replace("\\*", ".*");
    let re = regex::Regex::new(&format!("^{escaped}$"));
    match re {
        Ok(re) => re.is_match(value),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Row, apply_filter_and_projection, parse_attributes};

    fn user_row() -> Row {
        json!({
            "uid": "oistes",
            "cn": "Øistein Søvik",
            "netgroups": ["ucore", "usit"]
        })
        .as_object()
        .cloned()
        .expect("fixture must be object")
    }

    #[test]
    fn filter_supports_key_value_match() {
        let rows = vec![user_row()];
        let result = apply_filter_and_projection(rows, Some("uid=oistes"), None);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn projection_keeps_selected_keys_only() {
        let rows = vec![user_row()];
        let attrs = vec!["uid".to_string()];
        let result = apply_filter_and_projection(rows, None, Some(&attrs));
        assert_eq!(result[0].len(), 1);
        assert!(result[0].contains_key("uid"));
    }

    #[test]
    fn parse_attributes_trims_and_rejects_empty_lists() {
        let attrs = parse_attributes(Some(" uid , cn ,, mail "))
            .expect("attribute list should parse")
            .expect("attribute list should be present");
        assert_eq!(attrs, vec!["uid", "cn", "mail"]);

        assert!(
            parse_attributes(None)
                .expect("missing list is allowed")
                .is_none()
        );

        let err = parse_attributes(Some(" , ,, ")).expect_err("empty attribute list should fail");
        assert!(
            err.to_string()
                .contains("--attributes must include at least one key")
        );
    }

    #[test]
    fn filter_supports_case_insensitive_substring_and_wildcard_matching() {
        let rows = vec![user_row()];

        let substring = apply_filter_and_projection(rows.clone(), Some("søvik"), None);
        assert_eq!(substring.len(), 1);

        let wildcard = apply_filter_and_projection(rows, Some("uid=*tes"), None);
        assert_eq!(wildcard.len(), 1);
    }

    #[test]
    fn filter_matches_arrays_and_missing_fields_fail_cleanly() {
        let rows = vec![user_row()];

        let array_match = apply_filter_and_projection(rows.clone(), Some("netgroups=usit"), None);
        assert_eq!(array_match.len(), 1);

        let missing = apply_filter_and_projection(rows, Some("mail=oistes@example.org"), None);
        assert!(missing.is_empty());
    }

    #[test]
    fn projection_runs_after_filtering() {
        let rows = vec![user_row()];
        let attrs = vec!["uid".to_string()];
        let result = apply_filter_and_projection(rows, Some("uid=oistes"), Some(&attrs));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
        assert_eq!(
            result[0].get("uid").and_then(|value| value.as_str()),
            Some("oistes")
        );
    }
}
