use unicode_width::UnicodeWidthStr;

use crate::ui::doc::{GuideEntriesBlock, GuideEntryRow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedGuideEntriesBlock {
    pub rows: Vec<PreparedGuideEntryRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedGuideEntryRow {
    pub key: String,
    pub value: String,
    pub indent: String,
    pub gap: String,
}

impl PreparedGuideEntriesBlock {
    pub(super) fn from_block(block: &GuideEntriesBlock) -> Self {
        let key_width = block
            .rows
            .iter()
            .map(|row| UnicodeWidthStr::width(row.key.as_str()))
            .max()
            .unwrap_or(0);

        Self {
            rows: block
                .rows
                .iter()
                .map(|row| {
                    PreparedGuideEntryRow::new(
                        row,
                        key_width,
                        &block.default_indent,
                        block.default_gap.as_deref(),
                    )
                })
                .collect(),
        }
    }
}

impl PreparedGuideEntryRow {
    fn new(
        row: &GuideEntryRow,
        key_width: usize,
        default_indent: &str,
        default_gap: Option<&str>,
    ) -> Self {
        let padding =
            " ".repeat(key_width.saturating_sub(UnicodeWidthStr::width(row.key.as_str())));
        Self {
            key: row.key.clone(),
            value: row.value.clone(),
            indent: row
                .indent_hint
                .clone()
                .unwrap_or_else(|| default_indent.to_string()),
            gap: row
                .gap_hint
                .clone()
                .or_else(|| default_gap.map(str::to_owned))
                .unwrap_or_else(|| format!("{padding}  ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PreparedGuideEntriesBlock, PreparedGuideEntryRow};
    use crate::ui::doc::{GuideEntriesBlock, GuideEntryRow};

    #[test]
    fn prepared_direct_guide_entries_ignore_help_indent_defaults_unit() {
        let block = GuideEntriesBlock {
            default_indent: String::new(),
            default_gap: None,
            rows: vec![
                GuideEntryRow {
                    key: "list".to_string(),
                    value: "List history entries".to_string(),
                    indent_hint: Some("  ".to_string()),
                    gap_hint: Some("   ".to_string()),
                },
                GuideEntryRow {
                    key: "clear".to_string(),
                    value: "Clear history entries".to_string(),
                    indent_hint: None,
                    gap_hint: None,
                },
            ],
        };

        let prepared = PreparedGuideEntriesBlock::from_block(&block);

        assert_eq!(
            prepared.rows[0],
            PreparedGuideEntryRow {
                key: "list".to_string(),
                value: "List history entries".to_string(),
                indent: "  ".to_string(),
                gap: "   ".to_string(),
            }
        );
        assert_eq!(
            prepared.rows[1],
            PreparedGuideEntryRow {
                key: "clear".to_string(),
                value: "Clear history entries".to_string(),
                indent: String::new(),
                gap: "  ".to_string(),
            }
        );
    }

    #[test]
    fn prepared_help_guide_entries_default_to_help_indent_unit() {
        let block = GuideEntriesBlock {
            default_indent: "  ".to_string(),
            default_gap: None,
            rows: vec![GuideEntryRow {
                key: "list".to_string(),
                value: "List history entries".to_string(),
                indent_hint: None,
                gap_hint: None,
            }],
        };

        let prepared = PreparedGuideEntriesBlock::from_block(&block);

        assert_eq!(
            prepared.rows[0],
            PreparedGuideEntryRow {
                key: "list".to_string(),
                value: "List history entries".to_string(),
                indent: "  ".to_string(),
                gap: "  ".to_string(),
            }
        );
    }

    #[test]
    fn prepared_guide_entries_can_override_default_gap_unit() {
        let block = GuideEntriesBlock {
            default_indent: "  ".to_string(),
            default_gap: Some(" -> ".to_string()),
            rows: vec![GuideEntryRow {
                key: "list".to_string(),
                value: "List history entries".to_string(),
                indent_hint: None,
                gap_hint: None,
            }],
        };

        let prepared = PreparedGuideEntriesBlock::from_block(&block);

        assert_eq!(
            prepared.rows[0],
            PreparedGuideEntryRow {
                key: "list".to_string(),
                value: "List history entries".to_string(),
                indent: "  ".to_string(),
                gap: " -> ".to_string(),
            }
        );
    }
}
