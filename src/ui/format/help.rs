use crate::guide::{GuideSection, GuideSectionKind, GuideView};
use crate::ui::chrome::{RuledSectionPolicy, SectionFrameStyle};
use crate::ui::document::{Block, Document, LineBlock, LinePart};
use crate::ui::document_model::{
    BlockModel, DocumentModel, KeyValueBlockModel, KeyValueRowModel, LowerDocumentOptions,
    SectionModel,
};
use crate::ui::presentation::HelpLayout;
use crate::ui::resolution::ResolvedHelpChromeSettings;
use crate::ui::style::StyleToken;
use crate::ui::{ResolvedGuideRenderSettings, TableBorderStyle};
use unicode_width::UnicodeWidthStr;

pub(crate) struct GuideRenderOptions<'a> {
    pub(crate) title_prefix: Option<&'a str>,
    pub(crate) layout: HelpLayout,
    pub(crate) guide: ResolvedGuideRenderSettings,
    pub(crate) panel_kind: Option<&'a str>,
}

#[cfg(test)]
pub(crate) fn build_help_document(
    raw: &str,
    title_prefix: Option<&str>,
    layout: HelpLayout,
    frame_style: SectionFrameStyle,
    help_table_border: TableBorderStyle,
) -> Document {
    build_help_document_from_view(
        &GuideView::from_text(raw),
        title_prefix,
        layout,
        ResolvedGuideRenderSettings::plain_help(frame_style, help_table_border),
    )
}

#[cfg(test)]
pub(crate) fn build_help_document_from_view(
    view: &GuideView,
    title_prefix: Option<&str>,
    layout: HelpLayout,
    guide: ResolvedGuideRenderSettings,
) -> Document {
    build_guide_document_from_view(
        view,
        GuideRenderOptions {
            title_prefix,
            layout,
            guide,
            panel_kind: Some("help"),
        },
    )
}

pub(crate) fn build_guide_document_from_view(
    view: &GuideView,
    options: GuideRenderOptions<'_>,
) -> Document {
    if view_is_plain_lines(view) {
        return document_from_plain_lines(view.preamble.iter().map(String::as_str));
    }
    let view = apply_title_prefix(view, options.title_prefix);
    let mut model = DocumentModel::from_guide_view(&view);
    if uses_clapish_help_layout(options.layout) {
        return build_clap_like_document_from_model(&model, options.guide.help_chrome);
    }
    normalize_section_spacing(
        &mut model.blocks,
        separator_lines(options.layout, options.guide.help_chrome.section_spacing),
    );
    let mut next_block_id = 1_u64;
    model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: options.guide.frame_style,
            ruled_section_policy: options.guide.ruled_section_policy,
            panel_kind: options.panel_kind,
            key_value_border: options.guide.help_chrome.table_border,
            key_value_indent: options.guide.help_chrome.entry_indent,
            key_value_gap: options.guide.help_chrome.entry_gap,
        },
        &mut next_block_id,
    )
}

fn uses_clapish_help_layout(layout: HelpLayout) -> bool {
    matches!(layout, HelpLayout::Compact | HelpLayout::Minimal)
}

fn clap_section_spacing(help_chrome: ResolvedHelpChromeSettings) -> usize {
    help_chrome.section_spacing.unwrap_or(1)
}

fn build_clap_like_document_from_model(
    model: &DocumentModel,
    help_chrome: ResolvedHelpChromeSettings,
) -> Document {
    let mut blocks = Vec::new();
    append_clap_top_level_blocks(
        &mut blocks,
        &model.blocks,
        clap_section_spacing(help_chrome),
        help_chrome,
    );
    Document { blocks }
}

fn append_clap_top_level_blocks(
    out: &mut Vec<Block>,
    blocks: &[BlockModel],
    section_spacing: usize,
    help_chrome: ResolvedHelpChromeSettings,
) {
    let mut needs_spacing_before_next = false;

    for block in blocks {
        match block {
            BlockModel::Blank => {
                needs_spacing_before_next = !out.is_empty();
            }
            BlockModel::Section(section) => {
                if !out.is_empty() {
                    push_blank_lines(out, section_spacing);
                }
                append_clap_section(out, section, help_chrome);
                needs_spacing_before_next = true;
            }
            other => {
                if needs_spacing_before_next && !out.is_empty() {
                    push_blank_lines(out, section_spacing);
                }
                append_clap_body_block(out, other, help_chrome);
                needs_spacing_before_next = false;
            }
        }
    }
}

fn append_clap_section(
    out: &mut Vec<Block>,
    section: &SectionModel,
    help_chrome: ResolvedHelpChromeSettings,
) {
    if let Some(inline_usage) = inline_usage_block(section) {
        out.push(inline_usage);
        return;
    }

    if let Some(title) = section
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
    {
        out.push(styled_line(vec![LinePart {
            text: format!("{title}:"),
            token: Some(StyleToken::PanelTitle),
        }]));
    }

    let mut needs_blank = false;
    for block in &section.blocks {
        match block {
            BlockModel::Blank => {
                needs_blank = !out.is_empty();
            }
            other => {
                if needs_blank && !out.is_empty() {
                    push_blank_lines(out, 1);
                }
                append_clap_body_block(out, other, help_chrome);
                needs_blank = false;
            }
        }
    }
}

fn inline_usage_block(section: &SectionModel) -> Option<Block> {
    let title = section.title.as_deref()?;
    if title != "Usage" {
        return None;
    }

    let body = section
        .blocks
        .iter()
        .filter_map(|block| match block {
            BlockModel::Paragraph(text) if !text.trim().is_empty() => Some(text),
            BlockModel::Blank => None,
            _ => None,
        })
        .collect::<Vec<_>>();

    if body.len() != 1 {
        return None;
    }

    Some(styled_line(vec![
        LinePart {
            text: "Usage:".to_string(),
            token: Some(StyleToken::PanelTitle),
        },
        LinePart {
            text: format!(" {}", body[0].trim()),
            token: Some(StyleToken::Value),
        },
    ]))
}

fn append_clap_body_block(
    out: &mut Vec<Block>,
    block: &BlockModel,
    help_chrome: ResolvedHelpChromeSettings,
) {
    match block {
        BlockModel::Paragraph(text) => out.push(plain_line(text, StyleToken::Value)),
        BlockModel::KeyValue(rows) => {
            if rows.key_header.is_some() && rows.value_header.is_some() {
                append_help_entry_rows(out, rows, help_chrome);
            } else {
                append_key_value_rows(out, rows, help_chrome);
            }
        }
        BlockModel::Section(section) => append_clap_section(out, section, help_chrome),
        BlockModel::Table(_) | BlockModel::List(_) => {
            append_fallback_block_document(out, block.clone());
        }
        BlockModel::Blank => push_blank_lines(out, 1),
    }
}

fn append_help_entry_rows(
    out: &mut Vec<Block>,
    block: &KeyValueBlockModel,
    help_chrome: ResolvedHelpChromeSettings,
) {
    let key_width = block
        .rows
        .iter()
        .map(|row| UnicodeWidthStr::width(row.key.as_str()))
        .max()
        .unwrap_or(0);

    for row in &block.rows {
        out.push(help_entry_row(row, key_width, help_chrome));
    }
}

fn append_key_value_rows(
    out: &mut Vec<Block>,
    block: &KeyValueBlockModel,
    help_chrome: ResolvedHelpChromeSettings,
) {
    let key_width = block
        .rows
        .iter()
        .map(|row| UnicodeWidthStr::width(row.key.as_str()))
        .max()
        .unwrap_or(0);

    for row in &block.rows {
        out.push(key_value_row(row, key_width, help_chrome));
    }
}

fn help_entry_row(
    row: &KeyValueRowModel,
    key_width: usize,
    help_chrome: ResolvedHelpChromeSettings,
) -> Block {
    let indent = help_chrome
        .entry_indent
        .map(|value| " ".repeat(value))
        .or_else(|| row.indent.clone())
        .unwrap_or_else(|| "  ".to_string());
    let padding = key_width.saturating_sub(UnicodeWidthStr::width(row.key.as_str()));
    let gap = if let Some(value) = help_chrome.entry_gap {
        format!("{}{}", " ".repeat(padding), " ".repeat(value))
    } else {
        row.gap
            .clone()
            .unwrap_or_else(|| format!("{}  ", " ".repeat(padding)))
    };

    let mut parts = vec![
        LinePart {
            text: indent,
            token: None,
        },
        LinePart {
            text: row.key.clone(),
            token: Some(StyleToken::Key),
        },
    ];

    if !row.value.is_empty() {
        parts.push(LinePart {
            text: format!("{gap}{}", row.value),
            token: Some(StyleToken::Value),
        });
    }

    styled_line(parts)
}

fn key_value_row(
    row: &KeyValueRowModel,
    key_width: usize,
    help_chrome: ResolvedHelpChromeSettings,
) -> Block {
    let indent = help_chrome
        .entry_indent
        .map(|value| " ".repeat(value))
        .or_else(|| row.indent.clone())
        .unwrap_or_else(|| "  ".to_string());
    let padding = key_width.saturating_sub(UnicodeWidthStr::width(row.key.as_str()));

    let mut parts = vec![
        LinePart {
            text: indent,
            token: None,
        },
        LinePart {
            text: row.key.clone(),
            token: Some(StyleToken::Key),
        },
    ];

    let separator = if row.value.is_empty() {
        format!(":{}", " ".repeat(padding))
    } else if let Some(value) = help_chrome.entry_gap {
        format!(":{}{}", " ".repeat(padding), " ".repeat(value))
    } else {
        row.gap
            .clone()
            .map(|gap| format!(":{gap}"))
            .unwrap_or_else(|| format!(":{} ", " ".repeat(padding)))
    };
    let text = if row.value.is_empty() {
        separator
    } else {
        format!("{separator}{}", row.value)
    };
    parts.push(LinePart {
        text,
        token: Some(StyleToken::Value),
    });

    styled_line(parts)
}

fn append_fallback_block_document(out: &mut Vec<Block>, block: BlockModel) {
    let mut next_block_id = 1_u64;
    let document = DocumentModel {
        blocks: vec![block],
    }
    .lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::None,
            ruled_section_policy: RuledSectionPolicy::Shared,
            panel_kind: None,
            key_value_border: TableBorderStyle::None,
            key_value_indent: None,
            key_value_gap: None,
        },
        &mut next_block_id,
    );
    out.extend(document.blocks);
}

fn plain_line(text: &str, token: StyleToken) -> Block {
    styled_line(vec![LinePart {
        text: text.to_string(),
        token: Some(token),
    }])
}

fn styled_line(parts: Vec<LinePart>) -> Block {
    Block::Line(LineBlock { parts })
}

fn push_blank_lines(out: &mut Vec<Block>, count: usize) {
    for _ in 0..count {
        out.push(Block::Line(LineBlock { parts: Vec::new() }));
    }
}

fn apply_title_prefix(view: &GuideView, title_prefix: Option<&str>) -> GuideView {
    let Some(prefix) = title_prefix else {
        return view.clone();
    };
    let mut updated = view.clone();
    if let Some(first) = updated.sections.first_mut() {
        first.title = format!("{prefix} · {}", first.title);
    } else if !updated.usage.is_empty() {
        updated.sections.insert(
            0,
            GuideSection {
                title: format!("{prefix} · Usage"),
                kind: GuideSectionKind::Usage,
                paragraphs: updated.usage.clone(),
                entries: Vec::new(),
                data: None,
            },
        );
        updated.usage.clear();
    }
    updated
}

fn view_is_plain_lines(view: &GuideView) -> bool {
    view.sections.is_empty()
        && view.usage.is_empty()
        && view.commands.is_empty()
        && view.arguments.is_empty()
        && view.options.is_empty()
        && view.common_invocation_options.is_empty()
        && view.notes.is_empty()
}

fn separator_lines(layout: HelpLayout, override_count: Option<usize>) -> usize {
    override_count.unwrap_or(match layout {
        HelpLayout::Minimal => 1,
        HelpLayout::Full | HelpLayout::Compact => 1,
    })
}

fn normalize_section_spacing(blocks: &mut Vec<BlockModel>, spacing: usize) {
    let mut normalized = Vec::with_capacity(blocks.len());
    let mut blank_run = 0usize;

    for block in blocks.drain(..) {
        match block {
            BlockModel::Blank => {
                blank_run += 1;
            }
            BlockModel::Section(mut section) => {
                if !normalized.is_empty() {
                    normalized.extend(std::iter::repeat_n(BlockModel::Blank, spacing));
                }
                normalize_section_spacing(&mut section.blocks, spacing);
                normalized.push(BlockModel::Section(section));
                blank_run = 0;
            }
            other => {
                if blank_run > 0 {
                    normalized.extend(std::iter::repeat_n(BlockModel::Blank, blank_run));
                    blank_run = 0;
                }
                normalized.push(other);
            }
        }
    }

    if blank_run > 0 {
        normalized.extend(std::iter::repeat_n(BlockModel::Blank, blank_run));
    }

    *blocks = normalized;
}

fn document_from_plain_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Document {
    Document {
        blocks: lines
            .into_iter()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(|line| {
                Block::Line(LineBlock {
                    parts: vec![LinePart {
                        text: line.to_string(),
                        token: None,
                    }],
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{GuideRenderOptions, build_guide_document_from_view, build_help_document};
    use crate::guide::{GuideSection, GuideSectionKind, GuideView};
    use crate::ui::TableBorderStyle;
    use crate::ui::chrome::SectionFrameStyle;
    use crate::ui::document::Block;
    use crate::ui::presentation::HelpLayout;
    use crate::ui::{ResolvedGuideRenderSettings, ResolvedHelpChromeSettings};

    #[test]
    fn builds_panel_sections_from_help_text() {
        let raw = "Usage: osp ldap user\n\nCommands:\n  show  Display\n";
        let doc = build_help_document(
            raw,
            None,
            HelpLayout::Full,
            SectionFrameStyle::Top,
            TableBorderStyle::None,
        );
        assert_eq!(doc.blocks.len(), 3);
        let Block::Panel(summary) = &doc.blocks[0] else {
            panic!("expected panel");
        };
        assert_eq!(summary.title.as_deref(), Some("Usage"));
    }

    #[test]
    fn command_sections_render_as_help_tables_unit() {
        let raw = "Commands:\n  show  Display current value\n";
        let doc = build_help_document(
            raw,
            None,
            HelpLayout::Full,
            SectionFrameStyle::Top,
            TableBorderStyle::None,
        );
        let Block::Panel(panel) = &doc.blocks[0] else {
            panic!("expected panel");
        };
        let Block::Line(line) = &panel.body.blocks[0] else {
            panic!("expected line");
        };
        assert_eq!(
            line.parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["  ", "show", "  Display current value"]
        );
    }

    #[test]
    fn build_help_document_from_view_renders_epilogue_after_sections_unit() {
        let view = GuideView {
            usage: vec!["osp".to_string()],
            epilogue: vec!["tail".to_string()],
            ..GuideView::default()
        };

        let rendered = build_help_document(
            "Usage: osp",
            None,
            HelpLayout::Compact,
            SectionFrameStyle::Top,
            TableBorderStyle::None,
        );
        assert!(!rendered.blocks.is_empty());

        let rendered_view = build_guide_document_from_view(
            &view,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                guide: ResolvedGuideRenderSettings::plain_help(
                    SectionFrameStyle::Top,
                    TableBorderStyle::None,
                ),
                panel_kind: Some("help"),
            },
        );
        assert_eq!(rendered_view.blocks.len(), 3);
        let Block::Line(usage) = &rendered_view.blocks[0] else {
            panic!("expected usage line");
        };
        assert_eq!(
            usage
                .parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Usage:", " osp"]
        );
        let Block::Line(blank) = &rendered_view.blocks[1] else {
            panic!("expected blank line");
        };
        assert!(blank.parts.is_empty());
        let Block::Line(tail) = &rendered_view.blocks[2] else {
            panic!("expected epilogue line");
        };
        assert_eq!(
            tail.parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["tail"]
        );
    }

    #[test]
    fn build_guide_document_preserves_custom_panel_kind_unit() {
        let view = GuideView {
            sections: vec![GuideSection::new("OSP", GuideSectionKind::Custom).paragraph("hello")],
            ..GuideView::default()
        };

        let rendered = build_guide_document_from_view(
            &view,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Full,
                guide: ResolvedGuideRenderSettings::plain_help(
                    SectionFrameStyle::Top,
                    TableBorderStyle::None,
                ),
                panel_kind: Some("intro"),
            },
        );

        let Block::Panel(panel) = &rendered.blocks[0] else {
            panic!("expected panel");
        };
        assert_eq!(panel.kind.as_deref(), Some("intro"));
    }

    #[test]
    fn help_render_options_override_entry_indent_gap_and_section_spacing_unit() {
        let view = GuideView::from_text(
            "Commands:\n  show  Display current value\n\nOptions:\n  -h, --help  Print help\n",
        );

        let rendered = build_guide_document_from_view(
            &view,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                guide: ResolvedGuideRenderSettings {
                    frame_style: SectionFrameStyle::Top,
                    ruled_section_policy: crate::ui::chrome::RuledSectionPolicy::PerSection,
                    help_chrome: ResolvedHelpChromeSettings {
                        table_border: TableBorderStyle::None,
                        entry_indent: Some(4),
                        entry_gap: Some(3),
                        section_spacing: Some(0),
                    },
                },
                panel_kind: Some("help"),
            },
        );

        assert_eq!(rendered.blocks.len(), 4);
        let Block::Line(commands) = &rendered.blocks[0] else {
            panic!("expected commands heading");
        };
        assert_eq!(
            commands
                .parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Commands:"]
        );
        let Block::Line(line) = &rendered.blocks[1] else {
            panic!("expected command line");
        };
        assert_eq!(
            line.parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["    ", "show", "   Display current value"]
        );
        let Block::Line(options) = &rendered.blocks[2] else {
            panic!("expected options heading");
        };
        assert_eq!(
            options
                .parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["Options:"]
        );
        let Block::Line(option_line) = &rendered.blocks[3] else {
            panic!("expected option line");
        };
        assert_eq!(
            option_line
                .parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["    ", "-h, --help", "   Print help"]
        );
    }

    #[test]
    fn compact_custom_section_data_renders_as_key_value_rows_unit() {
        let view = GuideView {
            sections: vec![GuideSection::new("config", GuideSectionKind::Custom).data(
                serde_json::json!({
                    "status": "ok",
                    "known_profiles": "default",
                }),
            )],
            ..GuideView::default()
        };

        let rendered = build_guide_document_from_view(
            &view,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                guide: ResolvedGuideRenderSettings::plain_help(
                    SectionFrameStyle::Top,
                    TableBorderStyle::None,
                ),
                panel_kind: Some("guide"),
            },
        );

        assert_eq!(rendered.blocks.len(), 3);
        let Block::Line(title) = &rendered.blocks[0] else {
            panic!("expected section title");
        };
        assert_eq!(
            title
                .parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["config:"]
        );
        let Block::Line(first) = &rendered.blocks[1] else {
            panic!("expected key/value row");
        };
        let rendered_first = first
            .parts
            .iter()
            .map(|part| part.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered_first.starts_with("  status:"));
        assert!(rendered_first.trim_end().ends_with("ok"));
    }
}
