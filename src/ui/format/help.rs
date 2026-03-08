use crate::ui::document::{Block, CodeBlock, Document, PanelBlock, PanelRules};
use crate::ui::inline::line_from_inline;
use crate::ui::style::StyleToken;

pub fn build_help_document(raw: &str, title: Option<&str>) -> Document {
    let sections = parse_sections(raw);
    if sections.is_empty() {
        return Document {
            blocks: vec![Block::Line(line_from_inline(raw.trim()))],
        };
    }

    let mut blocks = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        if section.lines.is_empty() {
            continue;
        }
        let panel_title = if index == 0 {
            title
                .map(|value| format!("{value} · {}", section.title))
                .unwrap_or_else(|| section.title.clone())
        } else {
            section.title.clone()
        };
        let body = section_lines_to_document(&section.lines);
        blocks.push(Block::Panel(PanelBlock {
            title: Some(panel_title),
            body,
            rules: PanelRules::Top,
            kind: Some("info".to_string()),
            border_token: Some(StyleToken::PanelBorder),
            title_token: Some(StyleToken::PanelTitle),
        }));
    }

    Document { blocks }
}

#[derive(Debug, Clone)]
struct HelpSection {
    title: String,
    lines: Vec<String>,
}

fn parse_sections(raw: &str) -> Vec<HelpSection> {
    let mut sections = Vec::new();
    let mut current = HelpSection {
        title: "Overview".to_string(),
        lines: Vec::new(),
    };

    for line in raw.lines() {
        let trimmed = line.trim_end();
        let title = parse_section_title(trimmed);
        if let Some(title) = title {
            if !current.lines.is_empty() {
                sections.push(current);
            }
            current = HelpSection {
                title,
                lines: Vec::new(),
            };
            continue;
        }
        current.lines.push(trimmed.to_string());
    }

    if !current.lines.is_empty() {
        sections.push(current);
    }

    sections
}

fn parse_section_title(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || !trimmed.ends_with(':') {
        return None;
    }
    if trimmed.starts_with('-') || trimmed.starts_with('*') {
        return None;
    }
    let head = trimmed.trim_end_matches(':').trim();
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

fn section_lines_to_document(lines: &[String]) -> Document {
    let mut blocks = Vec::new();
    let mut in_code = false;
    let mut code_lang: Option<String> = None;
    let mut code_lines: Vec<String> = Vec::new();

    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.trim_start().starts_with("```") {
            if in_code {
                blocks.push(Block::Code(CodeBlock {
                    code: code_lines.join("\n"),
                    language: code_lang.clone(),
                }));
                code_lines.clear();
                in_code = false;
                code_lang = None;
            } else {
                in_code = true;
                let language = trimmed
                    .trim_start()
                    .trim_start_matches("```")
                    .trim()
                    .to_string();
                if !language.is_empty() {
                    code_lang = Some(language);
                }
            }
            continue;
        }

        if in_code {
            code_lines.push(trimmed.to_string());
            continue;
        }

        if trimmed.trim().is_empty() {
            continue;
        }

        blocks.push(Block::Line(line_from_inline(trimmed)));
    }

    if in_code && !code_lines.is_empty() {
        blocks.push(Block::Code(CodeBlock {
            code: code_lines.join("\n"),
            language: code_lang,
        }));
    }

    Document { blocks }
}

#[cfg(test)]
mod tests {
    use super::build_help_document;
    use crate::ui::document::Block;

    #[test]
    fn builds_panel_sections_from_help_text() {
        let raw = "Summary:\nline one\n\nArguments:\n- a\n- b\n";
        let doc = build_help_document(raw, Some("osp ldap user"));
        assert_eq!(doc.blocks.len(), 2);
        let Block::Panel(summary) = &doc.blocks[0] else {
            panic!("expected panel");
        };
        assert!(
            summary
                .title
                .as_ref()
                .is_some_and(|value| value.contains("Summary"))
        );
    }

    #[test]
    fn keeps_fenced_code_as_code_block() {
        let raw = "Examples:\n```bash\nosp ldap user oistes\n```";
        let doc = build_help_document(raw, None);
        let Block::Panel(panel) = &doc.blocks[0] else {
            panic!("expected panel");
        };
        assert!(
            panel
                .body
                .blocks
                .iter()
                .any(|block| matches!(block, Block::Code(_)))
        );
    }
}
