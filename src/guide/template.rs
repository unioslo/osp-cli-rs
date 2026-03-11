use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GuideTemplateBlock {
    Heading(String),
    Paragraph(String),
    Include(GuideTemplateInclude),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuideTemplateInclude {
    Help,
    Overview,
}

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
                Tag::CodeBlock(_) => active = Some(ActiveBlock::CodeBlock(String::new())),
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
    CodeBlock(String),
}

fn push_active_text(active: &mut Option<ActiveBlock>, text: &str) {
    let Some(active) = active.as_mut() else {
        return;
    };
    match active {
        ActiveBlock::Heading(buf)
        | ActiveBlock::Paragraph(buf)
        | ActiveBlock::Item(buf)
        | ActiveBlock::CodeBlock(buf) => buf.push_str(text),
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
        ActiveBlock::CodeBlock(text) => {
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
