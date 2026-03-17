use unicode_width::UnicodeWidthStr;

use crate::ui::doc::{KeyValueBlock, KeyValueRow, KeyValueStyle};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PreparedKeyValueBlock {
    Plain(Vec<PreparedPlainRow>),
    Bulleted(Vec<PreparedBulletedRow>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedPlainRow {
    pub key: String,
    pub value: String,
    pub indent: String,
    pub value_spacing: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedBulletedRow {
    pub key: String,
    pub value: String,
}

impl PreparedKeyValueBlock {
    pub(super) fn from_block(block: &KeyValueBlock) -> Self {
        match block.style {
            KeyValueStyle::Plain => Self::Plain(prepare_plain_rows(&block.rows)),
            KeyValueStyle::Bulleted => Self::Bulleted(prepare_bulleted_rows(&block.rows)),
        }
    }
}

fn prepare_plain_rows(rows: &[KeyValueRow]) -> Vec<PreparedPlainRow> {
    let key_width = aligned_key_width(rows);
    rows.iter()
        .map(|row| PreparedPlainRow::new(row, key_width))
        .collect()
}

fn prepare_bulleted_rows(rows: &[KeyValueRow]) -> Vec<PreparedBulletedRow> {
    rows.iter().map(PreparedBulletedRow::new).collect()
}

fn aligned_key_width(rows: &[KeyValueRow]) -> usize {
    rows.iter()
        .map(|row| UnicodeWidthStr::width(row.key.as_str()))
        .max()
        .unwrap_or(0)
}

impl PreparedPlainRow {
    fn new(row: &KeyValueRow, key_width: usize) -> Self {
        let padding = key_width.saturating_sub(UnicodeWidthStr::width(row.key.as_str()));
        Self {
            key: row.key.clone(),
            value: row.value.clone(),
            indent: row.indent.clone().unwrap_or_default(),
            value_spacing: " ".repeat(padding.saturating_add(1)),
        }
    }
}

impl PreparedBulletedRow {
    fn new(row: &KeyValueRow) -> Self {
        Self {
            key: row.key.clone(),
            value: row.value.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PreparedKeyValueBlock, PreparedPlainRow};
    use crate::ui::doc::{KeyValueBlock, KeyValueRow, KeyValueStyle};

    #[test]
    fn prepared_plain_rows_align_to_one_width_owner_unit() {
        let block = KeyValueBlock {
            style: KeyValueStyle::Plain,
            rows: vec![
                KeyValueRow {
                    key: "uid".to_string(),
                    value: "alice".to_string(),
                    indent: None,
                    gap: None,
                },
                KeyValueRow {
                    key: "display_name".to_string(),
                    value: "Alice Example".to_string(),
                    indent: Some(">".to_string()),
                    gap: None,
                },
            ],
        };

        let PreparedKeyValueBlock::Plain(rows) = PreparedKeyValueBlock::from_block(&block) else {
            panic!("expected prepared plain rows");
        };

        assert_eq!(
            rows[0],
            PreparedPlainRow {
                key: "uid".to_string(),
                value: "alice".to_string(),
                indent: String::new(),
                value_spacing: " ".repeat(10),
            }
        );
        assert_eq!(
            rows[1],
            PreparedPlainRow {
                key: "display_name".to_string(),
                value: "Alice Example".to_string(),
                indent: ">".to_string(),
                value_spacing: " ".repeat(1),
            }
        );
    }
}
