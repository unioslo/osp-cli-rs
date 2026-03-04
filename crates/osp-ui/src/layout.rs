use std::collections::HashMap;

use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use crate::ResolvedRenderSettings;
use crate::document::{Block, Document, MregBlock, TableBlock};

#[derive(Debug, Clone)]
struct TableDescriptor {
    block_id: usize,
    headers: Vec<String>,
    raw_widths: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutContext {
    pub table_column_widths: HashMap<usize, Vec<usize>>,
    pub mreg_metrics: HashMap<usize, MregMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MregMetrics {
    pub key_width: usize,
    pub content_width: usize,
}

pub fn prepare_layout_context(
    document: &Document,
    settings: &ResolvedRenderSettings,
) -> LayoutContext {
    LayoutContext {
        table_column_widths: prepare_table_widths(document, settings.width, settings.unicode),
        mreg_metrics: prepare_mreg_metrics(
            document,
            settings.width,
            settings.indent_size,
            settings.margin,
        ),
    }
}

fn prepare_table_widths(
    document: &Document,
    width_limit: Option<usize>,
    unicode: bool,
) -> HashMap<usize, Vec<usize>> {
    let min_column_width = if unicode { 4 } else { 6 };

    let mut tables = Vec::new();
    for block in &document.blocks {
        let Block::Table(table) = block else {
            continue;
        };
        let block_id = block_identity(block);

        let raw_widths = compute_raw_table_widths(table, min_column_width);
        tables.push(TableDescriptor {
            block_id,
            headers: table.headers.clone(),
            raw_widths,
        });
    }

    if tables.is_empty() {
        return HashMap::new();
    }

    let mut global_width_by_header = HashMap::<String, usize>::new();
    for table in &tables {
        for (header, width) in table.headers.iter().zip(&table.raw_widths) {
            let entry = global_width_by_header
                .entry(header.clone())
                .or_insert(min_column_width);
            *entry = (*entry).max(*width);
        }
    }

    if let Some(limit) = width_limit {
        shrink_global_widths_to_fit(
            limit,
            min_column_width,
            &tables,
            &mut global_width_by_header,
        );
    }

    let mut output = HashMap::new();
    for table in &tables {
        let widths = table
            .headers
            .iter()
            .map(|header| {
                global_width_by_header
                    .get(header)
                    .copied()
                    .unwrap_or(min_column_width)
            })
            .collect::<Vec<usize>>();
        output.insert(table.block_id, widths);
    }

    output
}

fn compute_raw_table_widths(table: &TableBlock, min_column_width: usize) -> Vec<usize> {
    let mut widths = table
        .headers
        .iter()
        .map(|header| display_width(header).max(min_column_width))
        .collect::<Vec<usize>>();

    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(json_display_width(cell).max(min_column_width));
            }
        }
    }

    widths
}

fn shrink_global_widths_to_fit(
    width_limit: usize,
    min_column_width: usize,
    tables: &[TableDescriptor],
    global_width_by_header: &mut HashMap<String, usize>,
) {
    loop {
        let oversized = tables
            .iter()
            .filter(|table| table_width(table, global_width_by_header) > width_limit)
            .collect::<Vec<_>>();

        if oversized.is_empty() {
            break;
        }

        let mut selected_header: Option<&str> = None;
        let mut selected_width = 0usize;

        for table in oversized {
            for header in &table.headers {
                let width = global_width_by_header
                    .get(header)
                    .copied()
                    .unwrap_or(min_column_width);
                if width > min_column_width && width > selected_width {
                    selected_header = Some(header);
                    selected_width = width;
                }
            }
        }

        let Some(header) = selected_header else {
            break;
        };

        if let Some(width) = global_width_by_header.get_mut(header) {
            *width -= 1;
        }
    }
}

fn table_width(table: &TableDescriptor, widths: &HashMap<String, usize>) -> usize {
    if table.headers.is_empty() {
        return 0;
    }

    let content_width = table
        .headers
        .iter()
        .map(|header| widths.get(header).copied().unwrap_or(0))
        .sum::<usize>();
    let border_overhead = table.headers.len() * 3 + 1;
    content_width + border_overhead
}

fn prepare_mreg_metrics(
    document: &Document,
    width_limit: Option<usize>,
    indent_size: usize,
    margin: usize,
) -> HashMap<usize, MregMetrics> {
    let mut metrics = HashMap::new();

    for block in &document.blocks {
        let Block::Mreg(mreg) = block else {
            continue;
        };
        let block_id = block_identity(block);

        let key_width = compute_mreg_key_width(mreg).max(3);
        let available_width = width_limit.unwrap_or(100);
        let reserved = margin + indent_size + key_width + 4;
        let content_width = available_width.saturating_sub(reserved).max(16);

        metrics.insert(
            block_id,
            MregMetrics {
                key_width,
                content_width,
            },
        );
    }

    metrics
}

fn compute_mreg_key_width(block: &MregBlock) -> usize {
    block
        .rows
        .iter()
        .flat_map(|row| row.entries.iter())
        .map(|entry| display_width(&entry.key))
        .max()
        .unwrap_or(0)
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

fn json_display_width(value: &Value) -> usize {
    match value {
        Value::Null => display_width("null"),
        Value::Bool(v) => display_width(if *v { "true" } else { "false" }),
        Value::Number(v) => display_width(&v.to_string()),
        Value::String(v) => display_width(v),
        Value::Array(values) => {
            let joined = values
                .iter()
                .map(json_display_string)
                .collect::<Vec<String>>()
                .join(", ");
            display_width(&joined)
        }
        Value::Object(_) => display_width(&value.to_string()),
    }
}

fn json_display_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string().to_ascii_lowercase(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(values) => values
            .iter()
            .map(json_display_string)
            .collect::<Vec<String>>()
            .join(", "),
        Value::Object(_) => value.to_string(),
    }
}

pub(crate) fn block_identity(block: &Block) -> usize {
    block as *const Block as usize
}

#[cfg(test)]
mod tests {
    use super::{block_identity, prepare_layout_context};
    use crate::document::{
        Block, Document, MregBlock, MregEntry, MregRow, MregValue, TableBlock, TableStyle,
    };
    use crate::theme::DEFAULT_THEME_NAME;
    use crate::{RenderBackend, ResolvedRenderSettings};
    use serde_json::json;

    fn rich_settings(width: Option<usize>) -> ResolvedRenderSettings {
        ResolvedRenderSettings {
            backend: RenderBackend::Rich,
            color: false,
            unicode: true,
            width,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            style_overrides: crate::style::StyleOverrides::default(),
        }
    }

    #[test]
    fn shares_column_widths_by_header_name_across_tables() {
        let document = Document {
            blocks: vec![
                Block::Table(TableBlock {
                    style: TableStyle::Grid,
                    headers: vec!["name".to_string(), "value".to_string()],
                    rows: vec![vec![json!("short"), json!("abc")]],
                    header_pairs: Vec::new(),
                    align: None,
                    depth: 0,
                }),
                Block::Table(TableBlock {
                    style: TableStyle::Grid,
                    headers: vec!["name".to_string(), "value".to_string()],
                    rows: vec![vec![json!("very-long-name"), json!("x")]],
                    header_pairs: Vec::new(),
                    align: None,
                    depth: 0,
                }),
            ],
        };

        let context = prepare_layout_context(&document, &rich_settings(None));
        let first_id = block_identity(&document.blocks[0]);
        let second_id = block_identity(&document.blocks[1]);
        let first = context
            .table_column_widths
            .get(&first_id)
            .expect("first table widths should exist");
        let second = context
            .table_column_widths
            .get(&second_id)
            .expect("second table widths should exist");

        assert_eq!(first, second);
        assert!(first[0] >= "very-long-name".len());
    }

    #[test]
    fn shrinks_global_table_widths_when_terminal_is_narrow() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["name".to_string(), "description".to_string()],
                rows: vec![vec![
                    json!("alpha"),
                    json!("this text is intentionally long and should shrink"),
                ]],
                header_pairs: Vec::new(),
                align: None,
                depth: 0,
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(36)));
        let table_id = block_identity(&document.blocks[0]);
        let widths = context
            .table_column_widths
            .get(&table_id)
            .expect("table widths should exist");

        let total: usize = widths.iter().sum::<usize>() + widths.len() * 3 + 1;
        assert!(total <= 36);
    }

    #[test]
    fn computes_mreg_key_and_content_metrics() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                rows: vec![MregRow {
                    entries: vec![
                        MregEntry {
                            key: "uid".to_string(),
                            depth: 0,
                            value: MregValue::Scalar(json!("oistes")),
                        },
                        MregEntry {
                            key: "very_long_key".to_string(),
                            depth: 0,
                            value: MregValue::List(vec![json!("a"), json!("b")]),
                        },
                    ],
                }],
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(60)));
        let mreg_id = block_identity(&document.blocks[0]);
        let metrics = context
            .mreg_metrics
            .get(&mreg_id)
            .expect("mreg metrics should exist");

        assert!(metrics.key_width >= "very_long_key".len());
        assert!(metrics.content_width < 60);
        assert!(metrics.content_width >= 16);
    }
}
