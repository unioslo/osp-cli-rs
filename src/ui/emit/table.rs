use unicode_width::UnicodeWidthStr;

use crate::ui::doc::TableBlock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedTable {
    pub headers: Vec<PreparedCell>,
    pub rows: Vec<Vec<PreparedCell>>,
    pub widths: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedCell {
    pub raw: String,
    pub markdown: String,
    pub width: usize,
}

impl PreparedTable {
    pub(super) fn for_terminal(block: &TableBlock) -> Self {
        Self::prepare(block, 0)
    }

    pub(super) fn for_markdown(block: &TableBlock) -> Self {
        Self::prepare(block, 3)
    }

    fn prepare(block: &TableBlock, min_cell_width: usize) -> Self {
        let headers = prepare_cells(&block.headers);
        let rows = block
            .rows
            .iter()
            .map(|row| prepare_cells(row))
            .collect::<Vec<_>>();
        let mut widths = headers
            .iter()
            .map(|cell| cell.width.max(min_cell_width))
            .collect::<Vec<_>>();

        for row in &rows {
            for (index, cell) in row.iter().enumerate() {
                if let Some(width) = widths.get_mut(index) {
                    *width = (*width).max(cell.width.max(min_cell_width));
                }
            }
        }

        Self {
            headers,
            rows,
            widths,
        }
    }
}

fn prepare_cells(cells: &[String]) -> Vec<PreparedCell> {
    cells.iter().map(|cell| PreparedCell::new(cell)).collect()
}

impl PreparedCell {
    fn new(raw: &str) -> Self {
        Self {
            raw: raw.to_string(),
            markdown: raw.replace('|', "\\|"),
            width: UnicodeWidthStr::width(raw),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PreparedTable;
    use crate::ui::doc::{KeyValueRow, TableBlock};

    #[test]
    fn prepared_table_captures_shared_width_and_markdown_shape_unit() {
        let table = TableBlock {
            summary: vec![KeyValueRow {
                key: "count".to_string(),
                value: "1".to_string(),
                indent: None,
                gap: None,
            }],
            headers: vec!["na|me".to_string(), "id".to_string()],
            rows: vec![vec!["ali|ce".to_string(), "42".to_string()]],
        };

        let prepared = PreparedTable::for_markdown(&table);

        assert_eq!(prepared.widths, vec![6, 3]);
        assert_eq!(prepared.headers[0].raw, "na|me");
        assert_eq!(prepared.headers[0].markdown, "na\\|me");
        assert_eq!(prepared.rows[0][0].raw, "ali|ce");
        assert_eq!(prepared.rows[0][0].markdown, "ali\\|ce");
    }
}
