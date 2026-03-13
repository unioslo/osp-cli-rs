//! Semantic intermediate model used before lowering into renderable UI blocks.
//!
//! This module exists to bridge rich semantic inputs, like guides and generic
//! JSON values, into a layout-oriented shape that is still independent from the
//! final terminal renderer. It sits between "raw semantic payload" and
//! [`crate::ui::document`] so formatting code can make structure decisions
//! without dealing with ANSI, width probing, or renderer chrome directly.
//!
//! Contract:
//!
//! - this layer owns semantic block shaping and markdown lowering
//! - it should not own terminal capability detection or theme resolution
//! - callers should use it when they need guide/value formatting decisions, not
//!   as a general-purpose public document API

use serde_json::{Map, Value};
use unicode_width::UnicodeWidthStr;

use crate::guide::{GuideEntry, GuideSectionKind, GuideView};
use crate::ui::TableBorderStyle;
use crate::ui::chrome::{RuledSectionPolicy, SectionFrameStyle};
use crate::ui::display::value_to_display;
use crate::ui::document::{
    Block, Document, LineBlock, LinePart, PanelBlock, PanelRules, TableAlign, TableBlock,
    TableStyle, ValueBlock, ValueLayout,
};
use crate::ui::style::StyleToken;

#[derive(Debug, Clone, Default)]
pub struct DocumentModel {
    pub blocks: Vec<BlockModel>,
}

#[derive(Debug, Clone)]
pub enum BlockModel {
    Section(SectionModel),
    Paragraph(String),
    KeyValue(KeyValueBlockModel),
    Table(TableModel),
    List(ListModel),
    Blank,
}

#[derive(Debug, Clone, Default)]
pub struct SectionModel {
    pub title: Option<String>,
    pub blocks: Vec<BlockModel>,
}

#[derive(Debug, Clone, Default)]
pub struct KeyValueBlockModel {
    pub key_header: Option<String>,
    pub value_header: Option<String>,
    pub rows: Vec<KeyValueRowModel>,
    pub border_override: Option<TableBorderStyle>,
    pub markdown_style: KeyValueMarkdownStyle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum KeyValueMarkdownStyle {
    #[default]
    Table,
    Lines,
}

#[derive(Debug, Clone, Default)]
pub struct KeyValueRowModel {
    pub key: String,
    pub value: String,
    pub indent: Option<String>,
    pub gap: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TableModel {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub align: Option<Vec<TableAlign>>,
    pub border_override: Option<TableBorderStyle>,
}

#[derive(Debug, Clone, Default)]
pub struct ListModel {
    /// Shaped display strings before final renderer layout.
    pub items: Vec<String>,
    /// Leading spaces emitted when the list lowers to one line per item.
    pub indent: usize,
    /// Whether inline markdown-style spans should become styled line parts.
    pub inline_markup: bool,
    /// Preferred renderer layout for the list payload.
    pub layout: ValueLayout,
}

#[derive(Debug, Clone, Copy)]
pub struct LowerDocumentOptions<'a> {
    pub frame_style: SectionFrameStyle,
    pub ruled_section_policy: RuledSectionPolicy,
    pub panel_kind: Option<&'a str>,
    pub key_value_border: TableBorderStyle,
    pub key_value_indent: Option<usize>,
    pub key_value_gap: Option<usize>,
}

impl DocumentModel {
    /// Builds a semantic document model from a parsed guide view.
    ///
    /// This preserves guide sections and entry groupings until a later lowering
    /// step decides how they should appear in terminal output. Canonical
    /// top-level buckets (`usage`, `commands`, ...) are treated as synthetic
    /// defaults: when ordered `sections[]` already contains a builtin section
    /// of that kind, the ordered section wins and the synthetic default is
    /// suppressed so author order is preserved without duplication.
    pub fn from_guide_view(view: &GuideView) -> Self {
        let mut blocks = Vec::new();
        let use_ordered_sections = view.uses_ordered_section_representation();
        let has_usage_section = view.has_canonical_builtin_section_kind(GuideSectionKind::Usage);
        let has_commands_section =
            view.has_canonical_builtin_section_kind(GuideSectionKind::Commands);
        let has_arguments_section =
            view.has_canonical_builtin_section_kind(GuideSectionKind::Arguments);
        let has_options_section =
            view.has_canonical_builtin_section_kind(GuideSectionKind::Options);
        let has_common_invocation_options_section =
            view.has_canonical_builtin_section_kind(GuideSectionKind::CommonInvocationOptions);
        let has_notes_section = view.has_canonical_builtin_section_kind(GuideSectionKind::Notes);

        append_paragraphs(&mut blocks, &view.preamble);
        if !(use_ordered_sections && has_usage_section) {
            append_section_if_any(
                &mut blocks,
                Some("Usage".to_string()),
                guide_usage_paragraphs(&view.usage),
            );
        }
        if !(use_ordered_sections && has_commands_section) {
            append_section_if_any(
                &mut blocks,
                Some("Commands".to_string()),
                guide_entries(&view.commands),
            );
        }
        if !(use_ordered_sections && has_arguments_section) {
            append_section_if_any(
                &mut blocks,
                Some("Arguments".to_string()),
                guide_entries(&view.arguments),
            );
        }
        if !(use_ordered_sections && has_options_section) {
            append_section_if_any(
                &mut blocks,
                Some("Options".to_string()),
                guide_entries(&view.options),
            );
        }
        if !(use_ordered_sections && has_common_invocation_options_section) {
            append_section_if_any(
                &mut blocks,
                Some("Common Invocation Options".to_string()),
                guide_entries(&view.common_invocation_options),
            );
        }
        if !(use_ordered_sections && has_notes_section) {
            append_section_if_any(
                &mut blocks,
                Some("Notes".to_string()),
                guide_notes_paragraphs(&view.notes),
            );
        }

        for section in &view.sections {
            if !use_ordered_sections && section.is_canonical_builtin_section() {
                continue;
            }
            let mut section_blocks = guide_paragraphs(&section.paragraphs);
            if !section.entries.is_empty() {
                if !section_blocks.is_empty() {
                    section_blocks.push(BlockModel::Blank);
                }
                section_blocks.push(BlockModel::KeyValue(key_value_block_from_entries(
                    &section.entries,
                )));
            }
            if let Some(data) = section.data.as_ref() {
                let data_blocks = guide_section_data_blocks(data);
                if !data_blocks.is_empty() {
                    if !section_blocks.is_empty() {
                        section_blocks.push(BlockModel::Blank);
                    }
                    section_blocks.extend(data_blocks);
                }
            }
            append_section_if_any(&mut blocks, section_title(section), section_blocks);
        }

        if !blocks.is_empty() && !view.epilogue.is_empty() {
            blocks.push(BlockModel::Blank);
            blocks.push(BlockModel::Blank);
        }
        append_paragraphs(&mut blocks, &view.epilogue);

        Self { blocks }
    }

    /// Builds a document model from a JSON value.
    ///
    /// Object keys follow `preferred_keys` first when that ordering is
    /// applicable. Arrays and nested objects are normalized into the block
    /// shapes the UI formatter understands.
    pub fn from_value(value: &Value, preferred_keys: Option<&[String]>) -> Self {
        match value {
            Value::Object(map) => root_object_model(map, preferred_keys),
            Value::Array(items) => array_model(items),
            _ => Self {
                blocks: vec![BlockModel::Paragraph(value_to_display(value))],
            },
        }
    }

    /// Renders the model as Markdown, wrapping prose blocks to the optional
    /// width.
    ///
    /// Returns an empty string when the model contains no renderable blocks.
    pub fn to_markdown_with_width(&self, width: Option<usize>) -> String {
        let mut sections = Vec::new();
        render_markdown_blocks(&self.blocks, 0, width, &mut sections);
        let mut rendered = sections.join("\n\n");
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered
    }

    /// Lowers the semantic model into a renderable document tree.
    ///
    /// Advances `next_block_id` for any generated blocks that require stable
    /// IDs.
    pub fn lower_to_render_document(
        &self,
        options: LowerDocumentOptions<'_>,
        next_block_id: &mut u64,
    ) -> Document {
        Document {
            blocks: lower_blocks(
                &self.blocks,
                options,
                next_block_id,
                options.key_value_border,
            ),
        }
    }
}

fn append_paragraphs(blocks: &mut Vec<BlockModel>, paragraphs: &[String]) {
    for line in paragraphs {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        blocks.push(BlockModel::Paragraph(trimmed.to_string()));
    }
}

fn append_section_if_any(
    blocks: &mut Vec<BlockModel>,
    title: Option<String>,
    section_blocks: Vec<BlockModel>,
) {
    if section_blocks.is_empty() {
        return;
    }
    if !blocks.is_empty() {
        blocks.push(BlockModel::Blank);
        blocks.push(BlockModel::Blank);
    }
    blocks.push(BlockModel::Section(SectionModel {
        title,
        blocks: section_blocks,
    }));
}

fn guide_paragraphs(paragraphs: &[String]) -> Vec<BlockModel> {
    paragraphs
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| BlockModel::Paragraph(line.clone()))
        .collect()
}

fn guide_usage_paragraphs(paragraphs: &[String]) -> Vec<BlockModel> {
    paragraphs
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| BlockModel::Paragraph(format!("  {}", line.trim())))
        .collect()
}

fn guide_notes_paragraphs(paragraphs: &[String]) -> Vec<BlockModel> {
    paragraphs
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| BlockModel::Paragraph(format!("  {}", line.trim())))
        .collect()
}

fn guide_entries(entries: &[GuideEntry]) -> Vec<BlockModel> {
    if entries.is_empty() {
        return Vec::new();
    }
    vec![BlockModel::KeyValue(key_value_block_from_entries(entries))]
}

// Section `data` is the semantic escape hatch used by intro/help templates.
// Known row-shaped payloads lower into help-style entries, scalar arrays lower
// into the generic value/grid path, and anything more complex falls back to
// recursive value formatting instead of inventing a new section-specific shape.
fn guide_section_data_blocks(value: &Value) -> Vec<BlockModel> {
    if let Some(entries) = guide_entries_from_value(value) {
        return guide_entries(&entries);
    }
    if let Value::Object(map) = value
        && should_render_as_key_value_group(map)
    {
        return vec![BlockModel::KeyValue(key_value_block_from_guide_data_map(
            map, None,
        ))];
    }
    if let Value::Array(items) = value
        && items.iter().all(is_scalar_like)
    {
        return vec![BlockModel::List(ListModel {
            items: items.iter().map(value_to_display).collect(),
            indent: 2,
            inline_markup: true,
            layout: ValueLayout::AutoGrid,
        })];
    }
    value_blocks(value, None)
}

fn key_value_block_from_entries(entries: &[GuideEntry]) -> KeyValueBlockModel {
    KeyValueBlockModel {
        key_header: Some("name".to_string()),
        value_header: Some("short_help".to_string()),
        rows: entries
            .iter()
            .map(|entry| KeyValueRowModel {
                key: entry.name.clone(),
                value: entry.short_help.clone(),
                indent: entry.display_indent.clone(),
                gap: entry.display_gap.clone(),
            })
            .collect(),
        border_override: None,
        markdown_style: KeyValueMarkdownStyle::Lines,
    }
}

fn guide_entries_from_value(value: &Value) -> Option<Vec<GuideEntry>> {
    let Value::Array(items) = value else {
        return None;
    };

    items.iter().map(guide_entry_from_value).collect()
}

fn guide_entry_from_value(value: &Value) -> Option<GuideEntry> {
    let Value::Object(map) = value else {
        return None;
    };
    if map.keys().any(|key| key != "name" && key != "short_help") {
        return None;
    }

    Some(GuideEntry {
        name: map.get("name")?.as_str()?.to_string(),
        short_help: map
            .get("short_help")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        display_indent: None,
        display_gap: None,
    })
}

fn root_object_model(map: &Map<String, Value>, preferred_keys: Option<&[String]>) -> DocumentModel {
    if should_render_as_key_value_group(map) {
        return DocumentModel {
            blocks: vec![BlockModel::KeyValue(key_value_block_from_map(
                map,
                preferred_keys,
            ))],
        };
    }

    let mut blocks = Vec::new();
    for key in ordered_keys(map, preferred_keys) {
        let Some(value) = map.get(&key) else {
            continue;
        };
        let section_blocks = value_blocks(value, None);
        append_section_if_any(&mut blocks, Some(display_title(&key)), section_blocks);
    }
    DocumentModel { blocks }
}

fn array_model(items: &[Value]) -> DocumentModel {
    DocumentModel {
        blocks: value_blocks(&Value::Array(items.to_vec()), None),
    }
}

fn value_blocks(value: &Value, preferred_keys: Option<&[String]>) -> Vec<BlockModel> {
    match value {
        Value::Object(map) => root_object_model(map, preferred_keys).blocks,
        Value::Array(items) if items.is_empty() => {
            vec![BlockModel::Paragraph(value_to_display(value))]
        }
        Value::Array(items) if items.iter().all(is_scalar_like) => {
            vec![BlockModel::List(ListModel {
                items: items.iter().map(value_to_display).collect(),
                indent: 0,
                inline_markup: false,
                layout: ValueLayout::Vertical,
            })]
        }
        Value::Array(items) => {
            if let Some(table) = uniform_scalar_object_table(items) {
                return vec![BlockModel::Table(table)];
            }

            let mut blocks = Vec::new();
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    blocks.push(BlockModel::Blank);
                }
                blocks.extend(value_blocks(item, None));
            }
            blocks
        }
        _ => vec![BlockModel::Paragraph(value_to_display(value))],
    }
}

fn uniform_scalar_object_table(items: &[Value]) -> Option<TableModel> {
    let rows = uniform_scalar_object_rows(items)?;
    let headers = collect_header_order(&rows);
    Some(TableModel {
        headers: headers.clone(),
        rows: rows
            .iter()
            .map(|row| {
                headers
                    .iter()
                    .map(|key| {
                        row.get(key)
                            .cloned()
                            .unwrap_or_else(|| Value::String(String::new()))
                    })
                    .collect()
            })
            .collect(),
        align: None,
        border_override: None,
    })
}

fn collect_header_order(rows: &[Map<String, Value>]) -> Vec<String> {
    let mut order = Vec::new();
    for row in rows {
        for key in row.keys() {
            if !order.iter().any(|candidate| candidate == key) {
                order.push(key.clone());
            }
        }
    }
    order
}

fn key_value_block_from_map(
    map: &Map<String, Value>,
    preferred_keys: Option<&[String]>,
) -> KeyValueBlockModel {
    KeyValueBlockModel {
        key_header: None,
        value_header: None,
        rows: ordered_keys(map, preferred_keys)
            .into_iter()
            .filter_map(|key| {
                map.get(&key).map(|value| KeyValueRowModel {
                    key,
                    value: value_to_display(value),
                    indent: None,
                    gap: None,
                })
            })
            .collect(),
        border_override: None,
        markdown_style: KeyValueMarkdownStyle::Table,
    }
}

fn key_value_block_from_guide_data_map(
    map: &Map<String, Value>,
    preferred_keys: Option<&[String]>,
) -> KeyValueBlockModel {
    KeyValueBlockModel {
        key_header: None,
        value_header: None,
        rows: ordered_keys(map, preferred_keys)
            .into_iter()
            .filter_map(|key| {
                map.get(&key).map(|value| KeyValueRowModel {
                    key,
                    value: guide_data_value_to_display(value),
                    indent: None,
                    gap: None,
                })
            })
            .collect(),
        border_override: None,
        markdown_style: KeyValueMarkdownStyle::Table,
    }
}

fn guide_data_value_to_display(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Array(items) if items.is_empty() => "[]".to_string(),
        _ => value_to_display(value),
    }
}

fn lower_blocks(
    blocks: &[BlockModel],
    options: LowerDocumentOptions<'_>,
    next_block_id: &mut u64,
    inherited_border: TableBorderStyle,
) -> Vec<Block> {
    let mut out = Vec::new();
    let last_section_index = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| matches!(block, BlockModel::Section(_)).then_some(index))
        .next_back();

    for (index, block) in blocks.iter().enumerate() {
        match block {
            BlockModel::Section(section) => {
                let (rules, frame_style) = lower_section_chrome(
                    options.frame_style,
                    options.ruled_section_policy,
                    Some(index) == last_section_index,
                );
                out.push(Block::Panel(PanelBlock {
                    title: section.title.clone(),
                    body: Document {
                        blocks: lower_blocks(
                            &section.blocks,
                            options,
                            next_block_id,
                            inherited_border,
                        ),
                    },
                    rules,
                    frame_style,
                    kind: options.panel_kind.map(str::to_string),
                    border_token: Some(StyleToken::PanelBorder),
                    title_token: Some(StyleToken::PanelTitle),
                }));
            }
            BlockModel::Paragraph(text) => out.push(Block::Line(LineBlock {
                parts: crate::ui::inline::parts_from_inline(text)
                    .into_iter()
                    .map(|mut part| {
                        if part.token.is_none() {
                            part.token = Some(StyleToken::Value);
                        }
                        part
                    })
                    .collect(),
            })),
            BlockModel::KeyValue(block) => out.extend(lower_key_value_block(
                block,
                options,
                next_block_id,
                inherited_border,
            )),
            BlockModel::Table(block) => out.push(Block::Table(TableBlock {
                block_id: allocate_block_id(next_block_id),
                style: TableStyle::Guide,
                border_override: block.border_override,
                headers: block.headers.clone(),
                rows: block.rows.clone(),
                header_pairs: Vec::new(),
                align: block.align.clone(),
                shrink_to_fit: true,
                depth: 0,
            })),
            BlockModel::List(list) => {
                if matches!(list.layout, ValueLayout::AutoGrid) {
                    out.push(Block::Value(ValueBlock {
                        values: list.items.clone(),
                        indent: list.indent,
                        inline_markup: list.inline_markup,
                        layout: list.layout,
                    }));
                } else {
                    out.extend(list.items.iter().map(|item| {
                        let mut parts = crate::ui::inline::parts_from_inline(item);
                        for part in &mut parts {
                            if part.token.is_none() {
                                part.token = Some(StyleToken::Value);
                            }
                        }
                        if list.indent > 0 {
                            parts.insert(
                                0,
                                LinePart {
                                    text: " ".repeat(list.indent),
                                    token: None,
                                },
                            );
                        }
                        Block::Line(LineBlock { parts })
                    }));
                }
            }
            BlockModel::Blank => out.push(Block::Line(LineBlock { parts: Vec::new() })),
        }
    }
    out
}

fn lower_section_chrome(
    frame_style: SectionFrameStyle,
    ruled_section_policy: RuledSectionPolicy,
    is_last_section: bool,
) -> (PanelRules, Option<SectionFrameStyle>) {
    // Shared ruled sections suppress per-panel framing and let the surrounding
    // panel rules carry the section dividers between adjacent sections.
    if matches!(ruled_section_policy, RuledSectionPolicy::Shared) {
        match frame_style {
            SectionFrameStyle::Top => return (PanelRules::Top, None),
            SectionFrameStyle::Bottom => return (PanelRules::Bottom, None),
            SectionFrameStyle::TopBottom => {
                return (
                    if is_last_section {
                        PanelRules::Both
                    } else {
                        PanelRules::Top
                    },
                    None,
                );
            }
            SectionFrameStyle::None | SectionFrameStyle::Square | SectionFrameStyle::Round => {}
        }
    }

    (PanelRules::None, Some(frame_style))
}

fn lower_key_value_block(
    block: &KeyValueBlockModel,
    options: LowerDocumentOptions<'_>,
    next_block_id: &mut u64,
    inherited_border: TableBorderStyle,
) -> Vec<Block> {
    let border = block.border_override.unwrap_or(inherited_border);
    if matches!(border, TableBorderStyle::Square | TableBorderStyle::Round)
        && block.key_header.is_some()
        && block.value_header.is_some()
    {
        let headers = vec![
            block.key_header.clone().unwrap_or_default(),
            block.value_header.clone().unwrap_or_default(),
        ];
        let rows = block
            .rows
            .iter()
            .map(|row| {
                vec![
                    Value::String(row.key.clone()),
                    Value::String(row.value.clone()),
                ]
            })
            .collect();
        return vec![Block::Table(TableBlock {
            block_id: allocate_block_id(next_block_id),
            style: TableStyle::Guide,
            border_override: Some(border),
            headers,
            rows,
            header_pairs: Vec::new(),
            align: Some(vec![TableAlign::Left, TableAlign::Left]),
            shrink_to_fit: true,
            depth: 0,
        })];
    }

    if block.key_header.is_some() && block.value_header.is_some() {
        let key_width = block
            .rows
            .iter()
            .map(|row| UnicodeWidthStr::width(row.key.as_str()))
            .max()
            .unwrap_or(0);
        return block
            .rows
            .iter()
            .map(|row| lower_help_entry_row(row, key_width, options))
            .collect();
    }

    let key_width = block
        .rows
        .iter()
        .map(|row| UnicodeWidthStr::width(row.key.as_str()))
        .max()
        .unwrap_or(0);
    block
        .rows
        .iter()
        .map(|row| lower_key_value_row(row, key_width, options))
        .collect()
}

fn lower_help_entry_row(
    row: &KeyValueRowModel,
    key_width: usize,
    options: LowerDocumentOptions<'_>,
) -> Block {
    let indent = options
        .key_value_indent
        .map(|value| " ".repeat(value))
        .or_else(|| row.indent.clone())
        .unwrap_or_else(|| "  ".to_string());
    let current_width = UnicodeWidthStr::width(row.key.as_str());
    let padding = key_width.saturating_sub(current_width);
    let gap = if let Some(gap) = options.key_value_gap {
        format!("{}{}", " ".repeat(padding), " ".repeat(gap))
    } else if let Some(gap) = &row.gap {
        gap.clone()
    } else {
        format!("{}{}", " ".repeat(padding), "  ")
    };

    let mut parts = vec![
        LinePart {
            text: indent,
            token: None,
        },
        LinePart {
            text: row.key.clone(),
            token: Some(StyleToken::Key),
        },
    ];
    if !row.value.is_empty() {
        parts.push(LinePart {
            text: format!("{gap}{}", row.value),
            token: Some(StyleToken::Value),
        });
    }
    Block::Line(LineBlock { parts })
}

fn lower_key_value_row(
    row: &KeyValueRowModel,
    key_width: usize,
    options: LowerDocumentOptions<'_>,
) -> Block {
    let indent = options
        .key_value_indent
        .map(|value| " ".repeat(value))
        .or_else(|| row.indent.clone())
        .unwrap_or_default();
    let current_width = UnicodeWidthStr::width(row.key.as_str());
    let padding = key_width.saturating_sub(current_width);
    let gap = if let Some(gap) = options.key_value_gap {
        format!("{}{}", " ".repeat(padding), " ".repeat(gap))
    } else if let Some(gap) = &row.gap {
        gap.clone()
    } else if row.value.is_empty() {
        format!(":{}", " ".repeat(padding))
    } else {
        format!(":{} ", " ".repeat(padding))
    };

    let mut parts = vec![
        LinePart {
            text: indent,
            token: None,
        },
        LinePart {
            text: row.key.clone(),
            token: Some(StyleToken::Key),
        },
        LinePart {
            text: gap,
            token: None,
        },
    ];
    if !row.value.is_empty() {
        parts.push(LinePart {
            text: row.value.clone(),
            token: Some(StyleToken::Value),
        });
    }
    Block::Line(LineBlock { parts })
}

fn render_markdown_blocks(
    blocks: &[BlockModel],
    depth: usize,
    width: Option<usize>,
    out: &mut Vec<String>,
) {
    for block in blocks {
        match block {
            BlockModel::Section(section) => {
                let heading_level = "#".repeat((depth + 2).max(2));
                let mut section_parts = Vec::new();
                if let Some(title) = &section.title {
                    section_parts.push(format!("{heading_level} {title}"));
                }
                let mut nested = Vec::new();
                render_markdown_blocks(&section.blocks, depth + 1, width, &mut nested);
                if !nested.is_empty() {
                    if !section_parts.is_empty() {
                        section_parts.push(String::new());
                    }
                    section_parts.push(nested.join("\n\n"));
                }
                out.push(section_parts.join("\n"));
            }
            BlockModel::Paragraph(text) => out.push(text.clone()),
            BlockModel::KeyValue(block) => out.push(markdown_key_value(block, width)),
            BlockModel::Table(table) => out.push(markdown_table(
                &table.headers,
                &table.rows,
                width,
                table.align.as_deref(),
            )),
            BlockModel::List(list) => out.push(
                list.items
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            BlockModel::Blank => out.push(String::new()),
        }
    }
}

fn markdown_key_value(block: &KeyValueBlockModel, width: Option<usize>) -> String {
    if let (Some(key_header), Some(value_header)) = (&block.key_header, &block.value_header) {
        if matches!(block.markdown_style, KeyValueMarkdownStyle::Lines) {
            return block
                .rows
                .iter()
                .map(markdown_entry_line)
                .collect::<Vec<_>>()
                .join("\n");
        }
        let headers = vec![key_header.clone(), value_header.clone()];
        let rows = block
            .rows
            .iter()
            .map(|row| {
                vec![
                    Value::String(row.key.clone()),
                    Value::String(row.value.clone()),
                ]
            })
            .collect::<Vec<_>>();
        return markdown_table(&headers, &rows, width, None);
    }

    block
        .rows
        .iter()
        .map(|row| {
            if row.value.is_empty() {
                row.key.clone()
            } else {
                format!("{}: {}", row.key, row.value)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn markdown_entry_line(row: &KeyValueRowModel) -> String {
    if row.value.is_empty() {
        format!("- `{}`", row.key)
    } else {
        format!("- `{}` {}", row.key, row.value)
    }
}

fn markdown_table(
    headers: &[String],
    rows: &[Vec<Value>],
    width: Option<usize>,
    align: Option<&[TableAlign]>,
) -> String {
    if headers.is_empty() {
        return String::new();
    }

    let escaped_rows = rows
        .iter()
        .map(|row| row.iter().map(value_to_display).collect::<Vec<_>>())
        .collect::<Vec<_>>();

    let mut widths = headers
        .iter()
        .map(|header| UnicodeWidthStr::width(header.as_str()).max(3))
        .collect::<Vec<_>>();

    for row in &escaped_rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width_ref) = widths.get_mut(index) {
                *width_ref = (*width_ref).max(UnicodeWidthStr::width(cell.as_str()).max(3));
            }
        }
    }

    if let Some(total_width) = width
        && headers.len() == 2
    {
        let min_second = widths[1].min(UnicodeWidthStr::width(headers[1].as_str()).max(3));
        let available = total_width.saturating_sub(widths[0] + 7);
        widths[1] = available.max(min_second).min(widths[1]);
    }

    let aligns = align
        .map(|entries| entries.to_vec())
        .unwrap_or_else(|| vec![TableAlign::Default; headers.len()]);
    let mut lines = vec![
        format!(
            "| {} |",
            headers
                .iter()
                .enumerate()
                .map(|(index, header)| pad_markdown_cell(header, widths[index], aligns[index]))
                .collect::<Vec<_>>()
                .join(" | ")
        ),
        format!(
            "| {} |",
            widths
                .iter()
                .enumerate()
                .map(|(index, width)| markdown_separator(*width, aligns[index]))
                .collect::<Vec<_>>()
                .join(" | ")
        ),
    ];

    for row in escaped_rows {
        lines.push(format!(
            "| {} |",
            row.iter()
                .enumerate()
                .map(|(index, cell)| pad_markdown_cell(cell, widths[index], aligns[index]))
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    lines.join("\n")
}

fn markdown_separator(width: usize, align: TableAlign) -> String {
    let hyphens = "-".repeat(width.max(3));
    match align {
        TableAlign::Left => format!(":{hyphens}"),
        TableAlign::Center => format!(":{hyphens}:"),
        TableAlign::Right => format!("{hyphens}:"),
        TableAlign::Default => hyphens,
    }
}

fn pad_markdown_cell(value: &str, width: usize, align: TableAlign) -> String {
    let current = UnicodeWidthStr::width(value);
    if current >= width {
        return value.to_string();
    }
    let padding = width - current;
    match align {
        TableAlign::Right => format!("{}{}", " ".repeat(padding), value),
        TableAlign::Center => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", " ".repeat(left), value, " ".repeat(right))
        }
        TableAlign::Default | TableAlign::Left => format!("{value}{}", " ".repeat(padding)),
    }
}

fn should_render_as_key_value_group(map: &Map<String, Value>) -> bool {
    !map.is_empty()
        && map.values().all(|value| match value {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
            Value::Array(items) => items.iter().all(is_scalar_like),
            Value::Object(_) => false,
        })
}

fn uniform_scalar_object_rows(items: &[Value]) -> Option<Vec<Map<String, Value>>> {
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        let Value::Object(map) = item else {
            return None;
        };
        if map.values().any(|value| !is_scalar_like(value)) {
            return None;
        }
        rows.push(map.clone());
    }
    Some(rows)
}

fn is_scalar_like(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn ordered_keys(map: &Map<String, Value>, preferred_keys: Option<&[String]>) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(preferred) = preferred_keys {
        for key in preferred {
            if map.contains_key(key) && !keys.iter().any(|candidate| candidate == key) {
                keys.push(key.clone());
            }
        }
    }
    for key in map.keys() {
        if !keys.iter().any(|candidate| candidate == key) {
            keys.push(key.clone());
        }
    }
    keys
}

fn section_title(section: &crate::guide::GuideSection) -> Option<String> {
    let trimmed = section.title.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn display_title(key: &str) -> String {
    let mut title = String::new();
    let mut capitalize_next = true;
    for ch in key.chars() {
        if matches!(ch, '_' | '-' | '.') {
            title.push(' ');
            capitalize_next = true;
            continue;
        }
        if capitalize_next {
            title.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            title.push(ch);
        }
    }
    title
}

fn allocate_block_id(next_block_id: &mut u64) -> u64 {
    let id = *next_block_id;
    *next_block_id = next_block_id.saturating_add(1);
    id
}

#[cfg(test)]
mod tests;
