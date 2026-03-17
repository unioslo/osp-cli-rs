use unicode_width::UnicodeWidthStr;

use crate::ui::chrome::{
    FULL_HELP_LAYOUT_CHROME, GUIDE_SECTION_CHROME, PLAIN_SECTION_CHROME, RenderedTitle,
};
use crate::ui::doc::{
    Block, Doc, GuideEntriesBlock, KeyValueBlock, KeyValueRow, ListBlock, ParagraphBlock,
    SectionBlock, SectionTitleChrome, TableBlock,
};
use crate::ui::settings::{RenderBackend, ResolvedRenderSettings, TableBorderStyle};
use crate::ui::style::{StyleToken, ThemeStyler};
use crate::ui::visible_inline_text;

use super::grid::PreparedGridList;
use super::guide_entries::{PreparedGuideEntriesBlock, PreparedGuideEntryRow};
use super::key_value::{PreparedBulletedRow, PreparedKeyValueBlock, PreparedPlainRow};
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
    let mut lines = Vec::new();
    match PreparedKeyValueBlock::from_block(block) {
        PreparedKeyValueBlock::Plain(rows) => {
            for row in rows {
                lines.push(indent_lines(
                    &emit_plain_row(&row, &styler),
                    settings.margin,
                ));
            }
        }
        PreparedKeyValueBlock::Bulleted(rows) => {
            for row in rows {
                lines.push(indent_lines(
                    &emit_bulleted_row(&row, &styler),
                    settings.margin,
                ));
            }
        }
    }
    lines.join("\n")
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

fn emit_plain_row(row: &PreparedPlainRow, styler: &ThemeStyler<'_>) -> String {
    let key = styler.paint(&row.key, StyleToken::Key);
    if row.value.is_empty() {
        format!(
            "{}{}{}",
            row.indent,
            key,
            styler.paint(":", StyleToken::Punctuation)
        )
    } else {
        let value = styler.paint_value(&row.value);
        format!(
            "{}{}{}{}{}",
            row.indent,
            key,
            styler.paint(":", StyleToken::Punctuation),
            row.value_spacing,
            value
        )
    }
}

fn emit_bulleted_row(row: &PreparedBulletedRow, styler: &ThemeStyler<'_>) -> String {
    let key = styler.paint(&row.key, StyleToken::Key);
    if row.value.is_empty() {
        format!("{} {key}", styler.paint("-", StyleToken::Punctuation))
    } else {
        let value = styler.paint_value(&row.value);
        format!(
            "{} {key}  {value}",
            styler.paint("-", StyleToken::Punctuation)
        )
    }
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
                &table.widths,
                border.top_left,
                border.join_top,
                border.top_right,
                border.horizontal,
            ),
            StyleToken::Border,
        ),
        settings.margin,
    ));
    lines.push(indent_lines(
        &table_row(
            &table.headers,
            &table.widths,
            border.vertical,
            &styler,
            true,
        ),
        settings.margin,
    ));
    lines.push(indent_lines(
        &styler.paint(
            &table_rule(
                &table.widths,
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
        lines.push(indent_lines(
            &table_row(row, &table.widths, border.vertical, &styler, false),
            settings.margin,
        ));
    }
    lines.push(indent_lines(
        &styler.paint(
            &table_rule(
                &table.widths,
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

fn format_summary(
    rows: &[KeyValueRow],
    settings: &ResolvedRenderSettings,
    styler: &ThemeStyler<'_>,
) -> String {
    let sep = if settings.unicode { "  ·  " } else { "  |  " };
    let sep = styler.paint(sep, StyleToken::Punctuation);
    rows.iter()
        .map(|row| {
            format!(
                "{}{} {}",
                styler.paint(&row.key, StyleToken::Key),
                styler.paint(":", StyleToken::Punctuation),
                styler.paint_value(&row.value)
            )
        })
        .collect::<Vec<_>>()
        .join(&sep)
}
fn table_row(
    cells: &[PreparedCell],
    widths: &[usize],
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
        let styled_cell = if header {
            styler.paint(raw_cell, StyleToken::TableHeader)
        } else {
            styler.paint_value(raw_cell)
        };
        out.push_str(&styled_cell);
        let pad = width.saturating_sub(cell.map(|cell| cell.width).unwrap_or(0));
        out.push_str(&" ".repeat(pad));
        out.push(' ');
        out.push_str(&vertical);
    }
    out
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
