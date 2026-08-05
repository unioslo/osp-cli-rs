use unicode_width::UnicodeWidthStr;

use crate::core::output_model::ColumnAlignment;
use crate::ui::chrome::{
    FULL_HELP_LAYOUT_CHROME, GUIDE_SECTION_CHROME, PLAIN_SECTION_CHROME, RenderedTitle,
};
use crate::ui::doc::{
    Block, Doc, GuideEntriesBlock, KeyValueBlock, KeyValueRow, KeyValueStyle, KeyValueValue,
    ListBlock, ParagraphBlock, SectionBlock, SectionTitleChrome, TableBlock,
};
use crate::ui::settings::{RenderBackend, ResolvedRenderSettings, TableBorderStyle, TableOverflow};
use crate::ui::style::{StyleToken, ThemeStyler};
use crate::ui::text::{crop_display_width, wrap_display_width};
use crate::ui::visible_inline_text;

use super::grid::PreparedGridList;
use super::guide_entries::{PreparedGuideEntriesBlock, PreparedGuideEntryRow};
use super::key_value::{aligned_display_key_width, display_key};
use super::shared::{format_list_item, indent_lines};
use super::table::{PreparedCell, PreparedTable};

pub(super) fn emit_doc(doc: &Doc, settings: &ResolvedRenderSettings) -> String {
    let rendered = emit_blocks(&doc.blocks, settings);
    if rendered.is_empty() || rendered.ends_with('\n') {
        rendered
    } else {
        format!("{rendered}\n")
    }
}

fn emit_blocks(blocks: &[Block], settings: &ResolvedRenderSettings) -> String {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Blank => out.push(String::new()),
            Block::Rule => {
                if let Some(rule) = emit_rule(settings) {
                    out.push(rule);
                }
            }
            Block::Paragraph(block) => out.push(emit_paragraph(block, settings)),
            Block::Section(block) => out.push(emit_section(block, settings)),
            Block::Table(block) => out.push(emit_table(block, settings)),
            Block::GuideEntries(block) => out.push(emit_guide_entries(block, settings)),
            Block::KeyValue(block) => out.push(emit_key_value(block, settings)),
            Block::List(block) => out.push(emit_list(block, settings)),
            Block::Json(block) => out.push(indent_lines(&block.text, settings.margin)),
        }
    }
    out.join("\n")
}

fn emit_paragraph(block: &ParagraphBlock, settings: &ResolvedRenderSettings) -> String {
    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    let text = if block.inline_markup {
        visible_inline_text(&block.text)
    } else {
        block.text.clone()
    };
    let styled = indent_lines(&text, block.indent)
        .lines()
        .map(|line| styler.paint_value(line))
        .collect::<Vec<_>>()
        .join("\n");
    indent_lines(&styled, settings.margin)
}

fn emit_section(block: &SectionBlock, settings: &ResolvedRenderSettings) -> String {
    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    let chrome = section_chrome(block.title_chrome);
    let mut body_settings = settings.clone();
    body_settings.margin += block.body_indent;
    let body = emit_blocks(&block.blocks, &body_settings);
    let mut parts = Vec::new();
    if let Some(title) = block.title.as_deref() {
        let mut title_line = style_title_line(
            &chrome.render_title_line(title, settings.width, settings.unicode),
            &styler,
        );
        if let Some(suffix) = block.inline_title_suffix.as_deref() {
            title_line.push(' ');
            title_line.push_str(suffix);
        }
        let title_margin = match block.title_chrome {
            SectionTitleChrome::Plain => settings.margin,
            SectionTitleChrome::Ruled => 0,
        };
        parts.push(indent_lines(&title_line, title_margin));
    }
    if !body.is_empty() {
        parts.push(body);
    }
    let rendered = parts.join("\n");
    if block.trailing_newline && !rendered.is_empty() {
        format!("{rendered}\n")
    } else {
        rendered
    }
}

fn emit_rule(settings: &ResolvedRenderSettings) -> Option<String> {
    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    FULL_HELP_LAYOUT_CHROME
        .render_footer_rule(settings.width, settings.unicode)
        .map(|rule| styler.paint(&rule, StyleToken::Border))
}

fn section_chrome(title_chrome: SectionTitleChrome) -> crate::ui::chrome::SectionChrome {
    match title_chrome {
        SectionTitleChrome::Plain => PLAIN_SECTION_CHROME,
        SectionTitleChrome::Ruled => GUIDE_SECTION_CHROME,
    }
}

fn emit_key_value(block: &KeyValueBlock, settings: &ResolvedRenderSettings) -> String {
    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    let rendered = match block.style {
        KeyValueStyle::Plain => emit_plain_rows(&block.rows, "", settings, &styler),
        KeyValueStyle::Bulleted => emit_bulleted_rows(&block.rows, "", settings, &styler),
    };
    indent_lines(&rendered, settings.margin)
}

fn emit_guide_entries(block: &GuideEntriesBlock, settings: &ResolvedRenderSettings) -> String {
    emit_prepared_guide_entries(
        &PreparedGuideEntriesBlock::from_block(block).rows,
        settings.margin,
        settings,
    )
}

fn emit_prepared_guide_entries(
    rows: &[PreparedGuideEntryRow],
    margin: usize,
    settings: &ResolvedRenderSettings,
) -> String {
    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    rows.iter()
        .map(|row| indent_lines(&emit_guide_entry_row(row, &styler), margin))
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_guide_entry_row(row: &PreparedGuideEntryRow, styler: &ThemeStyler<'_>) -> String {
    let key = styler.paint(&row.key, StyleToken::Key);
    if row.value.is_empty() {
        format!("{}{}", row.indent, key)
    } else {
        let value = styler.paint_value(&row.value);
        format!("{}{}{}{}", row.indent, key, row.gap, value)
    }
}

fn emit_list(block: &ListBlock, settings: &ResolvedRenderSettings) -> String {
    if block.auto_grid && block.items.len() > settings.medium_list_max {
        return emit_grid_list(block, settings);
    }

    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    block
        .items
        .iter()
        .map(|item| {
            indent_lines(
                &styler.paint_value(&format_list_item(item, block.inline_markup)),
                settings.margin + block.indent,
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_grid_list(block: &ListBlock, settings: &ResolvedRenderSettings) -> String {
    let visible = block
        .items
        .iter()
        .map(|item| format_list_item(item, block.inline_markup))
        .collect::<Vec<_>>();
    let available_width = settings
        .width
        .unwrap_or(100)
        .saturating_sub(settings.margin + block.indent)
        .max(1);
    let grid = PreparedGridList::from_items(&visible, available_width);
    let prefix = " ".repeat(settings.margin + block.indent);
    let mut out = String::new();

    for row in &grid.rows {
        out.push_str(&prefix);
        let mut first = true;
        for (column_index, cell) in row.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            if !first {
                out.push_str(&" ".repeat(grid.gap));
            }
            first = false;
            out.push_str(cell);
            if column_index + 1 != grid.column_widths.len() {
                let pad = grid.column_widths[column_index]
                    .saturating_sub(UnicodeWidthStr::width(cell.as_str()));
                out.push_str(&" ".repeat(pad));
            }
        }
        out.push('\n');
    }

    out.trim_end_matches('\n').to_string()
}

fn emit_table(block: &TableBlock, settings: &ResolvedRenderSettings) -> String {
    if block.headers.is_empty() {
        return String::new();
    }

    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    let table = PreparedTable::for_terminal(block);
    let widths = fitted_table_widths(
        &table.widths,
        settings
            .width
            .map(|width| width.saturating_sub(settings.margin)),
        settings.table_overflow,
    );
    let mut lines = Vec::new();
    if !block.summary.is_empty() {
        lines.push(indent_lines(
            &format_summary(&block.summary, settings, &styler),
            settings.margin,
        ));
    }

    let unicode =
        settings.unicode && matches!(settings.backend, RenderBackend::Rich | RenderBackend::Plain);
    let border = table_border_chars(unicode, settings.table_border);
    lines.push(indent_lines(
        &styler.paint(
            &table_rule(
                &widths,
                border.top_left,
                border.join_top,
                border.top_right,
                border.horizontal,
            ),
            StyleToken::Border,
        ),
        settings.margin,
    ));
    lines.extend(
        table_row_lines(
            &table.headers,
            &widths,
            &table.column_align,
            border.vertical,
            &styler,
            true,
            settings.table_overflow,
        )
        .into_iter()
        .map(|line| indent_lines(&line, settings.margin)),
    );
    lines.push(indent_lines(
        &styler.paint(
            &table_rule(
                &widths,
                border.join_left,
                border.join_mid,
                border.join_right,
                border.horizontal,
            ),
            StyleToken::Border,
        ),
        settings.margin,
    ));
    for row in &table.rows {
        lines.extend(
            table_row_lines(
                row,
                &widths,
                &table.column_align,
                border.vertical,
                &styler,
                false,
                settings.table_overflow,
            )
            .into_iter()
            .map(|line| indent_lines(&line, settings.margin)),
        );
    }
    lines.push(indent_lines(
        &styler.paint(
            &table_rule(
                &widths,
                border.bottom_left,
                border.join_bottom,
                border.bottom_right,
                border.horizontal,
            ),
            StyleToken::Border,
        ),
        settings.margin,
    ));
    lines.join("\n")
}

fn fitted_table_widths(
    natural: &[usize],
    available_width: Option<usize>,
    overflow: TableOverflow,
) -> Vec<usize> {
    let Some(available_width) = available_width else {
        return natural.to_vec();
    };
    if matches!(overflow, TableOverflow::None) || natural.is_empty() {
        return natural.to_vec();
    }

    // Each column has two spaces and one separator, plus the final separator.
    let cell_budget = available_width.saturating_sub(natural.len() * 3 + 1);
    if natural.iter().sum::<usize>() <= cell_budget {
        return natural.to_vec();
    }

    let minimum = if cell_budget >= natural.len() * 3 {
        3
    } else {
        1
    };
    let mut widths = natural
        .iter()
        .map(|width| (*width).min(minimum))
        .collect::<Vec<_>>();
    let mut remaining = cell_budget.saturating_sub(widths.iter().sum::<usize>());

    while remaining > 0 {
        let mut changed = false;
        for (width, natural_width) in widths.iter_mut().zip(natural) {
            if *width < *natural_width {
                *width += 1;
                remaining -= 1;
                changed = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }

    widths
}

fn table_row_lines(
    cells: &[PreparedCell],
    widths: &[usize],
    column_align: &[ColumnAlignment],
    vertical: char,
    styler: &ThemeStyler<'_>,
    header: bool,
    overflow: TableOverflow,
) -> Vec<String> {
    let cell_lines = widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let raw = cells.get(index).map(|cell| cell.raw.as_str()).unwrap_or("");
            table_cell_lines(raw, *width, overflow)
        })
        .collect::<Vec<_>>();
    let height = cell_lines.iter().map(Vec::len).max().unwrap_or(1);

    (0..height)
        .map(|line_index| {
            let line_cells = cell_lines
                .iter()
                .map(|lines| PreparedCell {
                    raw: lines.get(line_index).cloned().unwrap_or_default(),
                    markdown: String::new(),
                    width: lines
                        .get(line_index)
                        .map(|line| UnicodeWidthStr::width(line.as_str()))
                        .unwrap_or(0),
                })
                .collect::<Vec<_>>();
            table_row(&line_cells, widths, column_align, vertical, styler, header)
        })
        .collect()
}

fn table_cell_lines(raw: &str, width: usize, overflow: TableOverflow) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    match overflow {
        TableOverflow::None => vec![raw.to_string()],
        TableOverflow::Clip => vec![crop_display_width(raw, width)],
        TableOverflow::Ellipsis => {
            if UnicodeWidthStr::width(raw) <= width {
                vec![raw.to_string()]
            } else if width == 1 {
                vec!["…".to_string()]
            } else {
                vec![format!("{}…", crop_display_width(raw, width - 1))]
            }
        }
        TableOverflow::Wrap => wrap_display_width(raw, width),
    }
}

fn format_summary(
    rows: &[KeyValueRow],
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> String {
    if rows.iter().any(|row| value_requires_block(&row.value)) {
        return emit_plain_rows(rows, "", settings, styler);
    }

    let sep = if settings.unicode { "  ·  " } else { "  |  " };
    let sep = styler.paint(sep, StyleToken::Punctuation);
    rows.iter()
        .map(|row| {
            format!(
                "{}{} {}",
                styler.paint(&display_key(row), StyleToken::Key),
                styler.paint(":", StyleToken::Punctuation),
                styler.paint_value(value_scalar_text(&row.value).unwrap_or_default())
            )
        })
        .collect::<Vec<_>>()
        .join(&sep)
}

fn emit_plain_rows(
    rows: &[KeyValueRow],
    base_indent: &str,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> String {
    let key_width = aligned_display_key_width(rows);
    rows.iter()
        .flat_map(|row| emit_plain_row_lines(row, base_indent, key_width, settings, styler))
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_plain_row_lines(
    row: &KeyValueRow,
    base_indent: &str,
    key_width: usize,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> Vec<String> {
    let effective_indent = format!("{base_indent}{}", row.indent.as_deref().unwrap_or_default());
    let display_key = display_key(row);
    let display_key_width = UnicodeWidthStr::width(display_key.as_str());
    let value_spacing = " ".repeat(
        key_width
            .saturating_sub(display_key_width)
            .saturating_add(1),
    );
    let key = styler.paint(&display_key, StyleToken::Key);
    let label = format!(
        "{}{}{}",
        effective_indent,
        key,
        styler.paint(":", StyleToken::Punctuation)
    );
    let continuation_indent = format!(
        "{}{}",
        effective_indent,
        " ".repeat(display_key_width.saturating_add(1 + value_spacing.len()))
    );
    let child_indent = format!("{effective_indent}{}", " ".repeat(settings.indent_size));

    match &row.value {
        KeyValueValue::Empty => vec![label],
        KeyValueValue::Scalar(text) if text.is_empty() => vec![label],
        KeyValueValue::Scalar(text) => {
            vec![format!(
                "{label}{value_spacing}{}",
                styler.paint_value(text)
            )]
        }
        KeyValueValue::Object(rows) => {
            let mut lines = vec![label];
            if !rows.is_empty() {
                lines.push(emit_plain_rows(rows, &child_indent, settings, styler));
            }
            lines
        }
        KeyValueValue::Array(items) => emit_plain_array_lines(
            items,
            &label,
            &format!("{label}{value_spacing}"),
            &continuation_indent,
            &child_indent,
            settings,
            styler,
        ),
    }
}

fn emit_plain_array_lines(
    items: &[KeyValueValue],
    label: &str,
    inline_prefix: &str,
    continuation_indent: &str,
    child_indent: &str,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> Vec<String> {
    let Some(scalar_items) = items
        .iter()
        .map(value_scalar_text)
        .collect::<Option<Vec<_>>>()
    else {
        return emit_plain_complex_array_lines(items, label, child_indent, settings, styler);
    };

    match scalar_items.as_slice() {
        [] => vec![label.to_string()],
        [only] if items.len() == 1 && !array_uses_count(items) => {
            vec![format!("{inline_prefix}{}", styler.paint_value(only))]
        }
        values if values.len() > settings.medium_list_max => {
            let mut lines = vec![label.to_string()];
            lines.extend(render_terminal_scalar_grid(
                values,
                child_indent,
                settings,
                styler,
            ));
            lines
        }
        [first, rest @ ..] => {
            let mut lines = vec![format!("{inline_prefix}{}", styler.paint_value(first))];
            lines.extend(
                rest.iter()
                    .map(|value| format!("{continuation_indent}{}", styler.paint_value(value))),
            );
            lines
        }
    }
}

fn emit_plain_complex_array_lines(
    items: &[KeyValueValue],
    label: &str,
    child_indent: &str,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> Vec<String> {
    let mut lines = vec![label.to_string()];
    for (index, item) in items.iter().enumerate() {
        lines.extend(emit_plain_array_item_lines(
            item,
            index,
            child_indent,
            settings,
            styler,
        ));
    }
    lines
}

fn emit_plain_array_item_lines(
    item: &KeyValueValue,
    index: usize,
    child_indent: &str,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> Vec<String> {
    let marker = styler.paint(&format!("[{}]", index + 1), StyleToken::Punctuation);
    let colon = styler.paint(":", StyleToken::Punctuation);
    match item {
        KeyValueValue::Empty => vec![format!("{child_indent}{marker}")],
        KeyValueValue::Scalar(text) => {
            vec![format!(
                "{child_indent}{marker} {colon} {}",
                styler.paint_value(text)
            )]
        }
        KeyValueValue::Object(rows) => {
            let nested_indent = format!("{child_indent}{}", " ".repeat(settings.indent_size));
            let mut lines = vec![format!("{child_indent}{marker}{colon}")];
            if !rows.is_empty() {
                lines.push(emit_plain_rows(rows, &nested_indent, settings, styler));
            }
            lines
        }
        KeyValueValue::Array(items) => {
            let nested_indent = format!("{child_indent}{}", " ".repeat(settings.indent_size));
            let mut lines = vec![format!("{child_indent}{marker}{colon}")];
            lines.extend(emit_plain_array_lines(
                items,
                &format!("{nested_indent}{}", styler.paint("items", StyleToken::Key)),
                &format!("{nested_indent}{}", styler.paint("items", StyleToken::Key)),
                &nested_indent,
                &format!("{nested_indent}{}", " ".repeat(settings.indent_size)),
                settings,
                styler,
            ));
            lines
        }
    }
}

fn emit_bulleted_rows(
    rows: &[KeyValueRow],
    base_indent: &str,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> String {
    rows.iter()
        .flat_map(|row| emit_bulleted_row_lines(row, base_indent, settings, styler))
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_bulleted_row_lines(
    row: &KeyValueRow,
    base_indent: &str,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> Vec<String> {
    let effective_indent = format!("{base_indent}{}", row.indent.as_deref().unwrap_or_default());
    let bullet = styler.paint("-", StyleToken::Punctuation);
    let key = styler.paint(&display_key(row), StyleToken::Key);
    let child_indent = format!("{effective_indent}{}", " ".repeat(settings.indent_size));

    match &row.value {
        KeyValueValue::Empty => vec![format!("{effective_indent}{bullet} {key}")],
        KeyValueValue::Scalar(text) if text.is_empty() => {
            vec![format!("{effective_indent}{bullet} {key}")]
        }
        KeyValueValue::Scalar(text) => vec![format!(
            "{effective_indent}{bullet} {key}  {}",
            styler.paint_value(text)
        )],
        KeyValueValue::Object(rows) => {
            let mut lines = vec![format!(
                "{effective_indent}{bullet} {key}{}",
                styler.paint(":", StyleToken::Punctuation)
            )];
            if !rows.is_empty() {
                lines.push(emit_plain_rows(rows, &child_indent, settings, styler));
            }
            lines
        }
        KeyValueValue::Array(items) => {
            let mut lines = vec![format!(
                "{effective_indent}{bullet} {key}{}",
                styler.paint(":", StyleToken::Punctuation)
            )];
            lines.extend(items.iter().enumerate().flat_map(|(index, item)| {
                emit_plain_array_item_lines(item, index, &child_indent, settings, styler)
            }));
            lines
        }
    }
}

fn render_terminal_scalar_grid(
    values: &[&str],
    indent: &str,
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> Vec<String> {
    let visible = values
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    let available_width = settings
        .width
        .unwrap_or(100)
        .saturating_sub(settings.margin + UnicodeWidthStr::width(indent))
        .max(1);
    let grid = PreparedGridList::from_items(&visible, available_width);
    let mut lines = Vec::new();

    for row in &grid.rows {
        let mut line = indent.to_string();
        let mut first = true;
        for (column_index, cell) in row.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            if !first {
                line.push_str(&" ".repeat(grid.gap));
            }
            first = false;
            line.push_str(&styler.paint_value(cell));
            if column_index + 1 != grid.column_widths.len() {
                let pad = grid.column_widths[column_index]
                    .saturating_sub(UnicodeWidthStr::width(cell.as_str()));
                line.push_str(&" ".repeat(pad));
            }
        }
        lines.push(line);
    }

    lines
}

fn value_scalar_text(value: &KeyValueValue) -> Option<&str> {
    match value {
        KeyValueValue::Empty => Some(""),
        KeyValueValue::Scalar(text) => Some(text),
        KeyValueValue::Array(_) | KeyValueValue::Object(_) => None,
    }
}

fn value_requires_block(value: &KeyValueValue) -> bool {
    match value {
        KeyValueValue::Empty | KeyValueValue::Scalar(_) => false,
        KeyValueValue::Array(items) => {
            items.is_empty()
                || items.len() > 1
                || items
                    .iter()
                    .any(|item| !matches!(item, KeyValueValue::Empty | KeyValueValue::Scalar(_)))
        }
        KeyValueValue::Object(_) => true,
    }
}

fn array_uses_count(items: &[KeyValueValue]) -> bool {
    items.is_empty()
        || items.len() > 1
        || items
            .first()
            .is_some_and(|value| !matches!(value, KeyValueValue::Empty | KeyValueValue::Scalar(_)))
}
fn table_row(
    cells: &[PreparedCell],
    widths: &[usize],
    column_align: &[ColumnAlignment],
    vertical: char,
    styler: &ThemeStyler<'_>,
    header: bool,
) -> String {
    let mut out = String::new();
    let vertical = styler.paint(&vertical.to_string(), StyleToken::Border);
    out.push_str(&vertical);
    for (index, width) in widths.iter().enumerate() {
        out.push(' ');
        let cell = cells.get(index);
        let raw_cell = cell.map(|cell| cell.raw.as_str()).unwrap_or("");
        let raw_width = cell.map(|cell| cell.width).unwrap_or(0);
        let (left_pad, right_pad) = aligned_padding(
            width.saturating_sub(raw_width),
            column_align.get(index).copied(),
        );
        let styled_cell = if header {
            styler.paint(raw_cell, StyleToken::TableHeader)
        } else {
            styler.paint_value(raw_cell)
        };
        out.push_str(&" ".repeat(left_pad));
        out.push_str(&styled_cell);
        out.push_str(&" ".repeat(right_pad));
        out.push(' ');
        out.push_str(&vertical);
    }
    out
}

fn aligned_padding(pad: usize, alignment: Option<ColumnAlignment>) -> (usize, usize) {
    match alignment.unwrap_or(ColumnAlignment::Default) {
        ColumnAlignment::Default | ColumnAlignment::Left => (0, pad),
        ColumnAlignment::Right => (pad, 0),
        ColumnAlignment::Center => (pad / 2, pad - (pad / 2)),
    }
}

fn style_title_line(title: &RenderedTitle, styler: &ThemeStyler<'_>) -> String {
    let mut out = String::new();
    if !title.prefix.is_empty() {
        out.push_str(&styler.paint(&title.prefix, StyleToken::Border));
    }
    if !title.title.is_empty() {
        out.push_str(&styler.paint(&title.title, StyleToken::PanelTitle));
    }
    if !title.suffix.is_empty() {
        let token = if title.suffix == ":" {
            StyleToken::Punctuation
        } else {
            StyleToken::Border
        };
        out.push_str(&styler.paint(&title.suffix, token));
    }
    out
}

fn table_rule(widths: &[usize], left: char, join: char, right: char, horizontal: char) -> String {
    let mut out = String::new();
    out.push(left);
    for (index, width) in widths.iter().enumerate() {
        out.push_str(&horizontal.to_string().repeat(width + 2));
        if index + 1 == widths.len() {
            out.push(right);
        } else {
            out.push(join);
        }
    }
    out
}

struct TableBorderChars {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    join_top: char,
    join_mid: char,
    join_bottom: char,
    join_left: char,
    join_right: char,
    horizontal: char,
    vertical: char,
}

fn table_border_chars(unicode: bool, style: TableBorderStyle) -> TableBorderChars {
    match (unicode, style) {
        (_, TableBorderStyle::None) => TableBorderChars {
            top_left: ' ',
            top_right: ' ',
            bottom_left: ' ',
            bottom_right: ' ',
            join_top: ' ',
            join_mid: ' ',
            join_bottom: ' ',
            join_left: ' ',
            join_right: ' ',
            horizontal: ' ',
            vertical: ' ',
        },
        (true, TableBorderStyle::Round) => TableBorderChars {
            top_left: '╭',
            top_right: '╮',
            bottom_left: '╰',
            bottom_right: '╯',
            join_top: '┬',
            join_mid: '┼',
            join_bottom: '┴',
            join_left: '├',
            join_right: '┤',
            horizontal: '─',
            vertical: '│',
        },
        (true, TableBorderStyle::Square) => TableBorderChars {
            top_left: '┏',
            top_right: '┓',
            bottom_left: '┗',
            bottom_right: '┛',
            join_top: '┳',
            join_mid: '╇',
            join_bottom: '┻',
            join_left: '┣',
            join_right: '┫',
            horizontal: '━',
            vertical: '┃',
        },
        (false, _) => TableBorderChars {
            top_left: '+',
            top_right: '+',
            bottom_left: '+',
            bottom_right: '+',
            join_top: '+',
            join_mid: '+',
            join_bottom: '+',
            join_left: '+',
            join_right: '+',
            horizontal: '-',
            vertical: '|',
        },
    }
}
