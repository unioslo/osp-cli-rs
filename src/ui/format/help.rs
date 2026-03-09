use crate::guide::{GuideBlock, GuideDoc, GuideEntry, GuideSection, GuideSectionKind};
use crate::ui::chrome::SectionFrameStyle;
use crate::ui::document::{Block, Document, LineBlock, LinePart, PanelBlock, PanelRules};
use crate::ui::inline::parts_from_inline;
use crate::ui::presentation::HelpLayout;
use crate::ui::style::StyleToken;

pub(crate) struct GuideRenderOptions<'a> {
    pub(crate) title_prefix: Option<&'a str>,
    pub(crate) layout: HelpLayout,
    pub(crate) frame_style: SectionFrameStyle,
    pub(crate) panel_kind: Option<&'a str>,
}

#[cfg(test)]
pub(crate) fn build_help_document(
    raw: &str,
    title_prefix: Option<&str>,
    layout: HelpLayout,
    frame_style: SectionFrameStyle,
) -> Document {
    build_help_document_from_doc(&GuideDoc::from_text(raw), title_prefix, layout, frame_style)
}

#[cfg(test)]
pub(crate) fn build_help_document_from_doc(
    doc: &GuideDoc,
    title_prefix: Option<&str>,
    layout: HelpLayout,
    frame_style: SectionFrameStyle,
) -> Document {
    build_guide_document_from_doc(
        doc,
        GuideRenderOptions {
            title_prefix,
            layout,
            frame_style,
            panel_kind: Some("help"),
        },
    )
}

pub(crate) fn build_guide_document_from_doc(
    doc: &GuideDoc,
    options: GuideRenderOptions<'_>,
) -> Document {
    if doc.sections.is_empty() {
        return document_from_plain_lines(doc.preamble.iter().map(String::as_str));
    }

    let mut blocks = Vec::new();

    if !doc.preamble.is_empty() {
        blocks.extend(document_from_plain_lines(doc.preamble.iter().map(String::as_str)).blocks);
        append_spacing(&mut blocks, separator_lines(options.layout));
    }

    for (index, section) in doc.sections.iter().enumerate() {
        if index > 0 {
            append_spacing(&mut blocks, separator_lines(options.layout));
        }

        let title = if index == 0 {
            options
                .title_prefix
                .map(|value| format!("{value} · {}", section.title))
                .unwrap_or_else(|| section.title.clone())
        } else {
            section.title.clone()
        };

        blocks.push(Block::Panel(PanelBlock {
            title: Some(title),
            body: build_help_section_body(section, options.layout),
            rules: PanelRules::None,
            frame_style: Some(options.frame_style),
            kind: options.panel_kind.map(str::to_string),
            border_token: Some(StyleToken::PanelBorder),
            title_token: Some(StyleToken::PanelTitle),
        }));
    }

    if !doc.epilogue.is_empty() {
        append_spacing(&mut blocks, separator_lines(options.layout));
        blocks.extend(document_from_plain_lines(doc.epilogue.iter().map(String::as_str)).blocks);
    }

    Document { blocks }
}

fn build_help_section_body(section: &GuideSection, layout: HelpLayout) -> Document {
    let normalized = normalize_help_blocks(&section.blocks, layout);
    let mut blocks = Vec::new();

    for block in normalized {
        match block {
            GuideBlock::Blank => blocks.push(blank_line_block()),
            GuideBlock::Paragraph { text } => {
                blocks.push(Block::Line(help_paragraph_line(section.kind, &text)));
            }
            GuideBlock::Entry(entry) => {
                blocks.push(Block::Line(help_entry_line(section.kind, &entry)));
            }
        }
    }

    Document { blocks }
}

fn help_paragraph_line(kind: GuideSectionKind, text: &str) -> LineBlock {
    let fallback = match kind {
        GuideSectionKind::Usage
        | GuideSectionKind::Notes
        | GuideSectionKind::Custom
        | GuideSectionKind::CommonInvocationOptions => Some(StyleToken::Value),
        GuideSectionKind::Commands | GuideSectionKind::Options | GuideSectionKind::Arguments => {
            Some(StyleToken::Value)
        }
    };
    guide_text_line(text, fallback)
}

fn help_entry_line(kind: GuideSectionKind, entry: &GuideEntry) -> LineBlock {
    if matches!(
        kind,
        GuideSectionKind::Commands
            | GuideSectionKind::Options
            | GuideSectionKind::Arguments
            | GuideSectionKind::CommonInvocationOptions
    ) {
        let mut parts = Vec::new();
        if !entry.indent.is_empty() {
            parts.push(LinePart {
                text: entry.indent.clone(),
                token: None,
            });
        }
        if !entry.head.is_empty() {
            parts.push(LinePart {
                text: entry.head.clone(),
                token: Some(StyleToken::Key),
            });
        }
        if !entry.tail.is_empty() {
            parts.push(LinePart {
                text: entry.tail.clone(),
                token: Some(StyleToken::Value),
            });
        }
        return LineBlock { parts };
    }

    guide_text_line(
        &format!("{}{}{}", entry.indent, entry.head, entry.tail),
        Some(StyleToken::Value),
    )
}

fn guide_text_line(text: &str, fallback: Option<StyleToken>) -> LineBlock {
    let parts = parts_from_inline(text)
        .into_iter()
        .map(|part| LinePart {
            token: part.token.or(fallback),
            ..part
        })
        .collect();
    LineBlock { parts }
}

fn normalize_help_blocks(blocks: &[GuideBlock], layout: HelpLayout) -> Vec<GuideBlock> {
    let blocks = trim_blank_blocks(blocks);
    match layout {
        HelpLayout::Full => blocks,
        HelpLayout::Compact => collapse_blank_blocks(&blocks, true),
        HelpLayout::Minimal => collapse_blank_blocks(&blocks, false),
    }
}

fn trim_blank_blocks(blocks: &[GuideBlock]) -> Vec<GuideBlock> {
    let mut start = 0usize;
    let mut end = blocks.len();

    while start < end && matches!(blocks[start], GuideBlock::Blank) {
        start += 1;
    }
    while end > start && matches!(blocks[end - 1], GuideBlock::Blank) {
        end -= 1;
    }

    blocks[start..end].to_vec()
}

fn collapse_blank_blocks(blocks: &[GuideBlock], keep_single_blank: bool) -> Vec<GuideBlock> {
    let mut out = Vec::new();
    let mut last_blank = false;

    for block in blocks {
        let is_blank = matches!(block, GuideBlock::Blank);
        if is_blank {
            if keep_single_blank && !last_blank {
                out.push(GuideBlock::Blank);
            }
            last_blank = true;
            continue;
        }
        out.push(block.clone());
        last_blank = false;
    }

    trim_blank_blocks(&out)
}

fn separator_lines(layout: HelpLayout) -> usize {
    match layout {
        HelpLayout::Minimal => 1,
        HelpLayout::Full | HelpLayout::Compact => 2,
    }
}

fn append_spacing(blocks: &mut Vec<Block>, count: usize) {
    for _ in 0..count {
        blocks.push(blank_line_block());
    }
}

fn blank_line_block() -> Block {
    Block::Line(LineBlock { parts: Vec::new() })
}

fn document_from_plain_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Document {
    Document {
        blocks: lines
            .into_iter()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(|line| Block::Line(guide_text_line(line, None)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GuideRenderOptions, build_guide_document_from_doc, build_help_document,
        build_help_document_from_doc,
    };
    use crate::guide::{GuideDoc, GuideSection, GuideSectionKind};
    use crate::ui::chrome::SectionFrameStyle;
    use crate::ui::document::Block;
    use crate::ui::presentation::HelpLayout;

    #[test]
    fn builds_panel_sections_from_help_text() {
        let raw = "Usage: osp ldap user\n\nCommands:\n  show  Display\n";
        let doc = build_help_document(raw, None, HelpLayout::Full, SectionFrameStyle::Top);
        assert_eq!(doc.blocks.len(), 4);
        let Block::Panel(summary) = &doc.blocks[0] else {
            panic!("expected panel");
        };
        assert_eq!(summary.title.as_deref(), Some("Usage"));
    }

    #[test]
    fn keyed_help_lines_keep_head_and_tail_tokens() {
        let raw = "Commands:\n  show  Display current value\n";
        let doc = build_help_document(raw, None, HelpLayout::Full, SectionFrameStyle::Top);
        let Block::Panel(panel) = &doc.blocks[0] else {
            panic!("expected panel");
        };
        let Block::Line(line) = &panel.body.blocks[0] else {
            panic!("expected line");
        };
        assert_eq!(line.parts.len(), 3);
        assert_eq!(line.parts[1].token, Some(crate::ui::style::StyleToken::Key));
        assert_eq!(
            line.parts[2].token,
            Some(crate::ui::style::StyleToken::Value)
        );
    }

    #[test]
    fn build_help_document_from_doc_renders_epilogue_after_sections_unit() {
        let doc = GuideDoc {
            preamble: Vec::new(),
            sections: vec![GuideSection::new("Usage", GuideSectionKind::Usage).paragraph("  osp")],
            epilogue: vec!["tail".to_string()],
        };

        let rendered =
            build_help_document_from_doc(&doc, None, HelpLayout::Compact, SectionFrameStyle::Top);
        assert_eq!(rendered.blocks.len(), 4);
    }

    #[test]
    fn build_guide_document_preserves_custom_panel_kind_unit() {
        let doc = GuideDoc {
            preamble: Vec::new(),
            sections: vec![GuideSection::new("OSP", GuideSectionKind::Custom).paragraph("  hello")],
            epilogue: Vec::new(),
        };

        let rendered = build_guide_document_from_doc(
            &doc,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: SectionFrameStyle::Top,
                panel_kind: Some("intro"),
            },
        );

        let Block::Panel(panel) = &rendered.blocks[0] else {
            panic!("expected panel");
        };
        assert_eq!(panel.kind.as_deref(), Some("intro"));
    }
}
