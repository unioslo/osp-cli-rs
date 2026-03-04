use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Line(LineBlock),
    Panel(PanelBlock),
    Code(CodeBlock),
    Json(JsonBlock),
    Table(TableBlock),
    Value(ValueBlock),
    Mreg(MregBlock),
}

#[derive(Debug, Clone)]
pub struct LineBlock {
    pub parts: Vec<LinePart>,
}

#[derive(Debug, Clone)]
pub struct LinePart {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct PanelBlock {
    pub title: Option<String>,
    pub body: Document,
    pub rules: PanelRules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRules {
    None,
    Top,
    Bottom,
    Both,
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    pub code: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JsonBlock {
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct TableBlock {
    pub style: TableStyle,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStyle {
    Grid,
    Markdown,
}

#[derive(Debug, Clone)]
pub struct ValueBlock {
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MregBlock {
    pub rows: Vec<MregRow>,
}

#[derive(Debug, Clone)]
pub struct MregRow {
    pub entries: Vec<MregEntry>,
}

#[derive(Debug, Clone)]
pub struct MregEntry {
    pub key: String,
    pub value: MregValue,
}

#[derive(Debug, Clone)]
pub enum MregValue {
    Scalar(String),
    List(Vec<String>),
}
