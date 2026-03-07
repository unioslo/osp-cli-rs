use comfy_table::{
    Cell, CellAlignment, ColumnConstraint, ContentArrangement, Table, Width, presets,
};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::display::value_to_display;
use crate::document::{
    Block, Document, MregBlock, MregValue, PanelRules, TableAlign, TableBlock, TableStyle,
};
use crate::layout::{LayoutContext, MregEntryMetrics, MregMetrics, prepare_layout_context};
use crate::style::{StyleToken, apply_style_spec, apply_style_with_theme_overrides};
use crate::{RenderBackend, ResolvedRenderSettings, TableOverflow};

pub fn render_document(document: &Document, settings: ResolvedRenderSettings) -> String {
    DocumentRenderer::new(document, &settings).render(document)
}

struct DocumentRenderer<'a> {
    settings: &'a ResolvedRenderSettings,
    layout: LayoutContext,
}

impl<'a> DocumentRenderer<'a> {
    fn new(document: &Document, settings: &'a ResolvedRenderSettings) -> Self {
        Self {
            settings,
            layout: prepare_layout_context(document, settings),
        }
    }

    fn render(&self, document: &Document) -> String {
        let mut out = String::new();
        for block in &document.blocks {
            out.push_str(&self.render_block(block));
        }
        out
    }

    fn render_block(&self, block: &Block) -> String {
        match block {
            Block::Line(line) => self.render_line_block(line),
            Block::Panel(panel) => self.render_panel_block(panel),
            Block::Code(code) => self.render_code_block(code),
            Block::Json(json) => {
                if matches!(self.settings.backend, RenderBackend::Plain) {
                    indent_lines(
                        &serde_json::to_string_pretty(&json.payload)
                            .unwrap_or_else(|_| "[]".to_string()),
                        self.settings.margin,
                    )
                } else {
                    self.render_json_block(&json.payload)
                }
            }
            Block::Value(values) => self.render_value_block(&values.values),
            Block::Mreg(mreg) => {
                self.render_mreg_block(mreg, self.layout.mreg_metrics.get(&mreg.block_id).cloned())
            }
            Block::Table(table) => self.render_table_block(
                table,
                self.layout
                    .table_column_widths
                    .get(&table.block_id)
                    .map(Vec::as_slice),
                matches!(self.settings.backend, RenderBackend::Rich),
            ),
        }
    }

    fn render_value_block(&self, values: &[String]) -> String {
        if values.is_empty() {
            return String::new();
        }

        values
            .iter()
            .map(|value| format!("{}{}\n", " ".repeat(self.settings.margin), value))
            .collect()
    }

    fn render_line_block(&self, block: &crate::document::LineBlock) -> String {
        let mut out = String::new();
        out.push_str(&" ".repeat(self.settings.margin));
        for part in &block.parts {
            if let Some(token) = part.token {
                out.push_str(&apply_style_with_theme_overrides(
                    &part.text,
                    token,
                    self.settings.color,
                    &self.settings.theme,
                    &self.settings.style_overrides,
                ));
            } else {
                out.push_str(&part.text);
            }
        }
        out.push('\n');
        out
    }

    fn render_code_block(&self, block: &crate::document::CodeBlock) -> String {
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
                self.settings.color,
                &self.settings.theme,
                &self.settings.style_overrides,
            );
            out.push_str(&" ".repeat(self.settings.margin));
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    fn render_json_block(&self, payload: &Value) -> String {
        let rendered = self.render_json_value(payload, 0);
        indent_lines(&rendered, self.settings.margin)
    }

    fn render_json_value(&self, value: &Value, depth: usize) -> String {
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
                    let key_json =
                        serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""));
                    let key_text = apply_style_with_theme_overrides(
                        &key_json,
                        StyleToken::JsonKey,
                        self.settings.color,
                        &self.settings.theme,
                        &self.settings.style_overrides,
                    );
                    let value_text = self.render_json_value(item, depth + 1);
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
                    out.push_str(&self.render_json_value(item, depth + 1));
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
                    self.settings.color,
                    &self.settings.theme,
                    &self.settings.style_overrides,
                )
            }
            Value::Number(number) => apply_style_with_theme_overrides(
                &number.to_string(),
                StyleToken::Number,
                self.settings.color,
                &self.settings.theme,
                &self.settings.style_overrides,
            ),
            Value::Bool(boolean) => apply_style_with_theme_overrides(
                if *boolean { "true" } else { "false" },
                if *boolean {
                    StyleToken::BoolTrue
                } else {
                    StyleToken::BoolFalse
                },
                self.settings.color,
                &self.settings.theme,
                &self.settings.style_overrides,
            ),
            Value::Null => apply_style_with_theme_overrides(
                "null",
                StyleToken::Null,
                self.settings.color,
                &self.settings.theme,
                &self.settings.style_overrides,
            ),
        }
    }

    fn render_panel_block(&self, block: &crate::document::PanelBlock) -> String {
        let fallback =
            section_style_token(block.kind.as_deref()).unwrap_or(StyleToken::PanelBorder);
        let border_token = block.border_token.unwrap_or(fallback);
        let title_token = block.title_token.unwrap_or(StyleToken::PanelTitle);
        let titled_divider = self.section_divider(
            block.title.as_deref().unwrap_or(""),
            border_token,
            title_token,
        );
        let trailing_divider = self.section_divider("", border_token, title_token);
        let inner = DocumentRenderer::new(&block.body, self.settings).render(&block.body);

        match block.rules {
            PanelRules::None => inner,
            PanelRules::Top => {
                format!(
                    "{}{}\n{}",
                    " ".repeat(self.settings.margin),
                    titled_divider,
                    inner
                )
            }
            PanelRules::Bottom => {
                format!(
                    "{inner}{}{}\n",
                    " ".repeat(self.settings.margin),
                    trailing_divider
                )
            }
            PanelRules::Both => format!(
                "{}{}\n{inner}{}{}\n",
                " ".repeat(self.settings.margin),
                titled_divider,
                " ".repeat(self.settings.margin),
                trailing_divider
            ),
        }
    }

    fn render_mreg_block(&self, block: &MregBlock, metrics: Option<MregMetrics>) -> String {
        if block.rows.is_empty() {
            return String::new();
        }

        let metrics =
            metrics.unwrap_or_else(|| fallback_mreg_metrics(block, self.settings.indent_size));
        let mut out = String::new();
        let mut metric_index = 0usize;

        for (row_index, row) in block.rows.iter().enumerate() {
            for entry in &row.entries {
                let key_indent = " ".repeat(entry.depth * self.settings.indent_size);
                let margin_indent = " ".repeat(self.settings.margin);
                let key_text = self.style_token(&entry.key, StyleToken::MregKey);
                let entry_metrics = metrics
                    .entry_metrics
                    .get(metric_index)
                    .and_then(|value| *value);
                metric_index += 1;

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
                        let scalar_gap =
                            " ".repeat(entry_metrics.map(|value| value.gap).unwrap_or(1));
                        let raw = value_to_display(value);
                        let value_text = self.style_value_cell(value, &raw);
                        out.push_str(&format!(
                            "{margin_indent}{key_indent}{key_text}:{scalar_gap}{value_text}\n"
                        ));
                    }
                    MregValue::VerticalList(values) => {
                        out.push_str(&self.render_mreg_vertical_list(
                            &margin_indent,
                            &key_indent,
                            &key_text,
                            values,
                            entry_metrics,
                        ));
                    }
                    MregValue::Grid(values) => {
                        out.push_str(&format!("{margin_indent}{key_indent}{key_text}:\n"));
                        for line in self.render_grid_lines(values, entry.depth) {
                            out.push_str(&" ".repeat(
                                self.settings.margin
                                    + (entry.depth + 1) * self.settings.indent_size,
                            ));
                            out.push_str(line.trim_end());
                            out.push('\n');
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
        &self,
        block: &TableBlock,
        column_widths: Option<&[usize]>,
        respect_depth: bool,
    ) -> String {
        if block.rows.is_empty() && block.header_pairs.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        let header_pairs = self.render_table_header_pairs(block, respect_depth);
        if !header_pairs.is_empty() {
            out.push_str(&header_pairs);
        }

        let effective_margin = self.settings.margin
            + if respect_depth {
                block.depth * self.settings.indent_size
            } else {
                0
            };

        let table_body = match block.style {
            TableStyle::Grid => self.render_grid_table(block, effective_margin, column_widths),
            TableStyle::Markdown => {
                self.render_markdown_table(block, effective_margin, column_widths)
            }
        };

        out.push_str(&table_body);
        out
    }

    fn render_table_header_pairs(&self, block: &TableBlock, respect_depth: bool) -> String {
        if block.header_pairs.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        let prefix = " ".repeat(
            self.settings.margin
                + if respect_depth {
                    block.depth * self.settings.indent_size
                } else {
                    0
                },
        );
        let sep = if self.settings.unicode {
            "  ·  "
        } else {
            "  |  "
        };
        let sep = if self.settings.color {
            self.style_token(sep, StyleToken::Muted)
        } else {
            sep.to_string()
        };

        out.push_str(&prefix);
        for (idx, (key, value)) in block.header_pairs.iter().enumerate() {
            if idx > 0 {
                out.push_str(&sep);
            }
            let key_text = self.style_token(key, StyleToken::MregKey);
            let raw = value_to_display(value);
            let value_text = self.style_value_cell(value, &raw);
            out.push_str(&key_text);
            out.push_str(": ");
            out.push_str(&value_text);
        }
        out.push_str(&sep);
        out.push_str(&format!("count: {}", block.rows.len()));
        out.push('\n');
        out
    }

    fn render_grid_table(
        &self,
        block: &TableBlock,
        margin: usize,
        column_widths: Option<&[usize]>,
    ) -> String {
        if block.rows.is_empty() {
            return String::new();
        }

        let table_data = truncate_table_to_widths(
            &block.headers,
            &block.rows,
            column_widths,
            self.settings.unicode,
            self.settings.table_overflow,
        );
        let aligns = resolve_table_alignments(table_data.headers.len(), block.align.as_deref());

        let mut table = Table::new();
        table.set_content_arrangement(ContentArrangement::Disabled);
        if self.settings.unicode {
            table.load_preset(presets::UTF8_FULL_CONDENSED);
        } else {
            table.load_preset(presets::ASCII_FULL_CONDENSED);
        }

        let header_cells = table_data
            .headers
            .iter()
            .map(|header| Cell::new(self.style_token(header, StyleToken::TableHeader)))
            .collect::<Vec<Cell>>();
        table.set_header(header_cells);

        for row in &table_data.rows {
            let styled_row = row
                .iter()
                .map(|cell| self.style_value_cell(&cell.value, &cell.text))
                .map(Cell::new)
                .collect::<Vec<Cell>>();
            table.add_row(styled_row);
        }

        for (index, width) in table_data.widths.iter().enumerate() {
            if let Some(column) = table.column_mut(index) {
                let align = aligns.get(index).copied().unwrap_or(TableAlign::Default);
                column.set_cell_alignment(table_align_to_cell_alignment(align));
                let total_width = width.saturating_add(column.padding_width() as usize);
                let absolute = Width::Fixed(total_width.min(u16::MAX as usize) as u16);
                column.set_constraint(ColumnConstraint::Absolute(absolute));
            }
        }

        let rendered = format!("{table}");
        let rendered = if self.settings.unicode {
            normalize_rich_table_chrome(&rendered)
        } else {
            normalize_ascii_table_chrome(&rendered)
        };
        indent_lines(&rendered, margin)
    }

    fn render_markdown_table(
        &self,
        block: &TableBlock,
        margin: usize,
        column_widths: Option<&[usize]>,
    ) -> String {
        let table_data = truncate_table_to_widths(
            &block.headers,
            &block.rows,
            column_widths,
            self.settings.unicode,
            self.settings.table_overflow,
        );
        if table_data.headers.is_empty() {
            return String::new();
        }

        let aligns = resolve_table_alignments(table_data.headers.len(), block.align.as_deref());
        let mut widths = table_data
            .headers
            .iter()
            .map(|header| display_width(header).max(3))
            .collect::<Vec<usize>>();
        for row in &table_data.rows {
            for (index, cell) in row.iter().enumerate() {
                if let Some(width) = widths.get_mut(index) {
                    *width = (*width).max(display_width(&cell.text).max(3));
                }
            }
        }

        let mut out = String::new();
        out.push('|');
        for (index, header) in table_data.headers.iter().enumerate() {
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

        for row in &table_data.rows {
            out.push('|');
            for (index, cell) in row.iter().enumerate() {
                out.push(' ');
                out.push_str(&pad_markdown_cell(
                    &escape_markdown_cell(&cell.text),
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

    fn render_mreg_vertical_list(
        &self,
        margin_indent: &str,
        key_indent: &str,
        key_text: &str,
        values: &[Value],
        metrics: Option<MregEntryMetrics>,
    ) -> String {
        let Some((first, rest)) = values.split_first() else {
            return format!("{margin_indent}{key_indent}{key_text}:\n");
        };

        let first_gap = " ".repeat(metrics.map(|value| value.first_gap).unwrap_or(1));
        let hanging =
            " ".repeat(self.settings.margin + metrics.map(|value| value.first_pad).unwrap_or(0));
        let mut out = String::new();
        out.push_str(&format!(
            "{margin_indent}{key_indent}{key_text}:{first_gap}{}\n",
            value_to_display(first)
        ));
        for value in rest {
            out.push_str(&format!("{hanging}{}\n", value_to_display(value)));
        }
        out
    }

    fn render_grid_lines(&self, values: &[Value], depth: usize) -> Vec<String> {
        if values.is_empty() {
            return Vec::new();
        }

        let mut items = values.iter().map(value_to_display).collect::<Vec<String>>();
        items.sort_by_key(|value| value.to_ascii_lowercase());

        let available_width = self
            .settings
            .width
            .unwrap_or(100)
            .saturating_sub(self.settings.margin + (depth + 1) * self.settings.indent_size)
            .max(1);

        let (matrix, column_widths) = arrange_in_grid(
            &items,
            available_width,
            self.settings.grid_padding.max(1),
            self.settings.grid_columns,
            self.settings.column_weight.max(1),
        );

        matrix
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if index + 1 == column_widths.len() {
                            value
                        } else {
                            pad_display_width(&value, column_widths[index])
                        }
                    })
                    .collect::<Vec<String>>()
                    .join(&" ".repeat(self.settings.grid_padding))
            })
            .collect()
    }

    fn style_value_cell(&self, value: &Value, text: &str) -> String {
        let trimmed = text.trim();
        if !self.settings.color || self.settings.theme.id.eq_ignore_ascii_case("plain") {
            return text.to_string();
        }

        if is_hex_color(trimmed) {
            return apply_style_spec(text, trimmed, true);
        }

        match value {
            Value::Bool(true) => self.style_token(text, StyleToken::BoolTrue),
            Value::Bool(false) => self.style_token(text, StyleToken::BoolFalse),
            Value::Null => self.style_token(text, StyleToken::Null),
            Value::Number(_) => self.style_token(text, StyleToken::Number),
            Value::String(raw) => {
                if let Some(token) = value_style_token_for_string(raw) {
                    return self.style_token(text, token);
                }
                text.to_string()
            }
            _ => text.to_string(),
        }
    }

    fn section_divider(
        &self,
        title: &str,
        border_token: StyleToken,
        title_token: StyleToken,
    ) -> String {
        let fill_char = if self.settings.unicode { '─' } else { '-' };
        let target_width = self.settings.width.unwrap_or(24).max(12);
        let title = title.trim();

        if title.is_empty() {
            let raw = fill_char.to_string().repeat(target_width);
            if self.settings.color {
                return self.style_token(&raw, StyleToken::PanelBorder);
            }
            return raw;
        }

        let left = if self.settings.unicode { "─ " } else { "- " };
        let prefix_width = left.chars().count() + title.chars().count() + 1;
        let suffix = if prefix_width >= target_width {
            String::new()
        } else {
            fill_char.to_string().repeat(target_width - prefix_width)
        };
        let raw = format!("{left}{title} {suffix}");

        if !self.settings.color {
            return raw;
        }

        if title_token == border_token {
            return self.style_token(&raw, border_token);
        }

        let border = self.style_token(left, border_token);
        let title = self.style_token(title, title_token);
        let trailing = self.style_token(&format!(" {suffix}"), border_token);
        format!("{border}{title}{trailing}")
    }

    fn style_token(&self, text: &str, token: StyleToken) -> String {
        apply_style_with_theme_overrides(
            text,
            token,
            self.settings.color,
            &self.settings.theme,
            &self.settings.style_overrides,
        )
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

#[derive(Debug, Clone)]
struct RenderedCell {
    value: Value,
    text: String,
}

#[derive(Debug, Clone)]
struct TruncatedTable {
    headers: Vec<String>,
    rows: Vec<Vec<RenderedCell>>,
    widths: Vec<usize>,
}

fn truncate_table_to_widths(
    headers: &[String],
    rows: &[Vec<Value>],
    column_widths: Option<&[usize]>,
    unicode: bool,
    overflow: TableOverflow,
) -> TruncatedTable {
    let fallback_widths = compute_fallback_widths(headers, rows, if unicode { 4 } else { 6 });
    let widths = match column_widths {
        Some(widths) if widths.len() == headers.len() => widths.to_vec(),
        _ => fallback_widths,
    };

    let headers = headers
        .iter()
        .zip(&widths)
        .map(|(header, width)| match overflow {
            TableOverflow::None => header.clone(),
            _ => truncate_display_width_crop(header, *width),
        })
        .collect::<Vec<String>>();

    let rows = rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(index, cell)| {
                    let width = widths.get(index).copied().unwrap_or(8);
                    let raw = value_to_display(cell);
                    let rendered = match overflow {
                        TableOverflow::Clip => truncate_display_width_crop(&raw, width),
                        TableOverflow::Ellipsis => truncate_display_width(&raw, width, unicode),
                        TableOverflow::Wrap => raw.clone(),
                        TableOverflow::None => raw.clone(),
                    };
                    RenderedCell {
                        value: cell.clone(),
                        text: rendered,
                    }
                })
                .collect::<Vec<RenderedCell>>()
        })
        .collect::<Vec<Vec<RenderedCell>>>();

    TruncatedTable {
        headers,
        rows,
        widths,
    }
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
                let raw = value_to_display(cell);
                *width = (*width).max(display_width(&raw).max(min_column_width));
            }
        }
    }

    widths
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
    let mut out = value.replace('\\', "\\\\");
    out = out.replace('\r', "\\r");
    out = out.replace('\n', "\\n");
    out.replace('|', "\\|")
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

fn arrange_in_grid(
    values: &[String],
    available_width: usize,
    grid_padding: usize,
    grid_columns: Option<usize>,
    column_weight: usize,
) -> (Vec<Vec<String>>, Vec<usize>) {
    let n = values.len();
    if n <= 1 {
        return (
            vec![values.to_vec()],
            vec![values.first().map_or(0, |value| display_width(value))],
        );
    }

    if let Some(forced) = grid_columns {
        return build_grid_matrix(values, forced.max(1).min(n), grid_padding, available_width);
    }

    let mut best_cols = 1usize;
    let mut best_score = usize::MAX;
    let mut best_widths = vec![display_width(&values[0])];
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let col_widths = compute_grid_column_widths(values, cols, rows, available_width);
        let total_width = col_widths.iter().sum::<usize>() + grid_padding * cols.saturating_sub(1);
        if total_width > available_width {
            break;
        }

        let score = rows.abs_diff(cols * column_weight);
        if score <= best_score {
            best_score = score;
            best_cols = cols;
            best_widths = col_widths;
        }
    }

    let rows = n.div_ceil(best_cols);
    let matrix = build_grid_rows(values, best_cols, rows, available_width);
    (matrix, best_widths)
}

fn build_grid_matrix(
    values: &[String],
    columns: usize,
    _grid_padding: usize,
    available_width: usize,
) -> (Vec<Vec<String>>, Vec<usize>) {
    let rows = values.len().div_ceil(columns);
    let widths = compute_grid_column_widths(values, columns, rows, available_width);
    let matrix = build_grid_rows(values, columns, rows, available_width);
    (matrix, widths)
}

fn compute_grid_column_widths(
    values: &[String],
    columns: usize,
    rows: usize,
    available_width: usize,
) -> Vec<usize> {
    let mut column_widths = vec![0usize; columns];
    for (index, value) in values.iter().enumerate() {
        let column_index = index / rows;
        if column_index >= columns {
            continue;
        }
        let truncated = truncate_display_width_crop(value, available_width.max(4));
        column_widths[column_index] = column_widths[column_index].max(display_width(&truncated));
    }
    column_widths
}

fn build_grid_rows(
    values: &[String],
    columns: usize,
    rows: usize,
    available_width: usize,
) -> Vec<Vec<String>> {
    let mut matrix = vec![vec![String::new(); columns]; rows];
    for (index, value) in values.iter().enumerate() {
        let row_index = index % rows;
        let column_index = index / rows;
        if column_index >= columns {
            continue;
        }
        matrix[row_index][column_index] =
            truncate_display_width_crop(value, available_width.max(4));
    }

    matrix
        .into_iter()
        .filter(|row| row.iter().any(|cell| !cell.is_empty()))
        .collect()
}

fn fallback_mreg_metrics(block: &MregBlock, indent_size: usize) -> MregMetrics {
    let key_width = block
        .rows
        .iter()
        .flat_map(|row| row.entries.iter())
        .filter(|entry| !matches!(entry.value, MregValue::Group | MregValue::Separator))
        .map(|entry| entry.depth * indent_size + mreg_alignment_key_width(&entry.key))
        .max()
        .unwrap_or(0);
    let value_column = key_width + 2;
    let entry_metrics = block
        .rows
        .iter()
        .flat_map(|row| row.entries.iter())
        .map(|entry| match entry.value {
            MregValue::Group | MregValue::Separator => None,
            _ => {
                let base_len = entry.depth * indent_size + mreg_alignment_key_width(&entry.key) + 1;
                let full_len = entry.depth * indent_size + display_width(&entry.key) + 1;
                let render_col = value_column.max(full_len + 1);
                Some(MregEntryMetrics {
                    gap: value_column.saturating_sub(base_len).max(1),
                    render_col,
                    first_gap: render_col.saturating_sub(full_len),
                    first_pad: render_col,
                })
            }
        })
        .collect();
    MregMetrics { entry_metrics }
}

fn resolve_table_alignments(width: usize, align: Option<&[TableAlign]>) -> Vec<TableAlign> {
    let mut out = align
        .map(|value| value.to_vec())
        .unwrap_or_else(|| vec![TableAlign::Default; width]);
    if out.len() < width {
        out.extend(std::iter::repeat_n(TableAlign::Default, width - out.len()));
    }
    out.truncate(width);
    out
}

fn table_align_to_cell_alignment(align: TableAlign) -> CellAlignment {
    match align {
        TableAlign::Right => CellAlignment::Right,
        TableAlign::Center => CellAlignment::Center,
        _ => CellAlignment::Left,
    }
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
    use crate::document::{
        Block, Document, JsonBlock, MregBlock, MregEntry, MregRow, MregValue, PanelBlock,
        TableBlock, TableStyle, ValueBlock,
    };
    use crate::format;
    use crate::{RenderRuntime, RenderSettings};
    use crate::{ResolvedRenderSettings, TableOverflow};
    use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
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
            table_overflow: TableOverflow::Clip,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::style::StyleOverrides::default(),
        }
    }

    fn plain_settings_with_width(width: usize) -> ResolvedRenderSettings {
        let mut settings = settings(RenderBackend::Plain, false, false);
        settings.width = Some(width);
        settings
    }

    fn mreg_render_settings(width: usize) -> RenderSettings {
        RenderSettings {
            format: OutputFormat::Mreg,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: Some(width),
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: crate::style::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        }
    }

    fn trim_line_endings(value: &str) -> String {
        value
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
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
    fn panel_rules_match_python_plain_layout() {
        let document = Document {
            blocks: vec![Block::Panel(PanelBlock {
                title: Some("Info".to_string()),
                body: Document {
                    blocks: vec![Block::Value(ValueBlock {
                        values: vec!["alpha".to_string(), "beta".to_string()],
                    })],
                },
                rules: crate::document::PanelRules::Both,
                kind: Some("info".to_string()),
                border_token: None,
                title_token: None,
            })],
        };

        assert_eq!(
            render_document(&document, plain_settings_with_width(80)),
            concat!(
                "- Info -------------------------------------------------------------------------\n",
                "alpha\n",
                "beta\n",
                "--------------------------------------------------------------------------------\n"
            )
        );
    }

    #[test]
    fn render_mreg_respects_color_toggle() {
        let block = MregBlock {
            block_id: 1,
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
    fn mreg_scalar_entries_render_inline() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                block_id: 1,
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "members".to_string(),
                        depth: 0,
                        value: MregValue::Scalar(json!("alice")),
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
                table_overflow: TableOverflow::Clip,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );
        assert_eq!(rendered, "members: alice\n");
    }

    #[test]
    fn mreg_large_lists_use_grid_layout() {
        let values = (1..=12)
            .map(|index| json!(format!("item-{index}")))
            .collect::<Vec<Value>>();
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                block_id: 1,
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "members".to_string(),
                        depth: 0,
                        value: MregValue::Grid(values),
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
                table_overflow: TableOverflow::Clip,
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
                block_id: 1,
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
                block_id: 1,
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!("oistes")]],
                header_pairs: vec![("group".to_string(), json!("ops"))],
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        assert_eq!(
            render_document(&document, settings(RenderBackend::Plain, false, false)),
            concat!(
                "group: ops  |  count: 1\n",
                "+--------+\n",
                "| uid    |\n",
                "+--------+\n",
                "| oistes |\n",
                "+--------+\n"
            )
        );
    }

    #[test]
    fn mreg_vertical_list_matches_python_plain_layout() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                block_id: 1,
                rows: vec![MregRow {
                    entries: vec![MregEntry {
                        key: "members".to_string(),
                        depth: 0,
                        value: MregValue::VerticalList(vec![
                            json!("alice"),
                            json!("bob"),
                            json!("carol"),
                        ]),
                    }],
                }],
            })],
        };

        let mut settings = settings(RenderBackend::Plain, false, false);
        settings.short_list_max = 1;
        settings.medium_list_max = 5;

        assert_eq!(
            render_document(&document, settings),
            "members: alice\n         bob\n         carol\n"
        );
    }

    #[test]
    fn json_block_matches_python_plain_layout() {
        let document = Document {
            blocks: vec![Block::Json(JsonBlock {
                payload: json!({"uid":"oistes"}),
            })],
        };

        assert_eq!(
            render_document(&document, settings(RenderBackend::Plain, false, false)),
            "{\n  \"uid\": \"oistes\"\n}\n"
        );
    }

    #[test]
    fn ldap_user_sample_renders_as_python_style_mreg() {
        let Value::Object(row) = json!({
            "cn": "_istein S_vik",
            "eduPersonAffiliation": ["employee", "member", "staff"],
            "gecos": "\\istein S|vik",
            "gidNumber": "346297",
            "homeDirectory": "/uio/kant/usit-gsd-u1/oistes",
            "loginShell": "/local/gnu/bin/bash",
            "objectClass": [
                "uioMembership",
                "top",
                "account",
                "posixAccount",
                "uioAccountObject",
                "sambaSamAccount"
            ],
            "uid": "oistes",
            "uidNumber": "361000",
            "uioAffiliation": "ANSATT@373034",
            "uioPrimaryAffiliation": "ANSATT@373034",
            "netgroups": [
                "ansatt-373034",
                "ansatt-tekadm-373034",
                "dia-drs-vaktsjefer",
                "it-uio-azure-users",
                "it-uio-ms365-ansatt",
                "it-uio-ms365-ansatt-publisert",
                "it-uio-ms365-eapp-acos-akademiet",
                "los-alle",
                "mattermost-uio",
                "mattermost-uio-it",
                "mattermost-usit",
                "meta-ansatt-360000",
                "meta-ansatt-370000",
                "meta-ansatt-373000",
                "meta-ansatt-373034",
                "meta-ansatt-900000",
                "meta-ansatt-tekadm-360000",
                "meta-ansatt-tekadm-370000",
                "meta-ansatt-tekadm-373000",
                "meta-ansatt-tekadm-373034",
                "meta-ansatt-tekadm-900000",
                "postmaster-eo-migrerte",
                "rt-it-uu-kontakt",
                "rt-saksbehandler",
                "rt-usit-intark-drift",
                "rt-usit-lifeportal-utv-kunder",
                "rt-usit-ops",
                "rt-usit-respons",
                "ucore",
                "uio-ans",
                "uio-tils",
                "usit",
                "vcs-cfengine",
                "vcs-dhcp",
                "vcs-it-org",
                "vcs-it-osprov",
                "vcs-iti",
                "vcs-ops",
                "vcs-radius",
                "vcs-ssd",
                "vcs-usit",
                "vcs-virtprov-admins",
                "vortex-opptak",
                "zabbix-iti-ops"
            ],
            "filegroups": ["oistes", "ucore", "usit", "vortex-opptak"]
        }) else {
            panic!("expected ldap user object");
        };

        let rows = vec![row];
        let settings = mreg_render_settings(80);
        let document = format::build_document(&rows, &settings);
        let rendered = render_document(&document, settings.resolve_render_settings());

        assert_eq!(
            trim_line_endings(&rendered),
            trim_line_endings(concat!(
                "cn:                    _istein S_vik\n",
                "eduPersonAffiliation (3): employee\n",
                "                          member\n",
                "                          staff\n",
                "gecos:                 \\istein S|vik\n",
                "gidNumber:             346297\n",
                "homeDirectory:         /uio/kant/usit-gsd-u1/oistes\n",
                "loginShell:            /local/gnu/bin/bash\n",
                "objectClass (6):\n",
                "  account            top             \n",
                "  posixAccount       uioAccountObject\n",
                "  sambaSamAccount    uioMembership   \n",
                "uid:                   oistes\n",
                "uidNumber:             361000\n",
                "uioAffiliation:        ANSATT@373034\n",
                "uioPrimaryAffiliation: ANSATT@373034\n",
                "netgroups (44):\n",
                "  ansatt-373034                       rt-it-uu-kontakt             \n",
                "  ansatt-tekadm-373034                rt-saksbehandler             \n",
                "  dia-drs-vaktsjefer                  rt-usit-intark-drift         \n",
                "  it-uio-azure-users                  rt-usit-lifeportal-utv-kunder\n",
                "  it-uio-ms365-ansatt                 rt-usit-ops                  \n",
                "  it-uio-ms365-ansatt-publisert       rt-usit-respons              \n",
                "  it-uio-ms365-eapp-acos-akademiet    ucore                        \n",
                "  los-alle                            uio-ans                      \n",
                "  mattermost-uio                      uio-tils                     \n",
                "  mattermost-uio-it                   usit                         \n",
                "  mattermost-usit                     vcs-cfengine                 \n",
                "  meta-ansatt-360000                  vcs-dhcp                     \n",
                "  meta-ansatt-370000                  vcs-it-org                   \n",
                "  meta-ansatt-373000                  vcs-it-osprov                \n",
                "  meta-ansatt-373034                  vcs-iti                      \n",
                "  meta-ansatt-900000                  vcs-ops                      \n",
                "  meta-ansatt-tekadm-360000           vcs-radius                   \n",
                "  meta-ansatt-tekadm-370000           vcs-ssd                      \n",
                "  meta-ansatt-tekadm-373000           vcs-usit                     \n",
                "  meta-ansatt-tekadm-373034           vcs-virtprov-admins          \n",
                "  meta-ansatt-tekadm-900000           vortex-opptak                \n",
                "  postmaster-eo-migrerte              zabbix-iti-ops               \n",
                "filegroups (4):        oistes\n",
                "                       ucore\n",
                "                       usit\n",
                "                       vortex-opptak\n"
            ))
        );
    }

    #[test]
    fn nested_object_lists_stack_when_table_would_hide_content() {
        let Value::Object(row) = json!({
            "id": 55753,
            "ipaddresses": [
                {
                    "id": 57171,
                    "macaddress": "10:62:e5:19:74:4a",
                    "created_at": "2019-12-02T21:50:27.600379+01:00",
                    "updated_at": "2022-06-20T09:51:40.448942+02:00",
                    "ipaddress": "129.240.130.83",
                    "host": 55753
                },
                {
                    "id": 57172,
                    "macaddress": "",
                    "created_at": "2019-12-02T21:50:28.054904+01:00",
                    "updated_at": "2023-01-20T16:06:24.373064+01:00",
                    "ipaddress": "2001:700:100:4003::83",
                    "host": 55753
                }
            ],
            "networks": [
                {
                    "policy": null,
                    "communities": [
                        {
                            "id": 3,
                            "name": "laptops",
                            "description": "Laptops",
                            "network": 1733,
                            "global_name": "community02"
                        },
                        {
                            "id": 2,
                            "name": "workstations",
                            "description": "Workstations",
                            "network": 1733,
                            "global_name": "community01"
                        }
                    ],
                    "network": "129.240.130.0/24",
                    "description": "knh-klientnett-2 (statisk DHCP)",
                    "vlan": 200,
                    "dns_delegated": false,
                    "category": "kn",
                    "location": "usit",
                    "frozen": false,
                    "reserved": 3,
                    "max_communities": null
                },
                {
                    "policy": null,
                    "network": "2001:700:100:4003::/64",
                    "description": "usit-knh",
                    "vlan": 200,
                    "dns_delegated": false,
                    "category": "",
                    "location": "",
                    "frozen": false,
                    "reserved": 3,
                    "max_communities": null
                }
            ]
        }) else {
            panic!("expected object");
        };

        let rows = vec![row];
        let settings = mreg_render_settings(100);
        let document = format::build_document(&rows, &settings);
        let rendered = render_document(&document, settings.resolve_render_settings());

        assert!(rendered.contains("ipaddresses (2):"));
        assert!(rendered.contains("| id"));
        assert!(rendered.contains("networks (2):"));
        assert!(rendered.contains("communities (2):"));
        assert!(rendered.contains("community02"));
        assert!(rendered.contains("| global_name"));
        assert!(!rendered.contains("{'id': 3"));
    }

    #[test]
    fn table_color_never_has_no_ansi_escape_codes() {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 1,
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
                block_id: 1,
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
                block_id: 1,
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
                block_id: 1,
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
                block_id: 1,
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
                block_id: 1,
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
                table_overflow: TableOverflow::Clip,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );

        assert!(!rendered.contains("..."));
        assert!(!rendered.contains("this-is-a-very-long-cell-that-should-truncate"));
    }

    #[test]
    fn table_overflow_none_keeps_full_content() {
        let long = "this-is-a-very-long-cell-that-should-not-truncate";
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 1,
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!(long)]],
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
                width: Some(20),
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                table_overflow: TableOverflow::None,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );

        assert!(rendered.contains(long));
    }

    #[test]
    fn table_overflow_ellipsis_truncates_with_suffix() {
        let long = "this-is-a-very-long-cell-that-should-truncate";
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 1,
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!(long)]],
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
                width: Some(20),
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                table_overflow: TableOverflow::Ellipsis,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );

        assert!(rendered.contains("..."));
        assert!(!rendered.contains(long));
    }

    #[test]
    fn table_overflow_wrap_preserves_tail() {
        let long = "abcdefghijklmno";
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 1,
                style: TableStyle::Grid,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!(long)]],
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
                width: Some(12),
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                table_overflow: TableOverflow::Wrap,
                theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::style::StyleOverrides::default(),
            },
        );

        assert!(rendered.contains("mno"));
        assert!(!rendered.contains("..."));
    }

    #[test]
    fn plain_theme_does_not_style_hex_cells() {
        let document = Document {
            blocks: vec![Block::Mreg(MregBlock {
                block_id: 1,
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
                table_overflow: TableOverflow::Clip,
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
                block_id: 1,
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
                table_overflow: TableOverflow::Clip,
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
                table_overflow: TableOverflow::Clip,
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
