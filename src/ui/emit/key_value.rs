use unicode_width::UnicodeWidthStr;

use crate::ui::doc::{KeyValueRow, KeyValueValue};

pub(super) fn display_key(row: &KeyValueRow) -> String {
    match &row.value {
        KeyValueValue::Array(items) if should_include_count(items) => {
            format!("{} ({})", row.key, items.len())
        }
        _ => row.key.clone(),
    }
}

pub(super) fn aligned_display_key_width(rows: &[KeyValueRow]) -> usize {
    rows.iter()
        .map(|row| UnicodeWidthStr::width(display_key(row).as_str()))
        .max()
        .unwrap_or(0)
}

fn should_include_count(items: &[KeyValueValue]) -> bool {
    if items.is_empty() || items.len() > 1 {
        return true;
    }

    !matches!(items[0], KeyValueValue::Empty | KeyValueValue::Scalar(_))
}

#[cfg(test)]
mod tests {
    use super::{aligned_display_key_width, display_key};
    use crate::ui::doc::{KeyValueRow, KeyValueValue};

    #[test]
    fn display_key_adds_counts_for_multiline_collection_values_unit() {
        let scalar = KeyValueRow {
            key: "uid".to_string(),
            value: KeyValueValue::Scalar("alice".to_string()),
            indent: None,
            gap: None,
        };
        let scalar_array = KeyValueRow {
            key: "groups".to_string(),
            value: KeyValueValue::Array(vec![
                KeyValueValue::Scalar("dev".to_string()),
                KeyValueValue::Scalar("ops".to_string()),
            ]),
            indent: None,
            gap: None,
        };

        assert_eq!(display_key(&scalar), "uid");
        assert_eq!(display_key(&scalar_array), "groups (2)");
        assert_eq!(aligned_display_key_width(&[scalar, scalar_array]), 10);
    }
}
