use serde_json::{Map, Value};
use unicode_width::UnicodeWidthStr;

use crate::guide::{GuideEntry, GuideView};
use crate::ui::TableBorderStyle;
use crate::ui::chrome::SectionFrameStyle;
use crate::ui::display::value_to_display;
use crate::ui::document::{
    Block, Document, LineBlock, LinePart, PanelBlock, PanelRules, TableAlign, TableBlock,
    TableStyle,
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
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct LowerDocumentOptions<'a> {
    pub frame_style: SectionFrameStyle,
    pub panel_kind: Option<&'a str>,
    pub key_value_border: TableBorderStyle,
    pub key_value_indent: Option<usize>,
    pub key_value_gap: Option<usize>,
}

impl DocumentModel {
    /// Builds a semantic document model from a parsed guide view.
    pub fn from_guide_view(view: &GuideView) -> Self {
        let mut blocks = Vec::new();

        append_paragraphs(&mut blocks, &view.preamble);
        append_section_if_any(
            &mut blocks,
            Some("Usage".to_string()),
            guide_usage_paragraphs(&view.usage),
        );
        append_section_if_any(
            &mut blocks,
            Some("Commands".to_string()),
            guide_entries(&view.commands),
        );
        append_section_if_any(
            &mut blocks,
            Some("Arguments".to_string()),
            guide_entries(&view.arguments),
        );
        append_section_if_any(
            &mut blocks,
            Some("Options".to_string()),
            guide_entries(&view.options),
        );
        append_section_if_any(
            &mut blocks,
            Some("Common Invocation Options".to_string()),
            guide_entries(&view.common_invocation_options),
        );
        append_section_if_any(
            &mut blocks,
            Some("Notes".to_string()),
            guide_paragraphs(&view.notes),
        );

        for section in &view.sections {
            let mut section_blocks = guide_paragraphs(&section.paragraphs);
            if !section.entries.is_empty() {
                if !section_blocks.is_empty() {
                    section_blocks.push(BlockModel::Blank);
                }
                section_blocks.push(BlockModel::KeyValue(key_value_block_from_entries(
                    &section.entries,
                )));
            }
            append_section_if_any(&mut blocks, Some(section.title.clone()), section_blocks);
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
    /// Object keys follow `preferred_keys` first when that ordering is applicable.
    pub fn from_value(value: &Value, preferred_keys: Option<&[String]>) -> Self {
        match value {
            Value::Object(map) => root_object_model(map, preferred_keys),
            Value::Array(items) => array_model(items),
            _ => Self {
                blocks: vec![BlockModel::Paragraph(value_to_display(value))],
            },
        }
    }

    /// Renders the model as Markdown, wrapping prose blocks to the optional width.
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
    /// Advances `next_block_id` for any generated blocks that require stable IDs.
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

fn guide_entries(entries: &[GuideEntry]) -> Vec<BlockModel> {
    if entries.is_empty() {
        return Vec::new();
    }
    vec![BlockModel::KeyValue(key_value_block_from_entries(entries))]
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
    }
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
    }
}

fn lower_blocks(
    blocks: &[BlockModel],
    options: LowerDocumentOptions<'_>,
    next_block_id: &mut u64,
    inherited_border: TableBorderStyle,
) -> Vec<Block> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            BlockModel::Section(section) => {
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
                    rules: PanelRules::None,
                    frame_style: Some(options.frame_style),
                    kind: options.panel_kind.map(str::to_string),
                    border_token: Some(StyleToken::PanelBorder),
                    title_token: Some(StyleToken::PanelTitle),
                }));
            }
            BlockModel::Paragraph(text) => out.push(Block::Line(LineBlock {
                parts: vec![LinePart {
                    text: text.clone(),
                    token: Some(StyleToken::Value),
                }],
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
                out.extend(list.items.iter().map(|item| {
                    Block::Line(LineBlock {
                        parts: vec![LinePart {
                            text: item.clone(),
                            token: Some(StyleToken::Value),
                        }],
                    })
                }));
            }
            BlockModel::Blank => out.push(Block::Line(LineBlock { parts: Vec::new() })),
        }
    }
    out
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

    if let Some(total_width) = width {
        if headers.len() == 2 {
            let min_second = widths[1].min(UnicodeWidthStr::width(headers[1].as_str()).max(3));
            let available = total_width.saturating_sub(widths[0] + 7);
            widths[1] = available.max(min_second).min(widths[1]);
        }
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
mod tests {
    use serde_json::json;

    use super::{BlockModel, DocumentModel, LowerDocumentOptions};
    use crate::guide::GuideView;
    use crate::ui::TableBorderStyle;
    use crate::ui::chrome::SectionFrameStyle;

    #[test]
    fn guide_view_lowers_commands_to_key_value_sections_unit() {
        let view = GuideView::from_text("Commands:\n  help  Show help\n");
        let model = DocumentModel::from_guide_view(&view);
        let BlockModel::Section(section) = &model.blocks[0] else {
            panic!("expected section");
        };
        assert_eq!(section.title.as_deref(), Some("Commands"));
        assert!(matches!(section.blocks[0], BlockModel::KeyValue(_)));
    }

    #[test]
    fn markdown_from_guide_view_uses_two_column_table_unit() {
        let view = GuideView::from_text("Commands:\n  help  Show help\n");
        let rendered = DocumentModel::from_guide_view(&view).to_markdown_with_width(Some(80));
        assert!(rendered.contains("| name"));
        assert!(rendered.contains("Show help"));
    }

    #[test]
    fn scalar_object_value_renders_as_key_value_group_unit() {
        let value = json!({"uid": "alice", "mail": "a@uio.no"});
        let model = DocumentModel::from_value(&value, None);
        assert!(matches!(model.blocks[0], BlockModel::KeyValue(_)));
    }

    #[test]
    fn help_key_values_can_lower_to_bordered_tables_unit() {
        let view = GuideView::from_text("Commands:\n  help  Show help\n");
        let model = DocumentModel::from_guide_view(&view);
        let document = model.lower_to_render_document(
            LowerDocumentOptions {
                frame_style: SectionFrameStyle::Top,
                panel_kind: Some("help"),
                key_value_border: TableBorderStyle::Round,
                key_value_indent: None,
                key_value_gap: None,
            },
            &mut 1,
        );
        let crate::ui::document::Block::Panel(panel) = &document.blocks[0] else {
            panic!("expected panel");
        };
        assert!(matches!(
            panel.body.blocks[0],
            crate::ui::document::Block::Table(_)
        ));
    }
}
