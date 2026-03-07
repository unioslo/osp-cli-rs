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

        // Batch the current winner until the next decision point instead of
        // shaving one character per pass. That preserves the widest-header
        // policy while avoiding O(width overflow) iterations.
        let Some((header, amount)) = next_shrink_step(
            width_limit,
            min_column_width,
            &oversized,
            global_width_by_header,
        ) else {
            break;
        };

        if let Some(width) = global_width_by_header.get_mut(header) {
            *width = width.saturating_sub(amount);
        }
    }
}

fn next_shrink_step<'a>(
    width_limit: usize,
    min_column_width: usize,
    oversized: &[&'a TableDescriptor],
    global_width_by_header: &HashMap<String, usize>,
) -> Option<(&'a str, usize)> {
    let mut selected_header: Option<&'a str> = None;
    let mut selected_width = 0usize;
    let mut second_widest = min_column_width;
    let mut selected_ties = 0usize;

    for table in oversized {
        for header in &table.headers {
            let width = global_width_by_header
                .get(header)
                .copied()
                .unwrap_or(min_column_width);
            if width <= min_column_width {
                continue;
            }

            if width > selected_width {
                second_widest = selected_width.max(min_column_width);
                selected_header = Some(header.as_str());
                selected_width = width;
                selected_ties = 1;
            } else if width == selected_width {
                selected_ties += 1;
            } else {
                second_widest = second_widest.max(width);
            }
        }
    }

    let header = selected_header?;
    let max_overflow = oversized
        .iter()
        .filter(|table| table.headers.iter().any(|candidate| candidate == header))
        .map(|table| table_width(table, global_width_by_header).saturating_sub(width_limit))
        .max()
        .unwrap_or(0);

    let rank_limited_amount = if selected_ties > 1 {
        1
    } else {
        selected_width.saturating_sub(second_widest.max(min_column_width))
    };
    let amount = rank_limited_amount.min(max_overflow).max(1);

    Some((header, amount))
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
    use super::{
        TableDescriptor, compute_raw_table_widths, next_shrink_step, prepare_layout_context,
        shrink_global_widths_to_fit, strip_count_suffix,
    };
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
            table_border: crate::TableBorderStyle::Square,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: crate::theme::resolve_theme(DEFAULT_THEME_NAME),
            style_overrides: crate::style::StyleOverrides::default(),
            chrome_frame: crate::messages::SectionFrameStyle::Top,
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
            table_border: crate::TableBorderStyle::Square,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: crate::theme::resolve_theme(DEFAULT_THEME_NAME),
            style_overrides: crate::style::StyleOverrides::default(),
            chrome_frame: crate::messages::SectionFrameStyle::Top,
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
        let first = context
            .table_column_widths
            .get(&1)
            .expect("first table widths should exist");
        let second = context
            .table_column_widths
            .get(&2)
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
        let widths = context
            .table_column_widths
            .get(&1)
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
        let rich_widths = rich_context
            .table_column_widths
            .get(&1)
            .expect("rich table widths should exist");
        let plain_widths = plain_context
            .table_column_widths
            .get(&1)
            .expect("plain table widths should exist");

        let rich_total: usize = rich_widths.iter().sum::<usize>() + rich_widths.len() * 3 + 1;
        let plain_total: usize = plain_widths.iter().sum::<usize>() + plain_widths.len() * 3 + 1;
        assert!(rich_total <= 36);
        assert!(plain_total > 36);
    }

    #[test]
    fn tie_widths_shrink_without_starving_one_column() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 1,
                style: TableStyle::Grid,
                headers: vec!["left".to_string(), "right".to_string()],
                rows: vec![vec![json!("abcdefghijk"), json!("mnopqrstuvw")]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(27)));
        let widths = context
            .table_column_widths
            .get(&1)
            .expect("table widths should exist");

        let total: usize = widths.iter().sum::<usize>() + widths.len() * 3 + 1;
        assert!(total <= 27);
        assert!(widths[0].abs_diff(widths[1]) <= 1);
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
        let metrics = context
            .mreg_metrics
            .get(&1)
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

    #[test]
    fn leaves_empty_header_tables_with_empty_widths() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 7,
                style: TableStyle::Grid,
                headers: Vec::new(),
                rows: vec![Vec::new()],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(8)));
        let widths = context
            .table_column_widths
            .get(&7)
            .expect("empty table widths should exist");

        assert!(widths.is_empty());
    }

    #[test]
    fn stops_shrinking_when_columns_have_hit_minimum_width() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 9,
                style: TableStyle::Grid,
                headers: vec!["a".to_string(), "b".to_string()],
                rows: vec![vec![json!("x"), json!("y")]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let context = prepare_layout_context(&document, &rich_settings(Some(10)));
        let widths = context
            .table_column_widths
            .get(&9)
            .expect("table widths should exist");

        assert_eq!(widths, &vec![4, 4]);
        assert!(widths.iter().sum::<usize>() + widths.len() * 3 + 1 > 10);
    }

    #[test]
    fn strip_count_suffix_ignores_numeric_counts_only() {
        assert_eq!(strip_count_suffix("hosts (12)"), "hosts");
        assert_eq!(strip_count_suffix("hosts"), "hosts");
        assert_eq!(strip_count_suffix("hosts (a)"), "hosts (a)");
    }

    #[test]
    fn compute_raw_table_widths_expand_for_cell_content() {
        let table = TableBlock {
            block_id: 1,
            style: TableStyle::Grid,
            headers: vec!["name".to_string(), "value".to_string()],
            rows: vec![vec![
                json!("alice"),
                json!("this is longer than the header"),
            ]],
            header_pairs: Vec::new(),
            align: None,
            shrink_to_fit: true,
            depth: 0,
        };

        let widths = compute_raw_table_widths(&table, 4);

        assert!(widths[1] > "value".len());
    }

    #[test]
    fn next_shrink_step_uses_second_widest_column_gap() {
        let tables = vec![TableDescriptor {
            block_id: 1,
            headers: vec![
                "name".to_string(),
                "description".to_string(),
                "owner".to_string(),
            ],
            raw_widths: vec![12, 9, 6],
            shrink_to_fit: true,
        }];
        let widths = std::collections::HashMap::from([
            ("name".to_string(), 12usize),
            ("description".to_string(), 9usize),
            ("owner".to_string(), 6usize),
        ]);

        let (header, amount) =
            next_shrink_step(24, 4, &[&tables[0]], &widths).expect("shrink step");

        assert_eq!(header, "name");
        assert_eq!(amount, 3);
    }

    #[test]
    fn shrink_global_widths_updates_selected_header_width() {
        let tables = vec![TableDescriptor {
            block_id: 1,
            headers: vec!["name".to_string(), "description".to_string()],
            raw_widths: vec![12, 24],
            shrink_to_fit: true,
        }];
        let mut widths = std::collections::HashMap::from([
            ("name".to_string(), 12usize),
            ("description".to_string(), 24usize),
        ]);

        shrink_global_widths_to_fit(28, 4, &tables, &mut widths);

        assert!(widths["description"] < 24);
    }
}
