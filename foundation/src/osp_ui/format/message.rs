use crate::osp_ui::document::{
    Block, CodeBlock, Document, JsonBlock, LineBlock, LinePart, PanelBlock, PanelRules,
};
use crate::osp_ui::style::StyleToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Error,
    Warning,
    Success,
    Info,
    Trace,
}

impl MessageKind {
    pub fn as_label(self) -> &'static str {
        match self {
            MessageKind::Error => "error",
            MessageKind::Warning => "warning",
            MessageKind::Success => "success",
            MessageKind::Info => "info",
            MessageKind::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRules {
    None,
    Top,
    Bottom,
    Both,
}

impl MessageRules {
    fn to_panel_rules(self) -> PanelRules {
        match self {
            MessageRules::None => PanelRules::None,
            MessageRules::Top => PanelRules::Top,
            MessageRules::Bottom => PanelRules::Bottom,
            MessageRules::Both => PanelRules::Both,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessageOptions {
    pub rules: MessageRules,
    pub kind: MessageKind,
    pub title: Option<String>,
}

impl Default for MessageOptions {
    fn default() -> Self {
        Self {
            rules: MessageRules::Both,
            kind: MessageKind::Info,
            title: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MessageContent {
    Text(String),
    Json(serde_json::Value),
    Document(Document),
    Code {
        code: String,
        language: Option<String>,
    },
}

impl From<&str> for MessageContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for MessageContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<serde_json::Value> for MessageContent {
    fn from(value: serde_json::Value) -> Self {
        Self::Json(value)
    }
}

impl From<Document> for MessageContent {
    fn from(value: Document) -> Self {
        Self::Document(value)
    }
}

pub struct MessageFormatter;

impl MessageFormatter {
    pub fn build(content: impl Into<MessageContent>, options: MessageOptions) -> Document {
        let content = content.into();

        if matches!(options.rules, MessageRules::None) {
            return build_flat_message(content, &options);
        }

        let title = options
            .title
            .as_ref()
            .cloned()
            .or_else(|| Some(options.kind.as_label().to_uppercase()));
        let body = normalize_content_to_document(content);
        Document {
            blocks: vec![Block::Panel(PanelBlock {
                title,
                body,
                rules: options.rules.to_panel_rules(),
                kind: Some(options.kind.as_label().to_string()),
                border_token: Some(kind_border_token(options.kind)),
                title_token: Some(kind_title_token(options.kind)),
            })],
        }
    }
}

fn build_flat_message(content: MessageContent, options: &MessageOptions) -> Document {
    match content {
        MessageContent::Json(value) => Document {
            blocks: vec![Block::Json(JsonBlock { payload: value })],
        },
        MessageContent::Document(document) => document,
        MessageContent::Code { code, language } => Document {
            blocks: vec![Block::Code(CodeBlock { code, language })],
        },
        MessageContent::Text(text) => {
            let mut output = Vec::new();
            for (index, line) in trim_blank_lines(text.lines()).into_iter().enumerate() {
                let text = if index == 0 {
                    if let Some(title) = options.title.as_deref() {
                        format!("{}: {line}", title.to_uppercase())
                    } else {
                        line.to_string()
                    }
                } else {
                    line.to_string()
                };

                output.push(Block::Line(LineBlock {
                    parts: vec![LinePart { text, token: None }],
                }));
            }
            Document { blocks: output }
        }
    }
}

fn normalize_content_to_document(content: MessageContent) -> Document {
    match content {
        MessageContent::Json(value) => Document {
            blocks: vec![Block::Json(JsonBlock { payload: value })],
        },
        MessageContent::Document(document) => document,
        MessageContent::Code { code, language } => Document {
            blocks: vec![Block::Code(CodeBlock { code, language })],
        },
        MessageContent::Text(text) => Document {
            blocks: trim_blank_lines(text.lines())
                .into_iter()
                .map(|line| {
                    Block::Line(LineBlock {
                        parts: vec![LinePart {
                            text: line.to_string(),
                            token: None,
                        }],
                    })
                })
                .collect(),
        },
    }
}

fn trim_blank_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut values = lines.into_iter().collect::<Vec<&str>>();
    while values.first().is_some_and(|line| line.trim().is_empty()) {
        values.remove(0);
    }
    while values.last().is_some_and(|line| line.trim().is_empty()) {
        values.pop();
    }
    values
}

fn kind_border_token(kind: MessageKind) -> StyleToken {
    match kind {
        MessageKind::Error => StyleToken::MessageError,
        MessageKind::Warning => StyleToken::MessageWarning,
        MessageKind::Success => StyleToken::MessageSuccess,
        MessageKind::Info => StyleToken::MessageInfo,
        MessageKind::Trace => StyleToken::MessageTrace,
    }
}

fn kind_title_token(kind: MessageKind) -> StyleToken {
    kind_border_token(kind)
}

#[cfg(test)]
mod tests {
    use super::{MessageContent, MessageFormatter, MessageKind, MessageOptions, MessageRules};
    use crate::osp_ui::document::{Block, Document, LineBlock, LinePart, PanelRules};
    use crate::osp_ui::style::StyleToken;
    use serde_json::json;

    #[test]
    fn message_rules_none_yields_lines_without_panel() {
        let doc = MessageFormatter::build(
            "hello\nworld",
            MessageOptions {
                rules: MessageRules::None,
                kind: MessageKind::Info,
                title: Some("info".to_string()),
            },
        );

        assert!(matches!(doc.blocks[0], Block::Line(_)));
        assert!(matches!(doc.blocks[1], Block::Line(_)));
    }

    #[test]
    fn message_rules_both_wrap_in_panel() {
        let doc = MessageFormatter::build(
            MessageContent::Json(serde_json::json!({"ok": true})),
            MessageOptions::default(),
        );

        assert!(matches!(doc.blocks[0], Block::Panel(_)));
    }

    #[test]
    fn flat_text_trims_blank_lines_and_uses_uppercase_title_prefix() {
        let doc = MessageFormatter::build(
            "\n\nhello\nworld\n\n",
            MessageOptions {
                rules: MessageRules::None,
                kind: MessageKind::Warning,
                title: Some("warning".to_string()),
            },
        );

        let Block::Line(first) = &doc.blocks[0] else {
            panic!("expected first block to be a line");
        };
        assert_eq!(first.parts[0].text, "WARNING: hello");
        let Block::Line(second) = &doc.blocks[1] else {
            panic!("expected second block to be a line");
        };
        assert_eq!(second.parts[0].text, "world");
    }

    #[test]
    fn flat_message_preserves_json_code_and_document_blocks() {
        let json_doc = MessageFormatter::build(
            MessageContent::Json(json!({"ok": true})),
            MessageOptions {
                rules: MessageRules::None,
                kind: MessageKind::Info,
                title: None,
            },
        );
        assert!(matches!(json_doc.blocks[0], Block::Json(_)));

        let code_doc = MessageFormatter::build(
            MessageContent::Code {
                code: "ldap user oistes".to_string(),
                language: Some("bash".to_string()),
            },
            MessageOptions {
                rules: MessageRules::None,
                kind: MessageKind::Trace,
                title: None,
            },
        );
        assert!(matches!(code_doc.blocks[0], Block::Code(_)));

        let inner = Document {
            blocks: vec![Block::Line(LineBlock {
                parts: vec![LinePart {
                    text: "nested".to_string(),
                    token: None,
                }],
            })],
        };
        let nested = MessageFormatter::build(
            MessageContent::Document(inner.clone()),
            MessageOptions {
                rules: MessageRules::None,
                kind: MessageKind::Info,
                title: None,
            },
        );
        assert_eq!(nested.blocks.len(), inner.blocks.len());
    }

    #[test]
    fn panel_message_uses_kind_tokens_and_default_title() {
        let doc = MessageFormatter::build(
            "failed",
            MessageOptions {
                rules: MessageRules::Top,
                kind: MessageKind::Error,
                title: None,
            },
        );

        let Block::Panel(panel) = &doc.blocks[0] else {
            panic!("expected panel block");
        };
        assert_eq!(panel.title.as_deref(), Some("ERROR"));
        assert_eq!(panel.kind.as_deref(), Some("error"));
        assert_eq!(panel.border_token, Some(StyleToken::MessageError));
        assert_eq!(panel.title_token, Some(StyleToken::MessageError));
    }

    #[test]
    fn message_kind_labels_and_rule_mapping_cover_all_variants() {
        assert_eq!(MessageKind::Success.as_label(), "success");
        assert_eq!(MessageKind::Trace.as_label(), "trace");

        let top = MessageFormatter::build(
            "hello",
            MessageOptions {
                rules: MessageRules::Top,
                kind: MessageKind::Info,
                title: Some("notice".to_string()),
            },
        );
        let Block::Panel(top_panel) = &top.blocks[0] else {
            panic!("expected panel block");
        };
        assert_eq!(top_panel.rules, PanelRules::Top);
        assert_eq!(top_panel.title.as_deref(), Some("notice"));

        let bottom = MessageFormatter::build(
            "hello",
            MessageOptions {
                rules: MessageRules::Bottom,
                kind: MessageKind::Success,
                title: None,
            },
        );
        let Block::Panel(bottom_panel) = &bottom.blocks[0] else {
            panic!("expected panel block");
        };
        assert_eq!(bottom_panel.rules, PanelRules::Bottom);
        assert_eq!(bottom_panel.title.as_deref(), Some("SUCCESS"));
    }

    #[test]
    fn blank_text_normalizes_to_empty_document() {
        let doc = MessageFormatter::build(
            "\n \n\t\n",
            MessageOptions {
                rules: MessageRules::None,
                kind: MessageKind::Info,
                title: None,
            },
        );

        assert!(doc.blocks.is_empty());
    }
}
