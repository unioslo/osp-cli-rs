//! Structured display blocks used as the boundary between formatting and
//! terminal rendering.
//!
//! This module exists so higher-level code can describe *what* should be shown
//! without deciding *how* it should be painted in a terminal. Formatters build
//! documents out of semantic blocks, and the renderer later turns those blocks
//! into themed terminal text.
//!
//! In practice, this keeps rendering bugs easier to localize:
//!
//! - if the document shape is wrong, the formatter is wrong
//! - if the document is right but the terminal output is wrong, the renderer is
//!   wrong
//!
//! Contract:
//!
//! - document types may carry semantic styling hints and layout intent
//! - they should not depend on terminal width probing, theme resolution, or
//!   config precedence
//! - block variants are intentionally higher-level than raw ANSI/text spans so
//!   multiple renderers can share the same model

use serde_json::Value;

use crate::ui::TableBorderStyle;
use crate::ui::chrome::SectionFrameStyle;
use crate::ui::style::StyleToken;

/// Renderable document composed of high-level display blocks.
///
/// The document model is the handoff point between semantic formatting and
/// terminal rendering. Callers populate it with blocks; renderers decide how
/// those blocks map onto plain or rich terminal output.
///
/// # Examples
///
/// ```
/// use osp_cli::ui::{Block, Document, LineBlock, LinePart};
///
/// let document = Document {
///     blocks: vec![Block::Line(LineBlock {
///         parts: vec![LinePart {
///             text: "hello".to_string(),
///             token: None,
///         }],
///     })],
/// };
///
/// assert_eq!(document.blocks.len(), 1);
/// ```
#[derive(Debug, Clone, Default)]
pub struct Document {
    /// Ordered blocks to render.
    pub blocks: Vec<Block>,
}

/// Top-level document block variants understood by the renderer.
#[derive(Debug, Clone)]
pub enum Block {
    /// A single styled line.
    Line(LineBlock),
    /// A framed section containing another document.
    Panel(PanelBlock),
    /// A fenced code block.
    Code(CodeBlock),
    /// A JSON payload block.
    Json(JsonBlock),
    /// A tabular data block.
    Table(TableBlock),
    /// A plain list-of-values block.
    Value(ValueBlock),
    /// An MREG-style key/value block.
    Mreg(MregBlock),
}

/// Single rendered line composed of independently styled parts.
#[derive(Debug, Clone)]
pub struct LineBlock {
    /// Ordered text parts for the line.
    pub parts: Vec<LinePart>,
}

/// Fragment of a rendered line with optional semantic styling.
#[derive(Debug, Clone)]
pub struct LinePart {
    /// Literal text to render.
    pub text: String,
    /// Optional style token for the fragment.
    pub token: Option<StyleToken>,
}

/// Framed panel containing a nested document.
///
/// Panels carry grouping intent without hard-coding terminal chrome. The
/// renderer is free to honor that intent with ASCII, Unicode, or theme-aware
/// borders.
#[derive(Debug, Clone)]
pub struct PanelBlock {
    /// Optional title displayed in the panel chrome.
    pub title: Option<String>,
    /// Nested document rendered inside the panel body.
    pub body: Document,
    /// Which horizontal rules to render around the panel.
    pub rules: PanelRules,
    /// Explicit frame-style override for the panel.
    pub frame_style: Option<SectionFrameStyle>,
    /// Optional semantic kind identifier for the panel.
    pub kind: Option<String>,
    /// Optional style token used for panel borders.
    pub border_token: Option<StyleToken>,
    /// Optional style token used for the panel title.
    pub title_token: Option<StyleToken>,
}

/// Rule placement policy for panel chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRules {
    /// Render no horizontal rules.
    None,
    /// Render only a top rule.
    Top,
    /// Render only a bottom rule.
    Bottom,
    /// Render both top and bottom rules.
    Both,
}

/// Fenced code block with optional language metadata.
#[derive(Debug, Clone)]
pub struct CodeBlock {
    /// Code payload to render verbatim.
    pub code: String,
    /// Optional language tag used for display or copy helpers.
    pub language: Option<String>,
}

/// JSON payload block.
#[derive(Debug, Clone)]
pub struct JsonBlock {
    /// JSON value to render.
    pub payload: Value,
}

/// Tabular document block.
///
/// Table blocks preserve row/column structure until the final render pass so
/// width-aware layout decisions stay inside the renderer.
#[derive(Debug, Clone)]
pub struct TableBlock {
    /// Stable identifier used for interactive table state.
    pub block_id: u64,
    /// Table rendering style.
    pub style: TableStyle,
    /// Optional border style override for this table.
    pub border_override: Option<TableBorderStyle>,
    /// Column headers in display order.
    pub headers: Vec<String>,
    /// Table rows in display order.
    pub rows: Vec<Vec<Value>>,
    /// Optional header metadata rendered above grouped tables.
    pub header_pairs: Vec<(String, Value)>,
    /// Optional per-column alignment hints.
    pub align: Option<Vec<TableAlign>>,
    /// Whether the renderer may shrink the table to fit width constraints.
    pub shrink_to_fit: bool,
    /// Logical nesting depth used for grouped table presentation.
    pub depth: usize,
}

/// Table presentation style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableStyle {
    /// Standard grid/table presentation.
    Grid,
    /// Semantic guide-table presentation.
    Guide,
    /// Markdown-compatible table presentation.
    Markdown,
}

/// Column alignment hint for table rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableAlign {
    /// Use renderer defaults for the column.
    Default,
    /// Left-align cell contents.
    Left,
    /// Center-align cell contents.
    Center,
    /// Right-align cell contents.
    Right,
}

/// Block representing a simple ordered list of scalar values.
#[derive(Debug, Clone)]
pub struct ValueBlock {
    /// Values to render line by line.
    pub values: Vec<String>,
    /// Additional indent applied before each rendered line.
    pub indent: usize,
    /// Whether inline markup should be parsed before rendering.
    pub inline_markup: bool,
    /// Layout policy for the values.
    pub layout: ValueLayout,
}

/// Layout policy for scalar value blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueLayout {
    /// Render one item per line.
    #[default]
    Vertical,
    /// Render as a grid when the list is long enough.
    AutoGrid,
}

/// MREG-style hierarchical key/value block.
#[derive(Debug, Clone)]
pub struct MregBlock {
    /// Stable identifier used for interactive MREG state.
    pub block_id: u64,
    /// Rows that make up the block.
    pub rows: Vec<MregRow>,
}

/// One row inside an MREG-style block.
#[derive(Debug, Clone)]
pub struct MregRow {
    /// Ordered entries rendered for the row.
    pub entries: Vec<MregEntry>,
}

/// Key/value entry inside an MREG row.
#[derive(Debug, Clone)]
pub struct MregEntry {
    /// Display key for the entry.
    pub key: String,
    /// Logical nesting depth of the entry.
    pub depth: usize,
    /// Rendered value payload.
    pub value: MregValue,
}

/// Rendered value kinds supported by MREG output.
#[derive(Debug, Clone)]
pub enum MregValue {
    /// Group heading marker.
    Group,
    /// Visual separator marker.
    Separator,
    /// Scalar JSON value.
    Scalar(Value),
    /// Vertical list of values.
    VerticalList(Vec<Value>),
    /// Compact grid of values.
    Grid(Vec<Value>),
}
