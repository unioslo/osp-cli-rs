//! Terminal renderer for structured UI documents.
//!
//! This module exists to turn the already-shaped [`crate::ui::document::Document`]
//! model into terminal text with consistent width, alignment, color, and
//! unicode behavior.
//!
//! High-level flow:
//!
//! - precompute shared layout metrics for the full document
//! - render each block according to the resolved backend, width, and theme
//! - keep width-sensitive formats such as tables and MREG output internally
//!   consistent across the whole render
//!
//! Contract:
//!
//! - document shaping belongs upstream in `ui::format`
//! - terminal string emission belongs here
//! - this module should not start making output-format selection or config
//!   precedence decisions

use comfy_table::{
    Cell, CellAlignment, ColumnConstraint, ContentArrangement, Table, Width, presets,
};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::chrome::{
    SectionRenderContext, SectionStyleTokens, render_section_block_with_overrides,
    render_section_divider_with_columns,
};
use crate::ui::display::value_to_display;
use crate::ui::document::{
    Block, Document, MregBlock, MregValue, PanelRules, TableAlign, TableBlock, TableStyle,
    ValueLayout,
};
use crate::ui::layout::{LayoutContext, MregEntryMetrics, MregMetrics, prepare_layout_context};
use crate::ui::style::{StyleToken, apply_style_spec, apply_style_with_theme_overrides};
use crate::ui::width::{display_width, mreg_alignment_key_width};
use crate::ui::{RenderBackend, ResolvedRenderSettings, TableBorderStyle, TableOverflow};

/// Renders a structured UI document using the resolved terminal settings.
///
/// The renderer precomputes shared layout metrics once and then renders block
/// by block. This keeps width-sensitive formats such as tables and MREG views
/// internally consistent across the full document.
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
            // Layout is computed once for the whole document so repeated blocks
            // can share stable widths instead of each block guessing locally.
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
                    // Plain mode favors predictable, copy-friendly JSON over
                    // token styling; rich mode renders the same structure with
                    // semantic coloring.
                    indent_lines(
                        &serde_json::to_string_pretty(&json.payload)
                            .unwrap_or_else(|_| "[]".to_string()),
                        self.settings.margin,
                    )
                } else {
                    self.render_json_block(&json.payload)
                }
            }
            Block::Value(values) => self.render_value_block(values),
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

    fn render_value_block(&self, block: &crate::ui::document::ValueBlock) -> String {
        if block.values.is_empty() {
            return String::new();
        }

        if matches!(block.layout, ValueLayout::AutoGrid)
            && block.values.len() > self.settings.medium_list_max
        {
            return self.render_value_grid(block);
        }

        let prefix = " ".repeat(self.settings.margin + block.indent);
        block
            .values
            .iter()
            .map(|value| format!("{prefix}{}\n", self.render_value_item(block, value)))
            .collect()
    }

    fn render_value_grid(&self, block: &crate::ui::document::ValueBlock) -> String {
        let visible = block
            .values
            .iter()
            .map(|value| visible_value_text(value, block.inline_markup))
            .collect::<Vec<_>>();
        let available_width = self
            .settings
            .width
            .unwrap_or(100)
            .saturating_sub(self.settings.margin + block.indent)
            .max(1);
        let (matrix, column_widths) = arrange_in_grid(
            &visible,
            available_width,
            self.settings.grid_padding.max(1),
            self.settings.grid_columns,
            self.settings.column_weight.max(1),
        );
        let rows = matrix.len();
        let columns = column_widths.len();
        let prefix = " ".repeat(self.settings.margin + block.indent);
        let mut out = String::new();

        for (row_index, row) in matrix.iter().enumerate().take(rows) {
            out.push_str(&prefix);
            let mut first = true;
            for column_index in 0..columns {
                let value_index = column_index * rows + row_index;
                if value_index >= block.values.len() {
                    continue;
                }

                let visible_cell = &row[column_index];
                if visible_cell.is_empty() {
                    continue;
                }

                if !first {
                    out.push_str(&" ".repeat(self.settings.grid_padding));
                }
                first = false;

                let mut rendered = if block.inline_markup && visible_cell == &visible[value_index] {
                    // Preserve inline span styling only when the arranged grid
                    // cell text still matches the original visible value.
                    // Once layout truncates or rewrites the cell, fall back to
                    // uniform value styling so we do not color the wrong
                    // fragments inside a shortened grid cell.
                    self.render_inline_value(&block.values[value_index])
                } else {
                    self.style_token(visible_cell, StyleToken::Value)
                };

                if column_index + 1 != columns {
                    let pad =
                        column_widths[column_index].saturating_sub(display_width(visible_cell));
                    rendered.push_str(&" ".repeat(pad));
                }
                out.push_str(&rendered);
            }
            out.push('\n');
        }

        out
    }

    fn render_inline_value(&self, value: &str) -> String {
        crate::ui::inline::render_inline(
            value,
            self.settings.color,
            &self.settings.theme,
            &self.settings.style_overrides,
        )
    }

    fn render_value_item(&self, block: &crate::ui::document::ValueBlock, value: &str) -> String {
        if block.inline_markup {
            self.render_inline_value(value)
        } else {
            self.style_token(value, StyleToken::Value)
        }
    }

    fn render_line_block(&self, block: &crate::ui::document::LineBlock) -> String {
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

    fn render_code_block(&self, block: &crate::ui::document::CodeBlock) -> String {
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

    fn render_panel_block(&self, block: &crate::ui::document::PanelBlock) -> String {
        let fallback =
            section_style_token(block.kind.as_deref()).unwrap_or(StyleToken::PanelBorder);
        let border_token = block.border_token.unwrap_or(fallback);
        let title_token = block.title_token.unwrap_or(StyleToken::PanelTitle);
        let inner = DocumentRenderer::new(&block.body, self.settings).render(&block.body);
        let render = SectionRenderContext {
            color: self.settings.color,
            theme: &self.settings.theme,
            style_overrides: &self.settings.style_overrides,
        };
        let tokens = SectionStyleTokens {
            border: border_token,
            title: title_token,
        };

        if let Some(frame_style) = block.frame_style {
            return render_section_block_with_overrides(
                block.title.as_deref().unwrap_or(""),
                inner.trim_end_matches('\n'),
                frame_style,
                self.settings.unicode,
                self.settings.width,
                render,
                tokens,
            );
        }

        let divider_width = Some(self.settings.width.unwrap_or(24).max(12));
        let title_columns = self.settings.margin.max(2);
        let titled_divider = render_section_divider_with_columns(
            block.title.as_deref().unwrap_or(""),
            self.settings.unicode,
            divider_width,
            title_columns,
            render,
            tokens,
        );
        let trailing_divider = render_section_divider_with_columns(
            "",
            self.settings.unicode,
            divider_width,
            title_columns,
            render,
            SectionStyleTokens::same(border_token),
        );

        match block.rules {
            PanelRules::None => inner,
            PanelRules::Top => format!("{titled_divider}\n{inner}"),
            PanelRules::Bottom => format!("{inner}{trailing_divider}\n"),
            PanelRules::Both => format!("{titled_divider}\n{inner}{trailing_divider}\n"),
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

        let layout_margin = self.settings.margin
            + if respect_depth {
                block.depth * self.settings.indent_size
            } else {
                0
            };

        let table_body = match block.style {
            TableStyle::Grid => self.render_grid_table(block, layout_margin, column_widths),
            TableStyle::Guide => self.render_guide_table(block, layout_margin, column_widths),
            TableStyle::Markdown => self.render_markdown_table(block, layout_margin, column_widths),
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
        // Header pairs act as a compact grouped summary, so include the row
        // count here instead of forcing the table body to carry that context.
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

        // Width truncation happens before handing cells to comfy-table so the
        // final output respects the renderer's overflow policy instead of the
        // table library making its own wrapping decisions.
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
        let border_style = block.border_override.unwrap_or(self.settings.table_border);
        match (self.settings.unicode, border_style) {
            (true, TableBorderStyle::None) => {
                table.load_preset(presets::UTF8_NO_BORDERS);
            }
            (false, TableBorderStyle::None) => {
                table.load_preset(presets::ASCII_NO_BORDERS);
            }
            (true, TableBorderStyle::Square | TableBorderStyle::Round) => {
                table.load_preset(presets::UTF8_FULL_CONDENSED);
            }
            (false, TableBorderStyle::Square | TableBorderStyle::Round) => {
                table.load_preset(presets::ASCII_FULL_CONDENSED);
            }
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
            normalize_rich_table_chrome(&rendered, border_style)
        } else {
            normalize_ascii_table_chrome(&rendered, border_style)
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

    fn render_guide_table(
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
            .map(|header| display_width(header))
            .collect::<Vec<usize>>();
        for row in &table_data.rows {
            for (index, cell) in row.iter().enumerate() {
                if let Some(width) = widths.get_mut(index) {
                    *width = (*width).max(display_width(&cell.text));
                }
            }
        }

        let mut out = String::new();
        out.push_str(
            &self.render_guide_table_row(
                &table_data
                    .headers
                    .iter()
                    .enumerate()
                    .map(|(index, header)| {
                        let align = aligns.get(index).copied().unwrap_or(TableAlign::Default);
                        (
                            pad_plain_cell(header, widths[index], align),
                            Some(StyleToken::TableHeader),
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
        );
        out.push('\n');

        for row in &table_data.rows {
            out.push_str(
                &self.render_guide_table_row(
                    &row.iter()
                        .enumerate()
                        .map(|(index, cell)| {
                            let align = aligns.get(index).copied().unwrap_or(TableAlign::Default);
                            (
                                pad_plain_cell(&cell.text, widths[index], align),
                                Some(value_style_token(&cell.value)),
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            );
            out.push('\n');
        }

        indent_lines(&out, margin)
    }

    fn render_guide_table_row(&self, cells: &[(String, Option<StyleToken>)]) -> String {
        let mut out = String::new();
        for (index, (text, token)) in cells.iter().enumerate() {
            if index > 0 {
                out.push_str("  ");
            }
            if let Some(token) = token {
                out.push_str(&apply_style_with_theme_overrides(
                    text,
                    *token,
                    self.settings.color,
                    &self.settings.theme,
                    &self.settings.style_overrides,
                ));
            } else {
                out.push_str(text);
            }
        }
        out
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
                self.style_token(text, StyleToken::Value)
            }
            _ => self.style_token(text, StyleToken::Value),
        }
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

fn normalize_rich_table_chrome(rendered: &str, border_style: TableBorderStyle) -> String {
    if matches!(border_style, TableBorderStyle::None) {
        return rendered.to_string();
    }

    // comfy-table gets close to the desired heavy/light border mix, but the
    // final chrome still needs a normalization pass to match the CLI theme.
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

    if matches!(border_style, TableBorderStyle::Round) {
        round_rich_table_outer_corners(&out)
    } else {
        out
    }
}

fn normalize_ascii_table_chrome(rendered: &str, border_style: TableBorderStyle) -> String {
    if matches!(border_style, TableBorderStyle::None) {
        return rendered.to_string();
    }

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

fn round_rich_table_outer_corners(rendered: &str) -> String {
    let mut lines = rendered.lines().map(str::to_string).collect::<Vec<_>>();
    if let Some(first) = lines.first_mut() {
        replace_first_char_if(first, &['┏', '┌'], '╭');
        replace_last_char_if(first, &['┓', '┐'], '╮');
    }
    if let Some(last) = lines.last_mut() {
        replace_first_char_if(last, &['┗', '└'], '╰');
        replace_last_char_if(last, &['┛', '┘'], '╯');
    }

    let mut out = lines.join("\n");
    if rendered.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn replace_first_char_if(text: &mut String, expected: &[char], replacement: char) {
    let mut chars = text.chars().collect::<Vec<_>>();
    if chars.first().is_some_and(|value| expected.contains(value)) {
        chars[0] = replacement;
        *text = chars.into_iter().collect();
    }
}

fn replace_last_char_if(text: &mut String, expected: &[char], replacement: char) {
    let mut chars = text.chars().collect::<Vec<_>>();
    if chars.last().is_some_and(|value| expected.contains(value)) {
        let last_index = chars.len().saturating_sub(1);
        chars[last_index] = replacement;
        *text = chars.into_iter().collect();
    }
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

fn pad_plain_cell(value: &str, width: usize, align: TableAlign) -> String {
    let rendered_width = display_width(value);
    if rendered_width >= width {
        return value.to_string();
    }

    let gap = width - rendered_width;
    match align {
        TableAlign::Right => format!("{}{}", " ".repeat(gap), value),
        TableAlign::Center => {
            let left = gap / 2;
            let right = gap - left;
            format!("{}{}{}", " ".repeat(left), value, " ".repeat(right))
        }
        TableAlign::Default | TableAlign::Left => format!("{}{}", value, " ".repeat(gap)),
    }
}

fn value_style_token(value: &Value) -> StyleToken {
    match value {
        Value::Bool(true) => StyleToken::BoolTrue,
        Value::Bool(false) => StyleToken::BoolFalse,
        Value::Null => StyleToken::Null,
        Value::Number(_) => StyleToken::Number,
        Value::String(raw) => value_style_token_for_string(raw).unwrap_or(StyleToken::Value),
        _ => StyleToken::Value,
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

fn visible_value_text(value: &str, inline_markup: bool) -> String {
    if !inline_markup {
        return value.to_string();
    }
    crate::ui::inline::parts_from_inline(value)
        .into_iter()
        .map(|part| part.text)
        .collect()
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
mod tests;
