use std::collections::HashMap;

use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use crate::display::value_to_display;
use crate::document::{Block, Document, MregBlock, MregValue, TableBlock};
use crate::{RenderBackend, ResolvedRenderSettings, TableOverflow};

#[derive(Debug, Clone)]
struct TableDescriptor {
    block_id: u64,
    headers: Vec<String>,
    raw_widths: Vec<usize>,
    shrink_to_fit: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutContext {
    pub table_column_widths: HashMap<u64, Vec<usize>>,
    pub mreg_metrics: HashMap<u64, MregMetrics>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MregEntryMetrics {
    pub gap: usize,
    pub render_col: usize,
    pub first_gap: usize,
    pub first_pad: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MregMetrics {
    pub entry_metrics: Vec<Option<MregEntryMetrics>>,
}

pub fn prepare_layout_context(
    document: &Document,
    settings: &ResolvedRenderSettings,
) -> LayoutContext {
    let allow_shrink = settings.backend == RenderBackend::Rich
        && !matches!(settings.table_overflow, TableOverflow::None);
    let width_limit = if matches!(settings.table_overflow, TableOverflow::None) {
        None
    } else {
        settings.width
    };
    LayoutContext {
        table_column_widths: prepare_table_widths(
            document,
            width_limit,
            settings.unicode,
            allow_shrink,
        ),
        mreg_metrics: prepare_mreg_metrics(document, settings.indent_size),
    }
}

fn prepare_table_widths(
    document: &Document,
    width_limit: Option<usize>,
    unicode: bool,
    force_shrink_to_fit: bool,
) -> HashMap<u64, Vec<usize>> {
    let min_column_width = if unicode { 4 } else { 6 };

    let mut tables = Vec::new();
    for block in &document.blocks {
        let Block::Table(table) = block else {
            continue;
        };
        let block_id = table.block_id;

        let raw_widths = compute_raw_table_widths(table, min_column_width);
        tables.push(TableDescriptor {
            block_id,
            headers: table.headers.clone(),
            raw_widths,
            shrink_to_fit: table.shrink_to_fit || force_shrink_to_fit,
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
        let widths = if table.shrink_to_fit || force_shrink_to_fit {
            table
                .headers
                .iter()
                .map(|header| {
                    global_width_by_header
                        .get(header)
                        .copied()
                        .unwrap_or(min_column_width)
                })
                .collect::<Vec<usize>>()
        } else {
            table.raw_widths.clone()
        };
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
            .filter(|table| {
                table.shrink_to_fit && table_width(table, global_width_by_header) > width_limit
            })
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

fn prepare_mreg_metrics(document: &Document, indent_size: usize) -> HashMap<u64, MregMetrics> {
    let mut metrics = HashMap::new();

    for block in &document.blocks {
        let Block::Mreg(mreg) = block else {
            continue;
        };
        let block_id = mreg.block_id;

        let key_width = compute_mreg_key_width(mreg, indent_size);
        let value_column = key_width + 2;
        let entry_metrics = mreg
            .rows
            .iter()
            .flat_map(|row| row.entries.iter())
            .map(|entry| match entry.value {
                MregValue::Group | MregValue::Separator => None,
                _ => {
                    let base_len =
                        entry.depth * indent_size + mreg_alignment_key_width(&entry.key) + 1;
                    let full_len = entry.depth * indent_size + display_width(&entry.key) + 1;
                    let gap = value_column.saturating_sub(base_len).max(1);
                    let render_col = value_column.max(full_len + 1);
                    Some(MregEntryMetrics {
                        gap,
                        render_col,
                        first_gap: render_col.saturating_sub(full_len),
                        first_pad: render_col,
                    })
                }
            })
            .collect();

        metrics.insert(block_id, MregMetrics { entry_metrics });
    }

    metrics
}

fn compute_mreg_key_width(block: &MregBlock, indent_size: usize) -> usize {
    block
        .rows
        .iter()
        .flat_map(|row| row.entries.iter())
        .filter(|entry| !matches!(entry.value, MregValue::Group | MregValue::Separator))
        .map(|entry| entry.depth * indent_size + mreg_alignment_key_width(&entry.key))
        .max()
        .unwrap_or(0)
}

fn mreg_alignment_key_width(key: &str) -> usize {
    display_width(strip_count_suffix(key))
}

fn strip_count_suffix(key: &str) -> &str {
    if let Some(prefix_end) = key.rfind(" (") {
        let suffix = &key[prefix_end + 2..];
        if let Some(count) = suffix.strip_suffix(')')
            && !count.is_empty()
            && count.bytes().all(|byte| byte.is_ascii_digit())
        {
            return &key[..prefix_end];
        }
    }
    key
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

fn json_display_width(value: &Value) -> usize {
    display_width(&value_to_display(value))
}

#[cfg(test)]
mod tests {
    use super::prepare_layout_context;
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
            table_overflow: crate::TableOverflow::Clip,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: crate::theme::resolve_theme(DEFAULT_THEME_NAME),
            style_overrides: crate::style::StyleOverrides::default(),
        }
    }

    fn plain_settings(width: Option<usize>) -> ResolvedRenderSettings {
        ResolvedRenderSettings {
            backend: RenderBackend::Plain,
            color: false,
            unicode: false,
            width,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::TableOverflow::Clip,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: crate::theme::resolve_theme(DEFAULT_THEME_NAME),
            style_overrides: crate::style::StyleOverrides::default(),
        }
    }

    #[test]
    fn shares_column_widths_by_header_name_across_tables() {
        let document = Document {
            blocks: vec![
                Block::Table(TableBlock {
                    block_id: 1,
                    style: TableStyle::Grid,
                    headers: vec!["name".to_string(), "value".to_string()],
                    rows: vec![vec![json!("short"), json!("abc")]],
                    header_pairs: Vec::new(),
                    align: None,
                    shrink_to_fit: true,
                    depth: 0,
                }),
                Block::Table(TableBlock {
                    block_id: 2,
                    style: TableStyle::Grid,
                    headers: vec!["name".to_string(), "value".to_string()],
                    rows: vec![vec![json!("very-long-name"), json!("x")]],
                    header_pairs: Vec::new(),
                    align: None,
                    shrink_to_fit: true,
                    depth: 0,
                }),
            ],
        };

        let context = prepare_layout_context(&document, &rich_settings(None));
        let first_id = match &document.blocks[0] {
            Block::Table(table) => table.block_id,
            _ => 0,
        };
        let second_id = match &document.blocks[1] {
            Block::Table(table) => table.block_id,
            _ => 0,
        };
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
                block_id: 1,
                style: TableStyle::Grid,
                headers: vec!["name".to_string(), "description".to_string()],
                rows: vec![vec![
                    json!("alpha"),
                    json!("this text is intentionally long and should shrink"),
                ]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(36)));
        let table_id = match &document.blocks[0] {
            Block::Table(table) => table.block_id,
            _ => 0,
        };
        let widths = context
            .table_column_widths
            .get(&table_id)
            .expect("table widths should exist");

        let total: usize = widths.iter().sum::<usize>() + widths.len() * 3 + 1;
        assert!(total <= 36);
    }

    #[test]
    fn rich_mode_shrinks_even_when_table_opted_out() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 1,
                style: TableStyle::Grid,
                headers: vec!["name".to_string(), "description".to_string()],
                rows: vec![vec![
                    json!("alpha"),
                    json!("this text is intentionally long and should shrink"),
                ]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: false,
                depth: 0,
            })],
        };

        let rich_context = prepare_layout_context(&document, &rich_settings(Some(36)));
        let plain_context = prepare_layout_context(&document, &plain_settings(Some(36)));
        let table_id = match &document.blocks[0] {
            Block::Table(table) => table.block_id,
            _ => 0,
        };
        let rich_widths = rich_context
            .table_column_widths
            .get(&table_id)
            .expect("rich table widths should exist");
        let plain_widths = plain_context
            .table_column_widths
            .get(&table_id)
            .expect("plain table widths should exist");

        let rich_total: usize = rich_widths.iter().sum::<usize>() + rich_widths.len() * 3 + 1;
        let plain_total: usize = plain_widths.iter().sum::<usize>() + plain_widths.len() * 3 + 1;
        assert!(rich_total <= 36);
        assert!(plain_total > 36);
    }

    #[test]
    fn computes_per_entry_mreg_alignment_metrics() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                block_id: 1,
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
                            value: MregValue::VerticalList(vec![json!("a"), json!("b")]),
                        },
                    ],
                }],
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(60)));
        let mreg_id = match &document.blocks[0] {
            Block::Mreg(mreg) => mreg.block_id,
            _ => 0,
        };
        let metrics = context
            .mreg_metrics
            .get(&mreg_id)
            .expect("mreg metrics should exist");

        assert_eq!(metrics.entry_metrics.len(), 2);
        let uid = metrics.entry_metrics[0].expect("scalar metrics");
        let list = metrics.entry_metrics[1].expect("vertical list metrics");
        assert!(uid.gap > 1);
        assert_eq!(list.gap, 1);
        assert!(list.first_pad >= list.render_col);
    }

    #[test]
    fn skips_alignment_metrics_for_group_entries() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                block_id: 1,
                rows: vec![MregRow {
                    entries: vec![
                        MregEntry {
                            key: "group".to_string(),
                            depth: 0,
                            value: MregValue::Group,
                        },
                        MregEntry {
                            key: "uid".to_string(),
                            depth: 1,
                            value: MregValue::Scalar(json!("oistes")),
                        },
                    ],
                }],
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(60)));
        let metrics = context.mreg_metrics.get(&1).expect("metrics");
        assert_eq!(metrics.entry_metrics.len(), 2);
        assert!(metrics.entry_metrics[0].is_none());
        assert!(metrics.entry_metrics[1].is_some());
    }
}
