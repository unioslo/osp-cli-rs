use crate::guide::{GuideSection, GuideSectionKind, GuideView};
use crate::ui::ResolvedGuideRenderSettings;
#[cfg(test)]
use crate::ui::TableBorderStyle;
#[cfg(test)]
use crate::ui::chrome::SectionFrameStyle;
use crate::ui::document::{Block, Document, LineBlock, LinePart};
use crate::ui::document_model::{BlockModel, DocumentModel, LowerDocumentOptions};
use crate::ui::presentation::HelpLayout;

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
    normalize_section_spacing(
        &mut model.blocks,
        separator_lines(options.layout, options.guide.help_chrome.section_spacing),
    );
    let mut next_block_id = 1_u64;
    model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: options.guide.frame_style,
            panel_kind: options.panel_kind,
            key_value_border: options.guide.help_chrome.table_border,
            key_value_indent: options.guide.help_chrome.entry_indent,
            key_value_gap: options.guide.help_chrome.entry_gap,
        },
        &mut next_block_id,
    )
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
        HelpLayout::Full | HelpLayout::Compact => 2,
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
        assert_eq!(doc.blocks.len(), 4);
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
        assert_eq!(rendered_view.blocks.len(), 4);
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
                layout: HelpLayout::Compact,
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

        assert_eq!(rendered.blocks.len(), 2);
        let Block::Panel(commands) = &rendered.blocks[0] else {
            panic!("expected commands panel");
        };
        let Block::Line(line) = &commands.body.blocks[0] else {
            panic!("expected command line");
        };
        assert_eq!(
            line.parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>(),
            vec!["    ", "show", "   Display current value"]
        );
    }
}
