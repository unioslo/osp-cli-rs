use comfy_table::{Cell, ContentArrangement, Table, presets};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::document::{Block, Document, MregValue, TableStyle};
use crate::style::{StyleToken, apply_style, apply_style_spec};
use crate::{RenderBackend, ResolvedRenderSettings};

pub fn render_document(document: &Document, settings: ResolvedRenderSettings) -> String {
    let mut out = String::new();
    for block in &document.blocks {
        out.push_str(&render_block(block, &settings));
    }
    out
}

fn render_block(block: &Block, settings: &ResolvedRenderSettings) -> String {
    match settings.backend {
        RenderBackend::Plain => render_block_plain(block, &settings.theme_name),
        RenderBackend::Rich => render_block_rich(block, settings),
    }
}

fn render_block_plain(block: &Block, theme_name: &str) -> String {
    match block {
        Block::Json(json) => {
            serde_json::to_string_pretty(&json.payload).unwrap_or_else(|_| "[]".to_string())
        }
        Block::Value(values) => render_value_block(&values.values),
        Block::Mreg(mreg) => render_mreg_block(mreg, false, theme_name),
        Block::Table(table) => render_table_block(table, false, false, None, theme_name),
    }
}

fn render_block_rich(block: &Block, settings: &ResolvedRenderSettings) -> String {
    match block {
        Block::Json(json) => {
            serde_json::to_string_pretty(&json.payload).unwrap_or_else(|_| "[]".to_string())
        }
        Block::Value(values) => render_value_block(&values.values),
        Block::Mreg(mreg) => render_mreg_block(mreg, settings.color, &settings.theme_name),
        Block::Table(table) => render_table_block(
            table,
            settings.unicode,
            settings.color,
            settings.width,
            &settings.theme_name,
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

fn render_mreg_block(block: &crate::document::MregBlock, color: bool, theme_name: &str) -> String {
    if block.rows.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for (row_index, row) in block.rows.iter().enumerate() {
        for entry in &row.entries {
            let key_text = apply_style(&entry.key, StyleToken::MregKey, color, theme_name);

            match &entry.value {
                MregValue::Scalar(value) => {
                    out.push_str(&format!("{key_text}: {value}\n"));
                }
                MregValue::List(values) => {
                    out.push_str(&format!("{key_text} ({})\n", values.len()));
                    for value in values {
                        out.push_str(&format!("  {value}\n"));
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

fn render_table_block(
    block: &crate::document::TableBlock,
    unicode: bool,
    color: bool,
    width: Option<usize>,
    theme_name: &str,
) -> String {
    if block.rows.is_empty() {
        return String::new();
    }

    match block.style {
        TableStyle::Grid => render_grid_table(block, unicode, color, width, theme_name),
        TableStyle::Markdown => render_markdown_table(block, width, unicode),
    }
}

fn render_grid_table(
    block: &crate::document::TableBlock,
    unicode: bool,
    color: bool,
    width: Option<usize>,
    theme_name: &str,
) -> String {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    if unicode {
        table.load_preset(presets::UTF8_FULL);
    } else {
        table.load_preset(presets::ASCII_FULL);
    }

    let (headers, rows) = truncate_table_for_width(&block.headers, &block.rows, width, unicode);
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
            .map(|value| style_hex_cell(value, color, theme_name))
            .map(Cell::new)
            .collect::<Vec<Cell>>();
        table.add_row(styled_row);
    }

    format!("{table}\n")
}

fn style_hex_cell(value: &str, color: bool, theme_name: &str) -> String {
    let trimmed = value.trim();
    if !color || theme_name.eq_ignore_ascii_case("plain") || !is_hex_color(trimmed) {
        return value.to_string();
    }
    apply_style_spec(value, trimmed, true)
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

fn render_markdown_table(
    block: &crate::document::TableBlock,
    width: Option<usize>,
    unicode: bool,
) -> String {
    let (headers, rows) = truncate_table_for_width(&block.headers, &block.rows, width, unicode);
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

fn truncate_table_for_width(
    headers: &[String],
    rows: &[Vec<String>],
    width: Option<usize>,
    unicode: bool,
) -> (Vec<String>, Vec<Vec<String>>) {
    let Some(width_limit) = width else {
        return (headers.to_vec(), rows.to_vec());
    };

    if headers.is_empty() {
        return (Vec::new(), rows.to_vec());
    }

    let column_count = headers.len();
    let border_overhead = column_count * 3 + 1;
    if width_limit <= border_overhead {
        return (headers.to_vec(), rows.to_vec());
    }

    let available = width_limit - border_overhead;
    let min_column_width = if unicode { 4 } else { 6 };
    let max_per_column = (available / column_count).max(min_column_width);

    let truncate_cell = |cell: &str| truncate_display_width(cell, max_per_column, unicode);

    let truncated_headers = headers
        .iter()
        .map(|header| truncate_cell(header))
        .collect::<Vec<String>>();
    let truncated_rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| truncate_cell(cell))
                .collect::<Vec<String>>()
        })
        .collect::<Vec<Vec<String>>>();

    (truncated_headers, truncated_rows)
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
        assert!(rendered.contains("| uid | group |"));
        assert!(rendered.contains("| --- | --- |"));
        assert!(rendered.contains("| oistes | uio |"));
    }

    #[test]
    fn width_limit_truncates_wide_cells() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string(), "description".to_string()],
                rows: vec![vec![
                    "oistes".to_string(),
                    "this-is-a-very-long-description-value".to_string(),
                ]],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Rich,
                color: false,
                unicode: false,
                width: Some(32),
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            },
        );
        assert!(rendered.contains("..."));
    }

    #[test]
    fn theme_hex_values_render_with_truecolor_when_enabled() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["name".to_string(), "accent".to_string()],
                rows: vec![vec!["dracula".to_string(), "#ff79c6".to_string()]],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Rich,
                color: true,
                unicode: false,
                width: None,
                theme_name: "dracula".to_string(),
            },
        );
        assert!(rendered.contains("\x1b[38;2;255;121;198m#ff79c6\x1b[0m"));
    }

    #[test]
    fn plain_theme_does_not_style_hex_cells() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["name".to_string(), "accent".to_string()],
                rows: vec![vec!["plain".to_string(), "#ff79c6".to_string()]],
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
        assert!(!rendered.contains("\x1b[38;2;255;121;198m#ff79c6\x1b[0m"));
    }
}
