#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Doc {
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Blank,
    Rule,
    Paragraph(ParagraphBlock),
    Section(SectionBlock),
    Table(TableBlock),
    GuideEntries(GuideEntriesBlock),
    KeyValue(KeyValueBlock),
    List(ListBlock),
    Json(JsonBlock),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParagraphBlock {
    pub text: String,
    pub indent: usize,
    pub inline_markup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionBlock {
    pub title: Option<String>,
    pub title_chrome: SectionTitleChrome,
    pub body_indent: usize,
    pub inline_title_suffix: Option<String>,
    pub trailing_newline: bool,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionTitleChrome {
    #[default]
    Plain,
    Ruled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableBlock {
    pub summary: Vec<KeyValueRow>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideEntriesBlock {
    pub default_indent: String,
    pub default_gap: Option<String>,
    pub rows: Vec<GuideEntryRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideEntryRow {
    pub key: String,
    pub value: String,
    pub indent_hint: Option<String>,
    pub gap_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValueBlock {
    pub style: KeyValueStyle,
    pub rows: Vec<KeyValueRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyValueStyle {
    Plain,
    Bulleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValueRow {
    pub key: String,
    pub value: String,
    pub indent: Option<String>,
    pub gap: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListBlock {
    pub items: Vec<String>,
    pub indent: usize,
    pub inline_markup: bool,
    pub auto_grid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonBlock {
    pub text: String,
}
