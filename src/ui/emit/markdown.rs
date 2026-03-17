use crate::ui::doc::{
    Block, Doc, GuideEntriesBlock, JsonBlock, KeyValueBlock, KeyValueRow, KeyValueStyle, ListBlock,
    ParagraphBlock, SectionBlock, SectionTitleChrome, TableBlock,
};

use super::shared::format_list_item;
use super::shared::indent_lines;
use super::table::{PreparedCell, PreparedTable};

pub(super) fn emit_doc(doc: &Doc) -> String {
    let rendered = emit_blocks(&doc.blocks);
    if rendered.is_empty() || rendered.ends_with('\n') {
        rendered
    } else {
        format!("{rendered}\n")
    }
}

fn emit_blocks(blocks: &[Block]) -> String {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Blank => out.push(String::new()),
            Block::Rule => out.push("---".to_string()),
            Block::Paragraph(block) => out.push(emit_paragraph(block)),
            Block::Section(block) => out.push(emit_section(block)),
            Block::Table(block) => out.push(emit_table(block)),
            Block::GuideEntries(block) => out.push(emit_guide_entries(block)),
            Block::KeyValue(block) => out.push(emit_key_value(block)),
            Block::List(block) => out.push(emit_list(block)),
            Block::Json(JsonBlock { text }) => out.push(format!("```json\n{text}\n```")),
        }
    }
    out.join("\n")
}

fn emit_paragraph(block: &ParagraphBlock) -> String {
    indent_lines(&block.text, block.indent)
}

fn emit_section(block: &SectionBlock) -> String {
    let mut out = String::new();
    if let Some(title) = block.title.as_deref() {
        match block.title_chrome {
            SectionTitleChrome::Plain => {
                out.push_str(title.trim_end_matches(':'));
                out.push(':');
            }
            SectionTitleChrome::Ruled => {
                out.push_str("## ");
                out.push_str(title.trim_end_matches(':'));
            }
        }
        if let Some(suffix) = block.inline_title_suffix.as_deref() {
            out.push(' ');
            out.push_str(suffix);
        }
        out.push_str("\n\n");
    }
    out.push_str(&emit_blocks(&block.blocks));
    out.trim_end().to_string()
}

fn emit_key_value(block: &KeyValueBlock) -> String {
    emit_rows(block.style, &block.rows)
}

fn emit_guide_entries(block: &GuideEntriesBlock) -> String {
    block
        .rows
        .iter()
        .map(|row| {
            if row.value.is_empty() {
                format!("- `{}`", row.key)
            } else {
                format!("- `{}` {}", row.key, row.value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_rows(style: KeyValueStyle, rows: &[KeyValueRow]) -> String {
    let mut lines = Vec::new();
    for row in rows {
        let line = match style {
            KeyValueStyle::Bulleted => {
                if row.value.is_empty() {
                    format!("- `{}`", row.key)
                } else {
                    format!("- `{}` {}", row.key, row.value)
                }
            }
            KeyValueStyle::Plain => {
                if row.value.is_empty() {
                    format!("- {}:", row.key)
                } else {
                    format!("- {}: {}", row.key, row.value)
                }
            }
        };
        lines.push(line);
    }
    lines.join("\n")
}

fn emit_list(block: &ListBlock) -> String {
    block
        .items
        .iter()
        .map(|item| format!("- {}", format_list_item(item, block.inline_markup)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn emit_table(block: &TableBlock) -> String {
    if block.headers.is_empty() {
        return String::new();
    }
    let table = PreparedTable::for_markdown(block);

    let mut lines = Vec::new();
    if !block.summary.is_empty() {
        lines.push(
            block
                .summary
                .iter()
                .map(|row| format!("- {}: {}", row.key, row.value))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        lines.push(String::new());
    }
    lines.push(markdown_row(&table.headers, &table.widths));
    lines.push(markdown_separator(&table.widths));
    for row in &table.rows {
        lines.push(markdown_row(row, &table.widths));
    }
    lines.join("\n")
}

fn markdown_row(cells: &[PreparedCell], widths: &[usize]) -> String {
    let mut out = String::from("|");
    for (index, width) in widths.iter().enumerate() {
        let cell = cells.get(index);
        out.push(' ');
        out.push_str(cell.map(|cell| cell.markdown.as_str()).unwrap_or(""));
        let pad = width.saturating_sub(cell.map(|cell| cell.width).unwrap_or(0));
        out.push_str(&" ".repeat(pad));
        out.push(' ');
        out.push('|');
    }
    out
}

fn markdown_separator(widths: &[usize]) -> String {
    let mut out = String::from("|");
    for width in widths {
        out.push(' ');
        out.push_str(&"-".repeat((*width).max(3)));
        out.push(' ');
        out.push('|');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{emit_doc, emit_table};
    use crate::ui::doc::{
        Block, Doc, GuideEntriesBlock, GuideEntryRow, KeyValueRow, ParagraphBlock, SectionBlock,
        SectionTitleChrome, TableBlock,
    };

    #[test]
    fn markdown_emitter_renders_sections_with_suffixes_and_trailing_newline_unit() {
        let doc = Doc {
            blocks: vec![
                Block::Section(SectionBlock {
                    title: Some("Usage".to_string()),
                    title_chrome: SectionTitleChrome::Ruled,
                    body_indent: 0,
                    inline_title_suffix: Some("osp history <COMMAND>".to_string()),
                    trailing_newline: false,
                    blocks: vec![Block::Paragraph(ParagraphBlock {
                        text: "Run `osp history list`".to_string(),
                        indent: 2,
                        inline_markup: true,
                    })],
                }),
                Block::Blank,
                Block::Section(SectionBlock {
                    title: Some("Commands".to_string()),
                    title_chrome: SectionTitleChrome::Plain,
                    body_indent: 0,
                    inline_title_suffix: None,
                    trailing_newline: false,
                    blocks: vec![Block::GuideEntries(GuideEntriesBlock {
                        default_indent: "  ".to_string(),
                        default_gap: None,
                        rows: vec![GuideEntryRow {
                            key: "list".to_string(),
                            value: "List history entries".to_string(),
                            indent_hint: None,
                            gap_hint: None,
                        }],
                    })],
                }),
            ],
        };

        let rendered = emit_doc(&doc);

        assert!(rendered.starts_with("## Usage osp history <COMMAND>\n\n  Run `osp history list`"));
        assert!(rendered.contains("\n\nCommands:\n\n- `list` List history entries"));
        assert!(rendered.ends_with('\n'));
    }

    #[test]
    fn markdown_emitter_handles_empty_and_summary_tables_unit() {
        let empty = TableBlock {
            summary: Vec::new(),
            headers: Vec::new(),
            rows: Vec::new(),
        };
        assert_eq!(emit_table(&empty), "");

        let table = TableBlock {
            summary: vec![KeyValueRow {
                key: "team".to_string(),
                value: "prod".to_string(),
                indent: None,
                gap: None,
            }],
            headers: vec!["uid".to_string(), "mail".to_string()],
            rows: vec![
                vec!["alice".to_string(), "a@example.com".to_string()],
                vec!["bob".to_string(), "".to_string()],
            ],
        };

        let rendered = emit_table(&table);

        assert!(rendered.contains("- team: prod"));
        assert!(rendered.contains("| uid"));
        assert!(rendered.contains("| ---"));
        assert!(rendered.contains("| bob   |"));
    }
}
