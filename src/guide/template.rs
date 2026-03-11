//! Restricted markdown template parsing for semantic guide and intro authoring.
//!
//! The template surface is intentionally small:
//! - headings and paragraphs for prose
//! - `{{ help }}` / `{{ overview }}` placeholders for semantic includes
//! - fenced `osp` blocks for embedded JSON data that should flow through the
//!   normal document/data renderer instead of rendering as literal code
//!
//! Markdown list items are still treated as prose paragraphs here; they do not
//! become semantic lists. `osp` fences are the explicit data authoring path.
//!
//! Ordinary code fences remain literal paragraph/code content. Invalid `osp`
//! fences also fall back to literal content so author mistakes stay visible.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuideTemplateBlock {
    Heading(String),
    Paragraph(String),
    // `osp` fences are authoring syntax for semantic JSON payloads. They are
    // parsed here so later guide/intro code can lower the data through the
    // normal document pipeline instead of treating the fence as literal code.
    Data(Value),
    Include(GuideTemplateInclude),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuideTemplateInclude {
    Help,
    Overview,
}

/// Parses markdown-like template authoring into semantic guide blocks.
///
/// This stays intentionally small: headings and paragraphs are preserved as
/// author text, `{{ help }}` / `{{ overview }}` become semantic includes, and
/// fenced `osp` blocks are decoded as JSON data. Non-`osp` code blocks fall
/// back to literal paragraph lines so ordinary code fences still render as
/// prose/code content instead of disappearing. Markdown list items also stay as
/// paragraph text here; only `osp` fences author semantic list/table data.
pub(crate) fn parse_markdown_template(template: &str) -> Vec<GuideTemplateBlock> {
    let parser = Parser::new_ext(template, Options::all());
    let mut out = Vec::new();
    let mut active: Option<ActiveBlock> = None;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { .. } => active = Some(ActiveBlock::Heading(String::new())),
                Tag::Paragraph => active = Some(ActiveBlock::Paragraph(String::new())),
                Tag::Item => active = Some(ActiveBlock::Item(String::new())),
                Tag::CodeBlock(kind) => {
                    let language = match kind {
                        CodeBlockKind::Fenced(language) => Some(language.to_string()),
                        CodeBlockKind::Indented => None,
                    };
                    active = Some(ActiveBlock::CodeBlock {
                        language,
                        text: String::new(),
                    });
                }
                Tag::Emphasis => push_active_text(&mut active, "*"),
                Tag::Strong => push_active_text(&mut active, "**"),
                Tag::Strikethrough => push_active_text(&mut active, "~~"),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::Item | TagEnd::CodeBlock => {
                    flush_active_block(&mut out, active.take());
                }
                TagEnd::Emphasis => push_active_text(&mut active, "*"),
                TagEnd::Strong => push_active_text(&mut active, "**"),
                TagEnd::Strikethrough => push_active_text(&mut active, "~~"),
                _ => {}
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                push_active_text(&mut active, &text);
            }
            Event::Code(text) => {
                push_active_text(&mut active, "`");
                push_active_text(&mut active, &text);
                push_active_text(&mut active, "`");
            }
            Event::SoftBreak => push_active_text(&mut active, "\n"),
            Event::HardBreak => push_active_text(&mut active, "\n"),
            Event::Rule => {
                flush_active_block(&mut out, active.take());
            }
            _ => {}
        }
    }

    flush_active_block(&mut out, active.take());
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveBlock {
    Heading(String),
    Paragraph(String),
    Item(String),
    CodeBlock {
        language: Option<String>,
        text: String,
    },
}

fn push_active_text(active: &mut Option<ActiveBlock>, text: &str) {
    let Some(active) = active.as_mut() else {
        return;
    };
    match active {
        ActiveBlock::Heading(buf) | ActiveBlock::Paragraph(buf) | ActiveBlock::Item(buf) => {
            buf.push_str(text)
        }
        ActiveBlock::CodeBlock { text: buf, .. } => buf.push_str(text),
    }
}

fn flush_active_block(out: &mut Vec<GuideTemplateBlock>, active: Option<ActiveBlock>) {
    let Some(active) = active else {
        return;
    };

    match active {
        ActiveBlock::Heading(text) => {
            let title = text.trim();
            if !title.is_empty() {
                out.push(GuideTemplateBlock::Heading(title.to_string()));
            }
        }
        ActiveBlock::Paragraph(text) => push_text_block(out, &text, false),
        ActiveBlock::Item(text) => push_text_block(out, &text, true),
        ActiveBlock::CodeBlock { language, text } => {
            // `osp` fences are semantic data blocks. Invalid JSON falls back to
            // literal code-line paragraphs so broken authoring is still visible.
            if language.as_deref() == Some("osp")
                && let Ok(value) = serde_json::from_str::<Value>(&text)
            {
                out.push(GuideTemplateBlock::Data(value));
                return;
            }

            for line in text.lines() {
                let trimmed = line.trim_end();
                if !trimmed.is_empty() {
                    out.push(GuideTemplateBlock::Paragraph(format!("`{trimmed}`")));
                }
            }
        }
    }
}

fn push_text_block(out: &mut Vec<GuideTemplateBlock>, text: &str, item: bool) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    if let Some(include) = parse_include(trimmed) {
        out.push(GuideTemplateBlock::Include(include));
        return;
    }

    // Markdown items remain author prose here. They are prefixed so the later
    // guide renderer preserves the authored list look, but they do not become
    // semantic lists the way `osp` data blocks do.
    let text = if item {
        format!("- {trimmed}")
    } else {
        trimmed.to_string()
    };
    out.push(GuideTemplateBlock::Paragraph(text));
}

fn parse_include(text: &str) -> Option<GuideTemplateInclude> {
    match text {
        "{{ help }}" => Some(GuideTemplateInclude::Help),
        "{{ overview }}" => Some(GuideTemplateInclude::Overview),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
