use crate::document::{
    Block, CodeBlock, Document, JsonBlock, LineBlock, LinePart, PanelBlock, PanelRules,
};

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
                    parts: vec![LinePart { text }],
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

#[cfg(test)]
mod tests {
    use super::{MessageContent, MessageFormatter, MessageKind, MessageOptions, MessageRules};
    use crate::document::Block;

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
}
