use comfy_table::{Cell, ContentArrangement, Table, presets};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::document::{
    Block, Document, MregBlock, MregValue, PanelRules, TableAlign, TableBlock, TableStyle,
};
use crate::layout::{LayoutContext, MregMetrics, block_identity, prepare_layout_context};
use crate::style::{
    StyleOverrides, StyleToken, apply_style_spec, apply_style_with_theme_overrides,
};
use crate::theme::ThemeDefinition;
use crate::{RenderBackend, ResolvedRenderSettings};

const DEFAULT_SHORT_LIST_MAX: usize = 1;
const DEFAULT_MEDIUM_LIST_MAX: usize = 5;
const DEFAULT_GRID_PADDING: usize = 4;
const DEFAULT_COLUMN_WEIGHT: usize = 3;

pub fn render_document(document: &Document, settings: ResolvedRenderSettings) -> String {
    let layout = prepare_layout_context(document, &settings);
    let mut out = String::new();
    for block in &document.blocks {
        out.push_str(&render_block(block, &settings, &layout));
    }
    out
}

fn render_block(
    block: &Block,
    settings: &ResolvedRenderSettings,
    layout: &LayoutContext,
) -> String {
    match settings.backend {
        RenderBackend::Plain => render_block_plain(block, settings, layout),
        RenderBackend::Rich => render_block_rich(block, settings, layout),
    }
}

fn render_block_plain(
    block: &Block,
    settings: &ResolvedRenderSettings,
    layout: &LayoutContext,
) -> String {
    match block {
        Block::Line(line) => render_line_block(
            line,
            false,
            &settings.theme,
            settings.margin,
            &settings.style_overrides,
        ),
        Block::Panel(panel) => render_panel_block(panel, settings),
        Block::Code(code) => render_code_block(
            code,
            settings.margin,
            false,
            &settings.theme,
            &settings.style_overrides,
        ),
        Block::Json(json) => indent_lines(
            &serde_json::to_string_pretty(&json.payload).unwrap_or_else(|_| "[]".to_string()),
            settings.margin,
        ),
        Block::Value(values) => render_value_block(&values.values, settings.margin),
        Block::Mreg(mreg) => render_mreg_block(
            mreg,
            false,
            false,
            settings.width,
            settings.margin,
            settings.indent_size,
            settings.short_list_max.max(DEFAULT_SHORT_LIST_MAX),
            settings.medium_list_max.max(DEFAULT_MEDIUM_LIST_MAX),
            settings.grid_padding.max(DEFAULT_GRID_PADDING),
            settings.grid_columns,
            settings.column_weight.max(DEFAULT_COLUMN_WEIGHT),
            &settings.theme,
            &settings.style_overrides,
            layout.mreg_metrics.get(&block_identity(block)).copied(),
        ),
        Block::Table(table) => render_table_block(
            table,
            false,
            false,
            settings.width,
            &settings.theme,
            settings.margin,
            settings.indent_size,
            false,
            layout
                .table_column_widths
                .get(&block_identity(block))
                .map(Vec::as_slice),
            &settings.style_overrides,
        ),
    }
}

fn render_block_rich(
    block: &Block,
    settings: &ResolvedRenderSettings,
    layout: &LayoutContext,
) -> String {
    match block {
        Block::Line(line) => render_line_block(
            line,
            settings.color,
            &settings.theme,
            settings.margin,
            &settings.style_overrides,
        ),
        Block::Panel(panel) => render_panel_block(panel, settings),
        Block::Code(code) => render_code_block(
            code,
            settings.margin,
            settings.color,
            &settings.theme,
            &settings.style_overrides,
        ),
        Block::Json(json) => render_json_block(
            &json.payload,
            settings.margin,
            settings.color,
            &settings.theme,
            &settings.style_overrides,
        ),
        Block::Value(values) => render_value_block(&values.values, settings.margin),
        Block::Mreg(mreg) => render_mreg_block(
            mreg,
            settings.color,
            settings.unicode,
            settings.width,
            settings.margin,
            settings.indent_size,
            settings.short_list_max.max(DEFAULT_SHORT_LIST_MAX),
            settings.medium_list_max.max(DEFAULT_MEDIUM_LIST_MAX),
            settings.grid_padding.max(DEFAULT_GRID_PADDING),
            settings.grid_columns,
            settings.column_weight.max(DEFAULT_COLUMN_WEIGHT),
            &settings.theme,
            &settings.style_overrides,
            layout.mreg_metrics.get(&block_identity(block)).copied(),
        ),
        Block::Table(table) => render_table_block(
            table,
            settings.unicode,
            settings.color,
            settings.width,
            &settings.theme,
            settings.margin,
            settings.indent_size,
            true,
            layout
                .table_column_widths
                .get(&block_identity(block))
                .map(Vec::as_slice),
            &settings.style_overrides,
        ),
    }
}

fn render_value_block(values: &[String], margin: usize) -> String {
    if values.is_empty() {
        String::new()
    } else {
        values
            .iter()
            .map(|value| format!("{}{}\n", " ".repeat(margin), value))
            .collect::<String>()
    }
}

fn render_line_block(
    block: &crate::document::LineBlock,
    color: bool,
    theme: &ThemeDefinition,
    margin: usize,
    style_overrides: &StyleOverrides,
) -> String {
    let mut out = String::new();
    out.push_str(&" ".repeat(margin));
    for part in &block.parts {
        if let Some(token) = part.token {
            out.push_str(&apply_style_with_theme_overrides(
                &part.text,
                token,
                color,
                theme,
                style_overrides,
            ));
        } else {
            out.push_str(&part.text);
        }
    }
    out.push('\n');
    out
}

fn render_code_block(
    block: &crate::document::CodeBlock,
    margin: usize,
    color: bool,
    theme: &ThemeDefinition,
    style_overrides: &StyleOverrides,
) -> String {
    let code = if block.code.ends_with('\n') {
        block.code.clone()
    } else {
        format!("{}\n", block.code)
    };
    let mut out = String::new();
    for line in code.trim_end_matches('\n').lines() {
        let line = apply_style_with_theme_overrides(
            line,
            StyleToken::Code,
            color,
            theme,
            style_overrides,
        );
        out.push_str(&" ".repeat(margin));
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn render_json_block(
    payload: &Value,
    margin: usize,
    color: bool,
    theme: &ThemeDefinition,
    style_overrides: &StyleOverrides,
) -> String {
    let rendered = render_json_value(payload, 0, color, theme, style_overrides);
    indent_lines(&rendered, margin)
}

fn render_json_value(
    value: &Value,
    depth: usize,
    color: bool,
    theme: &ThemeDefinition,
    style_overrides: &StyleOverrides,
) -> String {
    let indent = "  ".repeat(depth);
    let next_indent = "  ".repeat(depth + 1);

    match value {
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let mut out = String::new();
            out.push_str("{\n");
            for (index, (key, item)) in map.iter().enumerate() {
                let comma = if index + 1 < map.len() { "," } else { "" };
                let key_json = serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""));
                let key_text = apply_style_with_theme_overrides(
                    &key_json,
                    StyleToken::JsonKey,
                    color,
                    theme,
                    style_overrides,
                );
                let value_text =
                    render_json_value(item, depth + 1, color, theme, style_overrides);
                out.push_str(&next_indent);
                out.push_str(&key_text);
                out.push_str(": ");
                out.push_str(&value_text);
                out.push_str(comma);
                out.push('\n');
            }
            out.push_str(&indent);
            out.push('}');
            out
        }
        Value::Array(values) => {
            if values.is_empty() {
                return "[]".to_string();
            }
            let mut out = String::new();
            out.push_str("[\n");
            for (index, item) in values.iter().enumerate() {
                let comma = if index + 1 < values.len() { "," } else { "" };
                out.push_str(&next_indent);
                out.push_str(&render_json_value(item, depth + 1, color, theme, style_overrides));
                out.push_str(comma);
                out.push('\n');
            }
            out.push_str(&indent);
            out.push(']');
            out
        }
        Value::String(raw) => {
            let quoted = serde_json::to_string(raw).unwrap_or_else(|_| format!("\"{raw}\""));
            apply_style_with_theme_overrides(
                &quoted,
                StyleToken::Value,
                color,
                theme,
                style_overrides,
            )
        }
        Value::Number(number) => apply_style_with_theme_overrides(
            &number.to_string(),
            StyleToken::Number,
            color,
            theme,
            style_overrides,
        ),
        Value::Bool(boolean) => apply_style_with_theme_overrides(
            if *boolean { "true" } else { "false" },
            if *boolean {
                StyleToken::BoolTrue
            } else {
                StyleToken::BoolFalse
            },
            color,
            theme,
            style_overrides,
        ),
        Value::Null => {
            apply_style_with_theme_overrides("null", StyleToken::Null, color, theme, style_overrides)
        }
    }
}

fn render_panel_block(
    block: &crate::document::PanelBlock,
    settings: &ResolvedRenderSettings,
) -> String {
    let fallback = section_style_token(block.kind.as_deref()).unwrap_or(StyleToken::PanelBorder);
    let token = block.border_token.unwrap_or(fallback);
    let color = settings.color;
    let unicode = settings.unicode;
    let width = settings.width;
    let theme = &settings.theme;
    let title_token = block.title_token.unwrap_or(StyleToken::PanelTitle);
    let divider = section_divider(
        block.title.as_deref().unwrap_or(""),
        token,
        title_token,
        unicode,
        width,
        color,
        theme,
        &settings.style_overrides,
    );
    let mut child_settings = settings.clone();
    child_settings.margin = settings.margin + settings.indent_size;
    let inner = render_document(&block.body, child_settings);
    match block.rules {
        PanelRules::None => inner,
        PanelRules::Top => format!("{}{}\n{}", " ".repeat(settings.margin), divider, inner),
        PanelRules::Bottom => format!("{inner}{}{}\n", " ".repeat(settings.margin), divider),
        PanelRules::Both => format!(
            "{}{}\n{inner}{}{}\n",
            " ".repeat(settings.margin),
            divider,
            " ".repeat(settings.margin),
            divider
        ),
    }
}

fn render_mreg_block(
    block: &MregBlock,
    color: bool,
    unicode: bool,
    width: Option<usize>,
    margin: usize,
    indent_size: usize,
    short_list_max: usize,
    medium_list_max: usize,
    grid_padding: usize,
    grid_columns: Option<usize>,
    column_weight: usize,
    theme: &ThemeDefinition,
    style_overrides: &StyleOverrides,
    metrics: Option<MregMetrics>,
) -> String {
    if block.rows.is_empty() {
        return String::new();
    }

    let key_width = metrics
        .map(|value| value.key_width)
        .unwrap_or_else(|| compute_mreg_key_width(block, indent_size).max(3));
    let value_column = key_width + 2;
    let mut out = String::new();
    for (row_index, row) in block.rows.iter().enumerate() {
        for entry in &row.entries {
            let key_indent = " ".repeat(entry.depth * indent_size);
            let margin_indent = " ".repeat(margin);
            let key_text = apply_style_with_theme_overrides(
                &entry.key,
                StyleToken::MregKey,
                color,
                theme,
                style_overrides,
            );
            let key_len = display_width(&entry.key);
            let base_key_len = mreg_alignment_key_width(&entry.key);
            let base_len = entry.depth * indent_size + base_key_len + 1;
            let full_len = entry.depth * indent_size + key_len + 1;
            let scalar_gap_width = value_column.saturating_sub(base_len).max(1);
            let scalar_gap = " ".repeat(scalar_gap_width);

            match &entry.value {
                MregValue::Group => {
                    out.push_str(&format!("{margin_indent}{key_indent}{key_text}:\n"));
                }
                MregValue::Separator => {
                    out.push_str(&format!("{margin_indent}{key_indent}---\n"));
                }
                MregValue::Scalar(value) => {
                    if value.is_null() {
                        out.push_str(&format!("{margin_indent}{key_indent}{key_text}:\n"));
                        continue;
                    }
                    let raw = value_to_string(value);
                    let value_text = style_value_cell(value, &raw, color, theme, style_overrides);
                    out.push_str(&format!(
                        "{margin_indent}{key_indent}{key_text}:{scalar_gap}{value_text}\n"
                    ));
                }
                MregValue::List(values) => {
                    let list_available_width = width
                        .unwrap_or(100)
                        .saturating_sub(margin + (entry.depth + 1) * indent_size)
                        .max(16);
                    let rendered = render_mreg_list(
                        values,
                        list_available_width,
                        unicode,
                        short_list_max,
                        medium_list_max,
                        grid_padding,
                        grid_columns,
                        column_weight,
                    );
                    match rendered {
                        MregListRender::Inline(line) => {
                            out.push_str(&format!(
                                "{margin_indent}{key_indent}{key_text}:{scalar_gap}{line}\n"
                            ));
                        }
                        MregListRender::Vertical(lines) => {
                            if let Some((first, rest)) = lines.split_first() {
                                let first_value_col = value_column.max(full_len + 1);
                                let first_gap_width = first_value_col.saturating_sub(full_len);
                                out.push_str(&format!(
                                    "{margin_indent}{key_indent}{key_text}:{}{first}\n",
                                    " ".repeat(first_gap_width)
                                ));
                                let hanging = " ".repeat(margin + first_value_col);
                                for line in rest {
                                    out.push_str(&format!("{hanging}{line}\n"));
                                }
                            } else {
                                out.push_str(&format!("{margin_indent}{key_indent}{key_text}:\n"));
                            }
                        }
                        MregListRender::Grid(lines) => {
                            out.push_str(&format!("{margin_indent}{key_indent}{key_text}:\n"));
                            for line in lines {
                                out.push_str(&" ".repeat(margin + (entry.depth + 1) * indent_size));
                                let joined = line.join(&" ".repeat(grid_padding));
                                out.push_str(joined.trim_end());
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
    border_token: StyleToken,
    title_token: StyleToken,
    unicode: bool,
    width: Option<usize>,
    color: bool,
    theme: &ThemeDefinition,
    style_overrides: &StyleOverrides,
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

    if title.is_empty() {
        let raw = fill_char.to_string().repeat(target_width);
        if color {
            return apply_style_with_theme_overrides(
                &raw,
                StyleToken::PanelBorder,
                true,
                theme,
                style_overrides,
            );
        }
        return raw;
    }

    let left = if unicode { "─ " } else { "- " };
    let prefix_width = left.chars().count() + title.chars().count() + 1;
    let suffix = if prefix_width >= target_width {
        String::new()
    } else {
        fill_char.to_string().repeat(target_width - prefix_width)
    };
    let raw = format!("{left}{title} {suffix}");

    if !color {
        return raw;
    }

    if title_token == border_token {
        return apply_style_with_theme_overrides(&raw, border_token, true, theme, style_overrides);
    }

    let border = apply_style_with_theme_overrides(left, border_token, true, theme, style_overrides);
    let title = apply_style_with_theme_overrides(title, title_token, true, theme, style_overrides);
    let trailing = apply_style_with_theme_overrides(
        &format!(" {suffix}"),
        border_token,
        true,
        theme,
        style_overrides,
    );
    format!("{border}{title}{trailing}")
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

fn render_mreg_list(
    values: &[Value],
    available_width: usize,
    unicode: bool,
    short_list_max: usize,
    medium_list_max: usize,
    grid_padding: usize,
    grid_columns: Option<usize>,
    column_weight: usize,
) -> MregListRender {
    if values.is_empty() {
        return MregListRender::Inline(String::new());
    }
    let mut values = values.iter().map(value_to_string).collect::<Vec<String>>();

    let inline = values.join(", ");
    if values.len() <= short_list_max && display_width(&inline) <= available_width {
        return MregListRender::Inline(inline);
    }

    if values.len() <= medium_list_max {
        return MregListRender::Vertical(values.to_vec());
    }

    values.sort_by_key(|value| value.to_ascii_lowercase());

    let columns = choose_grid_columns(
        &values,
        available_width.max(1),
        grid_padding.max(1),
        grid_columns,
        column_weight.max(1),
    );

    if columns < 2 {
        return MregListRender::Vertical(values.to_vec());
    }

    let rows_count = values.len().div_ceil(columns);
    let mut matrix = vec![vec![String::new(); columns]; rows_count];
    let mut column_widths = vec![0usize; columns];

    // Fill by columns first, like the Python renderer, for stable balancing.
    for (index, value) in values.iter().enumerate() {
        let row_index = index % rows_count;
        let column_index = index / rows_count;
        if column_index >= columns {
            continue;
        }
        let truncated = truncate_display_width(value, available_width.max(4), unicode);
        column_widths[column_index] = column_widths[column_index].max(display_width(&truncated));
        matrix[row_index][column_index] = truncated;
    }

    let rows = matrix
        .into_iter()
        .map(|row| {
            row.into_iter()
                .enumerate()
                .map(|(index, value)| {
                    if index == columns - 1 {
                        value
                    } else {
                        pad_display_width(&value, column_widths[index])
                    }
                })
                .collect::<Vec<String>>()
        })
        .filter(|row| row.iter().any(|cell| !cell.trim_end().is_empty()))
        .collect::<Vec<Vec<String>>>();

    MregListRender::Grid(rows)
}

fn render_table_block(
    block: &TableBlock,
    unicode: bool,
    color: bool,
    width: Option<usize>,
    theme: &ThemeDefinition,
    margin: usize,
    indent_size: usize,
    respect_depth: bool,
    column_widths: Option<&[usize]>,
    style_overrides: &StyleOverrides,
) -> String {
    if block.rows.is_empty() && block.header_pairs.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let header_pairs = render_table_header_pairs(
        block,
        color,
        theme,
        margin,
        indent_size,
        respect_depth,
        style_overrides,
    );
    if !header_pairs.is_empty() {
        out.push_str(&header_pairs);
    }

    let effective_margin = margin
        + if respect_depth {
            block.depth * indent_size
        } else {
            0
        };

    let table_body = match block.style {
        TableStyle::Grid => render_grid_table(
            block,
            unicode,
            color,
            width,
            effective_margin,
            indent_size,
            column_widths,
            style_overrides,
        ),
        TableStyle::Markdown => {
            render_markdown_table(block, unicode, effective_margin, indent_size, column_widths)
        }
    };

    out.push_str(&table_body);
    out
}

fn render_table_header_pairs(
    block: &TableBlock,
    color: bool,
    theme: &ThemeDefinition,
    margin: usize,
    indent_size: usize,
    respect_depth: bool,
    style_overrides: &StyleOverrides,
) -> String {
    if block.header_pairs.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    let prefix = " ".repeat(
        margin
            + if respect_depth {
                block.depth * indent_size
            } else {
                0
            },
    );
    for (key, value) in &block.header_pairs {
        let key_text = apply_style_with_theme_overrides(
            key,
            StyleToken::MregKey,
            color,
            theme,
            style_overrides,
        );
        let raw = value_to_string(value);
        let value_text = style_value_cell(value, &raw, color, theme, style_overrides);
        out.push_str(&prefix);
        out.push_str(&key_text);
        out.push_str(": ");
        out.push_str(&value_text);
        out.push('\n');
    }
    out.push('\n');
    out
}

fn render_grid_table(
    block: &TableBlock,
    unicode: bool,
    _color: bool,
    _width: Option<usize>,
    margin: usize,
    _indent_size: usize,
    column_widths: Option<&[usize]>,
    _style_overrides: &StyleOverrides,
) -> String {
    if block.rows.is_empty() {
        return String::new();
    }
    let (headers, rows, _) =
        truncate_table_to_widths(&block.headers, &block.rows, column_widths, unicode);

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Disabled);
    if unicode {
        table.load_preset(presets::UTF8_FULL_CONDENSED);
    } else {
        table.load_preset(presets::ASCII_FULL_CONDENSED);
    }

    let header_cells = headers
        .iter()
        .map(|header| Cell::new(header))
        .collect::<Vec<Cell>>();
    table.set_header(header_cells);

    for row in &rows {
        let styled_row = row
            .iter()
            .map(|(_, text)| text.clone())
            .map(Cell::new)
            .collect::<Vec<Cell>>();
        table.add_row(styled_row);
    }

    let rendered = format!("{table}");
    let rendered = if unicode {
        normalize_rich_table_chrome(&rendered)
    } else {
        normalize_ascii_table_chrome(&rendered)
    };
    indent_lines(&rendered, margin)
}

fn normalize_rich_table_chrome(rendered: &str) -> String {
    let mut out = String::new();
    let lines = rendered.lines().collect::<Vec<&str>>();
    for (index, line) in lines.iter().enumerate() {
        let mut normalized = line.replace('┆', "│");
        if normalized.starts_with('┌') {
            normalized = normalized
                .replace('┌', "┏")
                .replace('┬', "┳")
                .replace('┐', "┓")
                .replace('─', "━");
        } else if index == 1 && normalized.starts_with('│') {
            normalized = normalized.replace('│', "┃");
        } else if normalized.starts_with('╞') {
            normalized = normalized
                .replace('╞', "┡")
                .replace('╪', "╇")
                .replace('╡', "┩")
                .replace('═', "━");
        }
        out.push_str(&normalized);
        if index + 1 < lines.len() || rendered.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn normalize_ascii_table_chrome(rendered: &str) -> String {
    let mut out = String::new();
    let lines = rendered.lines().collect::<Vec<&str>>();
    for (index, line) in lines.iter().enumerate() {
        let normalized = if line.starts_with('+')
            && line.ends_with('+')
            && line.bytes().all(|byte| byte == b'+' || byte == b'=')
        {
            if let Some(previous) = lines[..index]
                .iter()
                .rev()
                .find(|candidate| candidate.starts_with('+') && candidate.contains('-'))
            {
                previous.replace('=', "-")
            } else {
                line.replace('=', "-")
            }
        } else {
            (*line).to_string()
        };
        out.push_str(&normalized);
        if index + 1 < lines.len() || rendered.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn render_markdown_table(
    block: &TableBlock,
    unicode: bool,
    margin: usize,
    _indent_size: usize,
    column_widths: Option<&[usize]>,
) -> String {
    let (headers, rows, _widths) =
        truncate_table_to_widths(&block.headers, &block.rows, column_widths, unicode);
    if headers.is_empty() {
        return String::new();
    }

    let aligns = resolve_table_alignments(headers.len(), block.align.as_deref());
    let widths = rows.iter().fold(
        headers
            .iter()
            .map(|header| display_width(header).max(3))
            .collect::<Vec<usize>>(),
        |mut acc, row| {
            for (index, (_, cell)) in row.iter().enumerate() {
                if let Some(width) = acc.get_mut(index) {
                    *width = (*width).max(display_width(cell).max(3));
                }
            }
            acc
        },
    );

    let mut out = String::new();
    out.push('|');
    for (index, header) in headers.iter().enumerate() {
        out.push(' ');
        out.push_str(&pad_markdown_cell(
            &escape_markdown_cell(header),
            widths[index],
            aligns[index],
        ));
        out.push(' ');
        out.push('|');
    }
    out.push('\n');

    out.push('|');
    for (index, width) in widths.iter().enumerate() {
        out.push(' ');
        out.push_str(&markdown_separator(*width, aligns[index]));
        out.push(' ');
        out.push('|');
    }
    out.push('\n');

    for row in &rows {
        out.push('|');
        for (index, (_value, cell)) in row.iter().enumerate() {
            out.push(' ');
            out.push_str(&pad_markdown_cell(
                &escape_markdown_cell(cell),
                widths[index],
                aligns[index],
            ));
            out.push(' ');
            out.push('|');
        }
        out.push('\n');
    }

    indent_lines(&out, margin)
}

fn truncate_table_to_widths(
    headers: &[String],
    rows: &[Vec<Value>],
    column_widths: Option<&[usize]>,
    unicode: bool,
) -> (Vec<String>, Vec<Vec<(Value, String)>>, Vec<usize>) {
    let fallback_widths = compute_fallback_widths(headers, rows, if unicode { 4 } else { 6 });
    let widths = match column_widths {
        Some(widths) if widths.len() == headers.len() => widths.to_vec(),
        _ => fallback_widths,
    };

    let headers = headers
        .iter()
        .zip(&widths)
        .map(|(header, width)| truncate_display_width_crop(header, *width))
        .collect::<Vec<String>>();

    let rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(index, cell)| {
                    let width = widths.get(index).copied().unwrap_or(8);
                    let raw = value_to_string(cell);
                    (cell.clone(), truncate_display_width_crop(&raw, width))
                })
                .collect::<Vec<(Value, String)>>()
        })
        .collect::<Vec<Vec<(Value, String)>>>();

    (headers, rows, widths)
}

fn compute_fallback_widths(
    headers: &[String],
    rows: &[Vec<Value>],
    min_column_width: usize,
) -> Vec<usize> {
    let mut widths = headers
        .iter()
        .map(|header| display_width(header).max(min_column_width))
        .collect::<Vec<usize>>();

    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                let raw = value_to_string(cell);
                *width = (*width).max(display_width(&raw).max(min_column_width));
            }
        }
    }

    widths
}

fn style_value_cell(
    value: &Value,
    text: &str,
    color: bool,
    theme: &ThemeDefinition,
    style_overrides: &StyleOverrides,
) -> String {
    let trimmed = text.trim();
    if !color || theme.id.eq_ignore_ascii_case("plain") {
        return text.to_string();
    }

    if is_hex_color(trimmed) {
        return apply_style_spec(text, trimmed, true);
    }

    match value {
        Value::Bool(true) => apply_style_with_theme_overrides(
            text,
            StyleToken::BoolTrue,
            true,
            theme,
            style_overrides,
        ),
        Value::Bool(false) => apply_style_with_theme_overrides(
            text,
            StyleToken::BoolFalse,
            true,
            theme,
            style_overrides,
        ),
        Value::Null => {
            apply_style_with_theme_overrides(text, StyleToken::Null, true, theme, style_overrides)
        }
        Value::Number(_) => {
            apply_style_with_theme_overrides(text, StyleToken::Number, true, theme, style_overrides)
        }
        Value::String(raw) => {
            if let Some(token) = value_style_token_for_string(raw) {
                return apply_style_with_theme_overrides(text, token, true, theme, style_overrides);
            }
            text.to_string()
        }
        _ => text.to_string(),
    }
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

fn compute_mreg_key_width(block: &MregBlock, indent_size: usize) -> usize {
    block
        .rows
        .iter()
        .flat_map(|row| row.entries.iter())
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

fn truncate_display_width_crop(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }

    let mut out = String::new();
    let mut width = 0usize;
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

fn escape_markdown_cell(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|")
}

fn indent_lines(value: &str, margin: usize) -> String {
    let prefix = " ".repeat(margin);
    if value.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for line in value.lines() {
        out.push_str(&prefix);
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string().to_ascii_lowercase(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(values) => values
            .iter()
            .map(value_to_string)
            .collect::<Vec<String>>()
            .join(", "),
        Value::Object(_) => value.to_string(),
    }
}

fn value_style_token_for_string(value: &str) -> Option<StyleToken> {
    if is_hex_color(value.trim()) {
        return None;
    }
    if value.parse::<std::net::Ipv4Addr>().is_ok() {
        return Some(StyleToken::Ipv4);
    }
    if value.parse::<std::net::Ipv6Addr>().is_ok() {
        return Some(StyleToken::Ipv6);
    }
    None
}

fn choose_grid_columns(
    values: &[String],
    available_width: usize,
    grid_padding: usize,
    grid_columns: Option<usize>,
    column_weight: usize,
) -> usize {
    let n = values.len();
    if n <= 1 {
        return 1;
    }

    if let Some(forced) = grid_columns {
        return forced.max(1).min(n);
    }

    let mut best_cols = 1usize;
    let mut best_score = usize::MAX;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let mut col_widths = vec![0usize; cols];

        for (idx, value) in values.iter().enumerate() {
            let col = idx / rows;
            if col >= cols {
                continue;
            }
            col_widths[col] = col_widths[col].max(display_width(value));
        }

        let total_width = col_widths.iter().sum::<usize>() + grid_padding * cols.saturating_sub(1);
        if total_width > available_width {
            break;
        }

        let score = rows.abs_diff(cols * column_weight);
        if score <= best_score {
            best_score = score;
            best_cols = cols;
        }
    }
    best_cols.max(1).min(n)
}

fn resolve_table_alignments(width: usize, align: Option<&[TableAlign]>) -> Vec<TableAlign> {
    let mut out = align
        .map(|value| value.to_vec())
        .unwrap_or_else(|| vec![TableAlign::Default; width]);
    if out.len() < width {
        out.extend(std::iter::repeat(TableAlign::Default).take(width - out.len()));
    }
    out.truncate(width);
    out
}

fn markdown_separator(width: usize, align: TableAlign) -> String {
    let width = width.max(3);
    match align {
        TableAlign::Default => "-".repeat(width),
        TableAlign::Left => format!(":{}", "-".repeat(width.saturating_sub(1))),
        TableAlign::Right => format!("{}:", "-".repeat(width.saturating_sub(1))),
        TableAlign::Center => {
            if width <= 2 {
                ":".repeat(width)
            } else {
                format!(":{}:", "-".repeat(width - 2))
            }
        }
    }
}

fn pad_markdown_cell(value: &str, width: usize, align: TableAlign) -> String {
    match align {
        TableAlign::Right => format!("{value:>width$}"),
        TableAlign::Center => {
            let text_width = display_width(value);
            if text_width >= width {
                value.to_string()
            } else {
                let total = width - text_width;
                let left = total / 2;
                let right = total - left;
                format!("{}{}{}", " ".repeat(left), value, " ".repeat(right))
            }
        }
        _ => format!("{value:<width$}"),
    }
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
    use serde_json::{Value, json};

    fn settings(backend: RenderBackend, color: bool, unicode: bool) -> ResolvedRenderSettings {
        ResolvedRenderSettings {
            backend,
            color,
            unicode,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::style::StyleOverrides::default(),
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
                    depth: 0,
                    value: MregValue::Scalar(json!("oistes")),
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
                        depth: 0,
                        value: MregValue::List(vec![json!("alice"), json!("bob"), json!("carol")]),
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
                width: None,
                margin: 0,
                indent_size: 2,
                short_list_max: 3,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );
        assert!(rendered.contains("members: alice, bob, carol"));
    }

    #[test]
    fn mreg_large_lists_use_grid_layout() {
        let values = (1..=12)
            .map(|index| json!(format!("item-{index}")))
            .collect::<Vec<Value>>();
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "members".to_string(),
                        depth: 0,
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
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );

        assert!(rendered.contains("members:"));
        assert!(
            rendered
                .lines()
                .any(|line| line.matches("item-").count() >= 2)
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
    fn rich_json_block_uses_color_tokens() {
        let document = Document {
            blocks: vec![Block::Json(JsonBlock {
                payload: json!({"uid":"oistes","enabled":true,"count":2}),
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Rich, true, true));
        assert!(rendered.contains("\x1b["));
        assert!(rendered.contains("\"uid\""));
        assert!(rendered.contains("true"));
    }

    #[test]
    fn render_table_toggles_border_style() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!("oistes")]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let unicode = render_document(&document, settings(RenderBackend::Rich, false, true));
        let ascii = render_document(&document, settings(RenderBackend::Plain, false, false));

        assert!(unicode.contains('┏'));
        assert!(unicode.contains('│'));
        assert!(ascii.contains('+'));
    }

    #[test]
    fn table_renders_header_pairs_before_table() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!("oistes")]],
                header_pairs: vec![("group".to_string(), json!("ops"))],
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Plain, false, false));
        assert!(rendered.starts_with("group: ops\n\n"));
        assert!(rendered.contains("| uid"));
    }

    #[test]
    fn table_color_never_has_no_ansi_escape_codes() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!("oistes")]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
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
                rows: vec![vec![json!("oistes")]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Rich, false, false));
        for ch in ['┌', '┐', '└', '┘', '│', '─', '┬', '┴', '┼'] {
            assert!(!rendered.contains(ch));
        }
    }

    #[test]
    fn plain_ascii_table_uses_dash_header_separator() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!("oistes")]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Plain, false, false));
        assert!(
            rendered
                .lines()
                .any(|line| line.starts_with("+-") && line.contains('-'))
        );
        assert!(!rendered.lines().any(|line| line.starts_with("+=")));
    }

    #[test]
    fn mreg_alignment_accounts_for_nested_depth() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                rows: vec![MregRow {
                    entries: vec![
                        MregEntry {
                            key: "parent".to_string(),
                            depth: 0,
                            value: MregValue::Scalar(json!("root")),
                        },
                        MregEntry {
                            key: "nested".to_string(),
                            depth: 1,
                            value: MregValue::Scalar(json!("leaf")),
                        },
                    ],
                }],
            })],
        };

        let rendered = render_document(&document, settings(RenderBackend::Plain, false, false));
        let mut lines = rendered.lines();
        let top = lines.next().unwrap_or_default();
        let nested = lines.next().unwrap_or_default();
        assert!(top.starts_with("parent:  "));
        assert!(nested.starts_with("  nested: "));
    }

    #[test]
    fn markdown_table_render_has_pipe_format() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                style: TableStyle::Markdown,
                headers: vec!["uid".to_string(), "group".to_string()],
                rows: vec![vec![json!("oistes"), json!("uio")]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
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
                    json!("oistes"),
                    json!("this-is-a-very-long-cell-that-should-truncate"),
                ]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Plain,
                color: false,
                unicode: false,
                width: Some(40),
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );

        assert!(!rendered.contains("..."));
        assert!(!rendered.contains("this-is-a-very-long-cell-that-should-truncate"));
    }

    #[test]
    fn plain_theme_does_not_style_hex_cells() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "color".to_string(),
                        depth: 0,
                        value: MregValue::Scalar(json!("#ff00ff")),
                    }],
                }],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Rich,
                color: true,
                unicode: false,
                width: None,
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                theme_name: "plain".to_string(),
                theme: crate::theme::resolve_theme("plain"),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );
        assert!(!rendered.contains("\x1b["));
    }

    #[test]
    fn theme_hex_values_render_with_truecolor_when_enabled() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "color".to_string(),
                        depth: 0,
                        value: MregValue::Scalar(json!("#ff00ff")),
                    }],
                }],
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Rich,
                color: true,
                unicode: true,
                width: None,
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );

        assert!(rendered.contains("\x1b[38;2;255;0;255m"));
    }

    #[test]
    fn code_block_honors_color_code_override() {
        let document = Document {
            blocks: vec![Block::Code(crate::document::CodeBlock {
                code: "let x = 1;".to_string(),
                language: Some("rust".to_string()),
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Rich,
                color: true,
                unicode: true,
                width: None,
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides {
                    code: Some("#00ff00".to_string()),
                    ..Default::default()
                },
            },
        );

        assert!(rendered.contains("\x1b[38;2;0;255;0m"));
        assert!(rendered.contains("let x = 1;"));
    }
}
