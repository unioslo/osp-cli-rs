use comfy_table::{Cell, ContentArrangement, Table, presets};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::document::{Block, Document, MregBlock, MregValue, PanelRules, TableBlock, TableStyle};
use crate::layout::{LayoutContext, MregMetrics, prepare_layout_context};
use crate::style::{StyleToken, apply_style, apply_style_spec};
use crate::{RenderBackend, ResolvedRenderSettings};

const INLINE_LIST_MAX_ITEMS: usize = 3;
const GRID_LIST_MIN_ITEMS: usize = 9;
const GRID_MAX_COLUMNS: usize = 4;
const GRID_GAP: usize = 2;

pub fn render_document(document: &Document, settings: ResolvedRenderSettings) -> String {
    let layout = prepare_layout_context(document, &settings);
    let mut out = String::new();
    for (block_index, block) in document.blocks.iter().enumerate() {
        out.push_str(&render_block(block, block_index, &settings, &layout));
    }
    out
}

fn render_block(
    block: &Block,
    block_index: usize,
    settings: &ResolvedRenderSettings,
    layout: &LayoutContext,
) -> String {
    match settings.backend {
        RenderBackend::Plain => render_block_plain(block, block_index, settings, layout),
        RenderBackend::Rich => render_block_rich(block, block_index, settings, layout),
    }
}

fn render_block_plain(
    block: &Block,
    block_index: usize,
    settings: &ResolvedRenderSettings,
    layout: &LayoutContext,
) -> String {
    match block {
        Block::Line(line) => render_line_block(line),
        Block::Panel(panel) => render_panel_block(
            panel,
            RenderBackend::Plain,
            false,
            false,
            settings.width,
            &settings.theme_name,
        ),
        Block::Code(code) => render_code_block(code),
        Block::Json(json) => {
            serde_json::to_string_pretty(&json.payload).unwrap_or_else(|_| "[]".to_string())
        }
        Block::Value(values) => render_value_block(&values.values),
        Block::Mreg(mreg) => render_mreg_block(
            mreg,
            false,
            false,
            settings.width,
            &settings.theme_name,
            layout.mreg_metrics.get(&block_index).copied(),
        ),
        Block::Table(table) => render_table_block(
            table,
            false,
            false,
            settings.width,
            &settings.theme_name,
            layout
                .table_column_widths
                .get(&block_index)
                .map(Vec::as_slice),
        ),
    }
}

fn render_block_rich(
    block: &Block,
    block_index: usize,
    settings: &ResolvedRenderSettings,
    layout: &LayoutContext,
) -> String {
    match block {
        Block::Line(line) => render_line_block(line),
        Block::Panel(panel) => render_panel_block(
            panel,
            RenderBackend::Rich,
            settings.color,
            settings.unicode,
            settings.width,
            &settings.theme_name,
        ),
        Block::Code(code) => render_code_block(code),
        Block::Json(json) => {
            serde_json::to_string_pretty(&json.payload).unwrap_or_else(|_| "[]".to_string())
        }
        Block::Value(values) => render_value_block(&values.values),
        Block::Mreg(mreg) => render_mreg_block(
            mreg,
            settings.color,
            settings.unicode,
            settings.width,
            &settings.theme_name,
            layout.mreg_metrics.get(&block_index).copied(),
        ),
        Block::Table(table) => render_table_block(
            table,
            settings.unicode,
            settings.color,
            settings.width,
            &settings.theme_name,
            layout
                .table_column_widths
                .get(&block_index)
                .map(Vec::as_slice),
        ),
    }
}

fn render_value_block(values: &[String]) -> String {
    if values.is_empty() {
        String::new()
    } else {
        format!("{}\n", values.join("\n"))
    }
}

fn render_line_block(block: &crate::document::LineBlock) -> String {
    let mut out = String::new();
    for part in &block.parts {
        out.push_str(&part.text);
    }
    out.push('\n');
    out
}

fn render_code_block(block: &crate::document::CodeBlock) -> String {
    if block.code.ends_with('\n') {
        block.code.clone()
    } else {
        format!("{}\n", block.code)
    }
}

fn render_panel_block(
    block: &crate::document::PanelBlock,
    backend: RenderBackend,
    color: bool,
    unicode: bool,
    width: Option<usize>,
    theme_name: &str,
) -> String {
    let divider = section_divider(
        block.title.as_deref().unwrap_or(""),
        block.kind.as_deref(),
        unicode,
        width,
        color,
        theme_name,
    );
    let inner = render_document(
        &block.body,
        ResolvedRenderSettings {
            backend,
            color,
            unicode,
            width,
            theme_name: theme_name.to_string(),
        },
    );
    match block.rules {
        PanelRules::None => inner,
        PanelRules::Top => format!("{divider}\n{inner}"),
        PanelRules::Bottom => format!("{inner}{divider}\n"),
        PanelRules::Both => format!("{divider}\n{inner}{divider}\n"),
    }
}

fn render_mreg_block(
    block: &MregBlock,
    color: bool,
    unicode: bool,
    width: Option<usize>,
    theme_name: &str,
    metrics: Option<MregMetrics>,
) -> String {
    if block.rows.is_empty() {
        return String::new();
    }

    let key_width = metrics
        .map(|value| value.key_width)
        .unwrap_or_else(|| compute_mreg_key_width(block).max(3));
    let available_width = metrics
        .map(|value| value.content_width)
        .unwrap_or_else(|| width.unwrap_or(100).saturating_sub(key_width + 4).max(16));

    let mut out = String::new();
    for (row_index, row) in block.rows.iter().enumerate() {
        for entry in &row.entries {
            let padded_key = pad_display_width(&entry.key, key_width);
            let key_text = apply_style(&padded_key, StyleToken::MregKey, color, theme_name);

            match &entry.value {
                MregValue::Scalar(value) => {
                    let value_text = style_value_cell(value, color, theme_name);
                    out.push_str(&format!("{key_text}: {value_text}\n"));
                }
                MregValue::List(values) => {
                    let rendered = render_mreg_list(values, available_width, unicode);
                    match rendered {
                        MregListRender::Inline(line) => {
                            out.push_str(&format!("{key_text}: {line}\n"));
                        }
                        MregListRender::Vertical(lines) => {
                            let heading = apply_style(
                                &format!("{} ({})", entry.key, values.len()),
                                StyleToken::MregKey,
                                color,
                                theme_name,
                            );
                            let bullet = if unicode { "•" } else { "-" };
                            out.push_str(&format!("{heading}\n"));
                            for line in lines {
                                out.push_str(&format!("  {bullet} {line}\n"));
                            }
                        }
                        MregListRender::Grid(lines) => {
                            let heading = apply_style(
                                &format!("{} ({})", entry.key, values.len()),
                                StyleToken::MregKey,
                                color,
                                theme_name,
                            );
                            out.push_str(&format!("{heading}\n"));
                            for line in lines {
                                out.push_str("  ");
                                out.push_str(&line.join("  "));
                                out.push('\n');
                            }
                        }
                    }
                }
            }
        }

        if block.rows.len() > 1 && row_index < block.rows.len() - 1 {
            out.push('\n');
        }
    }

    out
}

fn section_divider(
    title: &str,
    kind: Option<&str>,
    unicode: bool,
    width: Option<usize>,
    color: bool,
    theme_name: &str,
) -> String {
    let fill_char = if unicode { '─' } else { '-' };
    let target_width = width
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(24)
        .max(12);
    let title = title.trim();

    let raw = if title.is_empty() {
        fill_char.to_string().repeat(target_width)
    } else {
        let prefix = if unicode {
            format!("─ {title} ")
        } else {
            format!("- {title} ")
        };
        let prefix_width = prefix.chars().count();
        if prefix_width >= target_width {
            prefix
        } else {
            format!(
                "{prefix}{}",
                fill_char.to_string().repeat(target_width - prefix_width)
            )
        }
    };

    if color {
        apply_style(
            &raw,
            section_style_token(kind).unwrap_or(StyleToken::MessageInfo),
            true,
            theme_name,
        )
    } else {
        raw
    }
}

fn section_style_token(kind: Option<&str>) -> Option<StyleToken> {
    match kind.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("error") => Some(StyleToken::MessageError),
        Some("warning") => Some(StyleToken::MessageWarning),
        Some("success") => Some(StyleToken::MessageSuccess),
        Some("info") => Some(StyleToken::MessageInfo),
        Some("trace") => Some(StyleToken::MessageTrace),
        _ => None,
    }
}

enum MregListRender {
    Inline(String),
    Vertical(Vec<String>),
    Grid(Vec<Vec<String>>),
}

fn render_mreg_list(values: &[String], available_width: usize, unicode: bool) -> MregListRender {
    if values.is_empty() {
        return MregListRender::Inline(String::new());
    }

    let inline = values.join(", ");
    if values.len() <= INLINE_LIST_MAX_ITEMS && display_width(&inline) <= available_width {
        return MregListRender::Inline(inline);
    }

    if values.len() < GRID_LIST_MIN_ITEMS {
        return MregListRender::Vertical(values.to_vec());
    }

    let max_item_width = values
        .iter()
        .map(|value| display_width(value))
        .max()
        .unwrap_or(1);
    let cell_width = max_item_width.min(available_width.max(1)).max(4);
    let columns = ((available_width + GRID_GAP) / (cell_width + GRID_GAP))
        .max(1)
        .min(GRID_MAX_COLUMNS)
        .min(values.len());

    if columns < 2 {
        return MregListRender::Vertical(values.to_vec());
    }

    let rows_count = values.len().div_ceil(columns);
    let mut rows = Vec::new();

    for row_index in 0..rows_count {
        let mut row = Vec::new();
        for column_index in 0..columns {
            let value_index = row_index * columns + column_index;
            if value_index >= values.len() {
                continue;
            }

            let value = truncate_display_width(&values[value_index], cell_width, unicode);
            let value = if column_index == columns - 1 {
                value
            } else {
                pad_display_width(&value, cell_width)
            };
            row.push(value);
        }
        rows.push(row);
    }

    MregListRender::Grid(rows)
}

fn render_table_block(
    block: &TableBlock,
    unicode: bool,
    color: bool,
    width: Option<usize>,
    theme_name: &str,
    column_widths: Option<&[usize]>,
) -> String {
    if block.rows.is_empty() {
        return String::new();
    }

    match block.style {
        TableStyle::Grid => {
            render_grid_table(block, unicode, color, width, theme_name, column_widths)
        }
        TableStyle::Markdown => render_markdown_table(block, unicode, column_widths),
    }
}

fn render_grid_table(
    block: &TableBlock,
    unicode: bool,
    color: bool,
    _width: Option<usize>,
    theme_name: &str,
    column_widths: Option<&[usize]>,
) -> String {
    let (headers, rows, _) =
        truncate_table_to_widths(&block.headers, &block.rows, column_widths, unicode);

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    if unicode {
        table.load_preset(presets::UTF8_FULL);
    } else {
        table.load_preset(presets::ASCII_FULL);
    }

    let header_cells = headers
        .iter()
        .map(|header| {
            Cell::new(apply_style(
                header,
                StyleToken::TableHeader,
                color,
                theme_name,
            ))
        })
        .collect::<Vec<Cell>>();
    table.set_header(header_cells);

    for row in &rows {
        let styled_row = row
            .iter()
            .map(|value| style_value_cell(value, color, theme_name))
            .map(Cell::new)
            .collect::<Vec<Cell>>();
        table.add_row(styled_row);
    }

    format!("{table}\n")
}

fn render_markdown_table(
    block: &TableBlock,
    unicode: bool,
    column_widths: Option<&[usize]>,
) -> String {
    let (headers, rows, _widths) =
        truncate_table_to_widths(&block.headers, &block.rows, column_widths, unicode);
    if headers.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push('|');
    for header in &headers {
        out.push(' ');
        out.push_str(&escape_markdown_cell(header));
        out.push(' ');
        out.push('|');
    }
    out.push('\n');

    out.push('|');
    for _ in &headers {
        out.push_str(" --- |");
    }
    out.push('\n');

    for row in &rows {
        out.push('|');
        for cell in row {
            out.push(' ');
            out.push_str(&escape_markdown_cell(cell));
            out.push(' ');
            out.push('|');
        }
        out.push('\n');
    }

    out
}

fn truncate_table_to_widths(
    headers: &[String],
    rows: &[Vec<String>],
    column_widths: Option<&[usize]>,
    unicode: bool,
) -> (Vec<String>, Vec<Vec<String>>, Vec<usize>) {
    let fallback_widths = compute_fallback_widths(headers, rows, if unicode { 4 } else { 6 });
    let widths = match column_widths {
        Some(widths) if widths.len() == headers.len() => widths.to_vec(),
        _ => fallback_widths,
    };

    let headers = headers
        .iter()
        .zip(&widths)
        .map(|(header, width)| truncate_display_width(header, *width, unicode))
        .collect::<Vec<String>>();

    let rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(index, cell)| {
                    let width = widths.get(index).copied().unwrap_or(8);
                    truncate_display_width(cell, width, unicode)
                })
                .collect::<Vec<String>>()
        })
        .collect::<Vec<Vec<String>>>();

    (headers, rows, widths)
}

fn compute_fallback_widths(
    headers: &[String],
    rows: &[Vec<String>],
    min_column_width: usize,
) -> Vec<usize> {
    let mut widths = headers
        .iter()
        .map(|header| display_width(header).max(min_column_width))
        .collect::<Vec<usize>>();

    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(display_width(cell).max(min_column_width));
            }
        }
    }

    widths
}

fn style_value_cell(value: &str, color: bool, theme_name: &str) -> String {
    let trimmed = value.trim();
    if !color || theme_name.eq_ignore_ascii_case("plain") {
        return value.to_string();
    }

    if is_hex_color(trimmed) {
        return apply_style_spec(value, trimmed, true);
    }

    if trimmed.eq_ignore_ascii_case("true") {
        return apply_style(value, StyleToken::MessageSuccess, true, theme_name);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return apply_style(value, StyleToken::MessageError, true, theme_name);
    }
    if trimmed.eq_ignore_ascii_case("null") {
        return apply_style(value, StyleToken::MessageTrace, true, theme_name);
    }
    if is_numeric(trimmed) {
        return apply_style(value, StyleToken::MessageInfo, true, theme_name);
    }

    value.to_string()
}

fn is_numeric(value: &str) -> bool {
    value.parse::<i64>().is_ok() || value.parse::<f64>().is_ok()
}

fn is_hex_color(value: &str) -> bool {
    if value.len() != 7 || !value.starts_with('#') {
        return false;
    }
    value
        .as_bytes()
        .iter()
        .skip(1)
        .all(|byte| byte.is_ascii_hexdigit())
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

fn pad_display_width(value: &str, target_width: usize) -> String {
    let current = display_width(value);
    if current >= target_width {
        return value.to_string();
    }

    let mut out = String::with_capacity(value.len() + (target_width - current));
    out.push_str(value);
    out.push_str(&" ".repeat(target_width - current));
    out
}

fn truncate_display_width(value: &str, max_width: usize, unicode: bool) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }

    let suffix = if unicode { "…" } else { "..." };
    let suffix_width = UnicodeWidthStr::width(suffix);
    if max_width <= suffix_width {
        return if unicode {
            "…".to_string()
        } else {
            ".".repeat(max_width.min(3))
        };
    }

    let mut out = String::new();
    let mut width = 0usize;
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width + suffix_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push_str(suffix);
    out
}

fn escape_markdown_cell(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::render_document;
    use crate::RenderBackend;
    use crate::ResolvedRenderSettings;
    use crate::document::{
        Block, Document, JsonBlock, MregBlock, MregEntry, MregRow, MregValue, TableBlock,
        TableStyle, ValueBlock,
    };
    use serde_json::json;

    fn settings(backend: RenderBackend, color: bool, unicode: bool) -> ResolvedRenderSettings {
        ResolvedRenderSettings {
            backend,
            color,
            unicode,
            width: None,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
        }
    }

    #[test]
    fn render_value_block_appends_trailing_newline() {
        let document = Document {
            blocks: vec![Block::Value(ValueBlock {
                values: vec!["one".to_string(), "two".to_string()],
            })],
        };
        assert_eq!(
            render_document(&document, settings(RenderBackend::Plain, false, false)),
            "one\ntwo\n"
        );
    }

    #[test]
    fn render_mreg_respects_color_toggle() {
        let block = MregBlock {
            rows: vec![MregRow {
                entries: vec![MregEntry {
                    key: "uid".to_string(),
                    value: MregValue::Scalar("oistes".to_string()),
                }],
            }],
        };
        let plain = render_document(
            &Document {
                blocks: vec![Block::Mreg(block.clone())],
            },
            settings(RenderBackend::Plain, false, false),
        );
        let colored = render_document(
            &Document {
                blocks: vec![Block::Mreg(block)],
            },
            settings(RenderBackend::Rich, true, false),
        );

        assert_eq!(plain, "uid: oistes\n");
        assert!(colored.contains("uid"));
        assert!(colored.contains("\x1b["));
    }

    #[test]
    fn mreg_short_lists_render_inline() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "members".to_string(),
                        value: MregValue::List(vec![
                            "alice".to_string(),
                            "bob".to_string(),
                            "carol".to_string(),
                        ]),
                    }],
                }],
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Plain, false, false));
        assert!(rendered.contains("members: alice, bob, carol"));
    }

    #[test]
    fn mreg_large_lists_use_grid_layout() {
        let values = (1..=12)
            .map(|index| format!("item-{index}"))
            .collect::<Vec<String>>();
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "members".to_string(),
                        value: MregValue::List(values),
                    }],
                }],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Plain,
                color: false,
                unicode: false,
                width: Some(48),
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            },
        );

        assert!(rendered.contains("members (12)"));
        assert!(
            rendered
                .lines()
                .any(|line| line.contains("item-1") && line.contains("item-2"))
        );
    }

    #[test]
    fn render_json_block_is_pretty() {
        let document = Document {
            blocks: vec![Block::Json(JsonBlock {
                payload: json!([{"uid": "oistes"}]),
            })],
        };
        let rendered = render_document(&document, settings(RenderBackend::Plain, false, false));
        assert!(rendered.contains('\n'));
        assert!(rendered.contains("\"uid\""));
    }

    #[test]
    fn render_table_toggles_border_style() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec!["oistes".to_string()]],
            })],
        };

        let unicode = render_document(&document, settings(RenderBackend::Rich, false, true));
        let ascii = render_document(&document, settings(RenderBackend::Plain, false, false));

        assert!(unicode.contains('┌'));
        assert!(ascii.contains('+'));
    }

    #[test]
    fn table_color_never_has_no_ansi_escape_codes() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec!["oistes".to_string()]],
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Rich, false, true));
        assert!(!rendered.contains("\x1b["));
    }

    #[test]
    fn table_unicode_off_has_no_box_drawing_characters() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec!["oistes".to_string()]],
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Rich, false, false));
        for ch in ['┌', '┐', '└', '┘', '│', '─', '┬', '┴', '┼'] {
            assert!(!rendered.contains(ch));
        }
    }

    #[test]
    fn markdown_table_render_has_pipe_format() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Markdown,
                headers: vec!["uid".to_string(), "group".to_string()],
                rows: vec![vec!["oistes".to_string(), "uio".to_string()]],
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Plain, false, false));
        assert!(rendered.contains("| uid"));
        assert!(rendered.contains("| ---"));
        assert!(rendered.contains("| oistes"));
    }

    #[test]
    fn width_limit_truncates_wide_cells() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string(), "description".to_string()],
                rows: vec![vec![
                    "oistes".to_string(),
                    "this-is-a-very-long-cell-that-should-truncate".to_string(),
                ]],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Plain,
                color: false,
                unicode: false,
                width: Some(40),
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            },
        );

        assert!(rendered.contains("..."));
    }

    #[test]
    fn plain_theme_does_not_style_hex_cells() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["color".to_string()],
                rows: vec![vec!["#ff00ff".to_string()]],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Rich,
                color: true,
                unicode: false,
                width: None,
                theme_name: "plain".to_string(),
            },
        );
        assert!(!rendered.contains("\x1b["));
    }

    #[test]
    fn theme_hex_values_render_with_truecolor_when_enabled() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["color".to_string()],
                rows: vec![vec!["#ff00ff".to_string()]],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Rich,
                color: true,
                unicode: true,
                width: None,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            },
        );

        assert!(rendered.contains("\x1b[38;2;255;0;255m"));
    }
}
