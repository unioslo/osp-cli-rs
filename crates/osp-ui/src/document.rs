use serde_json::Value;

use crate::style::StyleToken;

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
    pub token: Option<StyleToken>,
}

#[derive(Debug, Clone)]
pub struct PanelBlock {
    pub title: Option<String>,
    pub body: Document,
    pub rules: PanelRules,
    pub kind: Option<String>,
    pub border_token: Option<StyleToken>,
    pub title_token: Option<StyleToken>,
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
    pub block_id: u64,
    pub style: TableStyle,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub header_pairs: Vec<(String, Value)>,
    pub align: Option<Vec<TableAlign>>,
    pub shrink_to_fit: bool,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStyle {
    Grid,
    Markdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlign {
    Default,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub struct ValueBlock {
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MregBlock {
    pub block_id: u64,
    pub rows: Vec<MregRow>,
}

#[derive(Debug, Clone)]
pub struct MregRow {
    pub entries: Vec<MregEntry>,
}

#[derive(Debug, Clone)]
pub struct MregEntry {
    pub key: String,
    pub depth: usize,
    pub value: MregValue,
}

#[derive(Debug, Clone)]
pub enum MregValue {
    Group,
    Separator,
    Scalar(Value),
    List(Vec<Value>),
}
