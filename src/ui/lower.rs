use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::core::output::OutputFormat;
use crate::core::output_model::{
    Group, OutputItems, OutputResult, group_rows, output_items_to_rows, output_items_to_value,
};
use crate::core::row::Row;
use crate::guide::{GuideEntry, GuideSection, GuideSectionKind, GuideView};

use super::doc::{
    Block, Doc, GuideEntriesBlock, GuideEntryRow, JsonBlock, KeyValueBlock, KeyValueRow,
    KeyValueStyle, ListBlock, ParagraphBlock, SectionBlock, SectionTitleChrome, TableBlock,
};
use super::plan::RenderPlan;
use super::settings::{HelpLayout, ResolvedHelpChromeSettings};
use super::visible_inline_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuidePresentation {
    Direct,
    HelpLayout(HelpLayout, bool),
}

#[derive(Debug, Clone, Copy)]
struct GuideSectionRef<'a> {
    title: &'a str,
    kind: GuideSectionKind,
    paragraphs: &'a [String],
    entries: &'a [GuideEntry],
    data: Option<&'a Value>,
}

impl<'a> GuideSectionRef<'a> {
    fn from_section(section: &'a GuideSection) -> Self {
        Self {
            title: &section.title,
            kind: section.kind,
            paragraphs: &section.paragraphs,
            entries: &section.entries,
            data: section.data.as_ref(),
        }
    }
}

pub fn lower_output(output: &OutputResult, plan: &RenderPlan) -> Doc {
    let guide = GuideView::try_from_output_result(output);

    match plan.format {
        OutputFormat::Guide => {
            if let Some(guide) = guide.as_ref() {
                lower_guide(
                    guide,
                    plan.format,
                    plan.settings.indent_size,
                    GuidePresentation::Direct,
                    None,
                )
            } else {
                Doc {
                    blocks: lower_value_blocks(&canonical_value(output)),
                }
            }
        }
        OutputFormat::Json => Doc {
            blocks: vec![Block::Json(JsonBlock {
                text: serde_json::to_string_pretty(&json_value(output))
                    .unwrap_or_else(|_| "null".to_string()),
            })],
        },
        OutputFormat::Table => lower_table_doc(output),
        OutputFormat::Markdown => {
            if let Some(guide) = guide.as_ref() {
                lower_guide(
                    guide,
                    plan.format,
                    plan.settings.indent_size,
                    GuidePresentation::Direct,
                    None,
                )
            } else {
                lower_table_doc(output)
            }
        }
        OutputFormat::Value => {
            if let Some(guide) = guide.as_ref() {
                Doc {
                    blocks: vec![Block::List(ListBlock {
                        items: guide.to_value_lines(),
                        indent: 0,
                        inline_markup: false,
                        auto_grid: false,
                    })],
                }
            } else {
                lower_value_doc(output)
            }
        }
        OutputFormat::Mreg => lower_mreg_doc(output),
        OutputFormat::Auto => Doc::default(),
    }
}

fn lower_value_doc(output: &OutputResult) -> Doc {
    let values = output_items_to_rows(&output.items)
        .iter()
        .flat_map(|row| row_value_displays(row, &output.meta.key_index))
        .collect::<Vec<_>>();

    let blocks = match values.len() {
        0 => Vec::new(),
        1 => vec![Block::Paragraph(ParagraphBlock {
            text: values[0].clone(),
            indent: 0,
            inline_markup: false,
        })],
        _ => vec![Block::List(ListBlock {
            items: values,
            indent: 0,
            inline_markup: false,
            auto_grid: false,
        })],
    };

    Doc { blocks }
}

pub(crate) fn lower_guide_help_layout(
    view: &GuideView,
    plan: &RenderPlan,
    layout: HelpLayout,
    show_footer_rule: bool,
) -> Doc {
    lower_guide(
        view,
        plan.format,
        plan.settings.indent_size,
        GuidePresentation::HelpLayout(layout, show_footer_rule),
        Some(&plan.settings.help_chrome),
    )
}

pub fn canonical_value(output: &OutputResult) -> Value {
    output
        .document
        .as_ref()
        .map(|document| document.value.clone())
        .unwrap_or_else(|| output_items_to_value(&output.items))
}

pub(crate) fn json_value(output: &OutputResult) -> Value {
    match &output.items {
        OutputItems::Rows(rows) => {
            Value::Array(rows.iter().cloned().map(Value::Object).collect::<Vec<_>>())
        }
        OutputItems::Groups(groups) => {
            Value::Array(groups.iter().map(json_group_value).collect::<Vec<_>>())
        }
    }
}

fn lower_table_doc(output: &OutputResult) -> Doc {
    match &output.items {
        OutputItems::Rows(rows) => Doc {
            blocks: vec![Block::Table(table_from_rows(rows, &output.meta.key_index))],
        },
        OutputItems::Groups(groups) => {
            let mut blocks = Vec::new();
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    blocks.push(Block::Blank);
                }
                blocks.push(Block::Table(table_from_group(
                    group,
                    &output.meta.key_index,
                )));
            }
            Doc { blocks }
        }
    }
}

fn json_group_value(group: &Group) -> Value {
    let mut item = Row::new();
    item.insert("groups".to_string(), Value::Object(group.groups.clone()));
    item.insert(
        "aggregates".to_string(),
        Value::Object(group.aggregates.clone()),
    );
    item.insert(
        "rows".to_string(),
        Value::Array(
            group
                .rows
                .iter()
                .cloned()
                .map(Value::Object)
                .collect::<Vec<_>>(),
        ),
    );
    Value::Object(item)
}

fn lower_mreg_doc(output: &OutputResult) -> Doc {
    if let Some(guide) = GuideView::try_from_output_result(output)
        .or_else(|| GuideView::try_from_row_projection(output))
    {
        return Doc {
            blocks: lower_mreg_guide_blocks(&guide.to_json_value(), 0),
        };
    }

    match &output.items {
        OutputItems::Rows(rows) if rows.len() == 1 => Doc {
            blocks: vec![Block::KeyValue(key_value_from_map(
                &rows[0],
                Some(&output.meta.key_index),
            ))],
        },
        OutputItems::Rows(rows) if rows.is_empty() => Doc::default(),
        _ => lower_table_doc(output),
    }
}

fn lower_value_blocks(value: &Value) -> Vec<Block> {
    match value {
        Value::Null => Vec::new(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            vec![Block::Paragraph(ParagraphBlock {
                text: display_value(value),
                indent: 0,
                inline_markup: false,
            })]
        }
        Value::Array(items) => {
            if let Some(table) = table_from_value_array(items) {
                return vec![Block::Table(table)];
            }
            vec![Block::List(ListBlock {
                items: items.iter().map(display_value).collect(),
                indent: 0,
                inline_markup: false,
                auto_grid: false,
            })]
        }
        Value::Object(map) => vec![Block::KeyValue(key_value_from_map(map, None))],
    }
}

fn row_value_displays(row: &Row, key_order: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    for key in key_order {
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some(value) = row.get(key)
            && !value.is_null()
        {
            out.push(display_value(value));
        }
    }

    for key in row.keys() {
        if seen.contains(key) {
            continue;
        }
        if let Some(value) = row.get(key)
            && !value.is_null()
        {
            out.push(display_value(value));
        }
    }

    out
}

fn lower_mreg_guide_blocks(value: &Value, indent: usize) -> Vec<Block> {
    match value {
        Value::Object(map) => ordered_keys_for_map(map)
            .into_iter()
            .flat_map(|key| {
                map.get(&key)
                    .map(|value| lower_mreg_guide_named_value(&key, value, indent))
                    .unwrap_or_default()
            })
            .collect(),
        _ => vec![Block::Paragraph(ParagraphBlock {
            text: display_value(value),
            indent,
            inline_markup: false,
        })],
    }
}

fn lower_mreg_guide_named_value(key: &str, value: &Value, indent: usize) -> Vec<Block> {
    match value {
        Value::Null => vec![mreg_line_block(format!("{key}:"), indent)],
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            vec![mreg_line_block(
                format!("{key}: {}", display_value(value)),
                indent,
            )]
        }
        Value::Array(items) if items.is_empty() => {
            vec![mreg_line_block(format!("{key}: []"), indent)]
        }
        Value::Array(items)
            if items
                .iter()
                .all(|item| !matches!(item, Value::Object(_) | Value::Array(_))) =>
        {
            if items.len() == 1 {
                vec![mreg_line_block(
                    format!("{key}: {}", display_value(&items[0])),
                    indent,
                )]
            } else {
                let mut blocks = vec![mreg_line_block(format!("{key} ({}):", items.len()), indent)];
                blocks.extend(
                    items
                        .iter()
                        .map(|item| mreg_line_block(display_value(item), indent + 2)),
                );
                blocks
            }
        }
        Value::Array(items) => {
            let mut blocks = vec![mreg_line_block(format!("{key} ({}):", items.len()), indent)];
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    blocks.push(mreg_line_block("---".to_string(), indent + 2));
                }
                blocks.extend(lower_mreg_guide_blocks(item, indent + 2));
            }
            blocks
        }
        Value::Object(map) => {
            let mut blocks = vec![mreg_line_block(format!("{key}:"), indent)];
            blocks.extend(
                ordered_keys_for_map(map)
                    .into_iter()
                    .flat_map(|nested_key| {
                        map.get(&nested_key)
                            .map(|nested| {
                                lower_mreg_guide_named_value(&nested_key, nested, indent + 2)
                            })
                            .unwrap_or_default()
                    }),
            );
            blocks
        }
    }
}

fn ordered_keys_for_map(map: &Map<String, Value>) -> Vec<String> {
    let mut keys = map.keys().cloned().collect::<Vec<_>>();
    keys.sort_by_key(|key| match key.as_str() {
        "preamble" => (0usize, key.clone()),
        "usage" => (1, key.clone()),
        "commands" => (2, key.clone()),
        "arguments" => (3, key.clone()),
        "options" => (4, key.clone()),
        "common_invocation_options" => (5, key.clone()),
        "notes" => (6, key.clone()),
        "sections" => (7, key.clone()),
        "epilogue" => (8, key.clone()),
        _ => (9, key.clone()),
    });
    keys
}

fn mreg_line_block(text: String, indent: usize) -> Block {
    Block::Paragraph(ParagraphBlock {
        text,
        indent,
        inline_markup: false,
    })
}

fn lower_guide(
    view: &GuideView,
    format: OutputFormat,
    indent_size: usize,
    presentation: GuidePresentation,
    help_chrome: Option<&ResolvedHelpChromeSettings>,
) -> Doc {
    let mut blocks = guide_paragraph_blocks(&view.preamble);
    let sections = guide_sections(view);

    match presentation {
        GuidePresentation::HelpLayout(layout, show_footer_rule) => {
            let Some(help_chrome) = help_chrome else {
                return Doc::default();
            };
            extend_with_blank_between(
                &mut blocks,
                help_layout_blocks(&sections, layout, show_footer_rule, help_chrome),
            );
        }
        GuidePresentation::Direct => {
            extend_with_blank_between(
                &mut blocks,
                direct_guide_blocks(&sections, format, indent_size),
            );
        }
    }

    extend_with_blank_between(&mut blocks, guide_paragraph_blocks(&view.epilogue));
    Doc { blocks }
}

fn guide_sections(view: &GuideView) -> Vec<GuideSectionRef<'_>> {
    let use_ordered_sections = uses_ordered_section_representation(view);
    let mut sections = Vec::new();
    let builtin_sections = [
        (
            GuideSectionKind::Usage,
            "Usage",
            view.usage.as_slice(),
            &[][..],
        ),
        (
            GuideSectionKind::Commands,
            "Commands",
            &[][..],
            view.commands.as_slice(),
        ),
        (
            GuideSectionKind::Arguments,
            "Arguments",
            &[][..],
            view.arguments.as_slice(),
        ),
        (
            GuideSectionKind::Options,
            "Options",
            &[][..],
            view.options.as_slice(),
        ),
        (
            GuideSectionKind::CommonInvocationOptions,
            "Common Invocation Options",
            &[][..],
            view.common_invocation_options.as_slice(),
        ),
        (
            GuideSectionKind::Notes,
            "Notes",
            view.notes.as_slice(),
            &[][..],
        ),
    ];
    for (kind, title, paragraphs, entries) in builtin_sections {
        if (paragraphs.is_empty() && entries.is_empty())
            || (use_ordered_sections && has_canonical_builtin_section_kind(view, kind))
        {
            continue;
        }

        sections.push(GuideSectionRef {
            title,
            kind,
            paragraphs,
            entries,
            data: None,
        });
    }

    for section in &view.sections {
        if !use_ordered_sections && is_canonical_builtin_section(section) {
            continue;
        }
        sections.push(GuideSectionRef::from_section(section));
    }

    sections
}

fn direct_guide_blocks(
    sections: &[GuideSectionRef<'_>],
    format: OutputFormat,
    indent_size: usize,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    let title_chrome = if matches!(format, OutputFormat::Guide | OutputFormat::Markdown) {
        SectionTitleChrome::Ruled
    } else {
        SectionTitleChrome::Plain
    };
    let body_indent = if matches!(format, OutputFormat::Guide) {
        indent_size
    } else {
        0
    };
    let trailing_newline = matches!(format, OutputFormat::Guide);
    for section in sections {
        let section_blocks = direct_section_blocks(*section, format);
        if section_blocks.is_empty() {
            continue;
        }

        extend_with_blank_between(
            &mut blocks,
            vec![Block::Section(SectionBlock {
                title: Some(section.title.to_string()),
                title_chrome,
                body_indent,
                inline_title_suffix: None,
                trailing_newline,
                blocks: section_blocks,
            })],
        );
    }
    blocks
}

fn direct_section_blocks(section: GuideSectionRef<'_>, format: OutputFormat) -> Vec<Block> {
    let mut section_blocks = guide_paragraph_blocks(section.paragraphs);
    let entry_blocks = entry_blocks(section.entries, format);
    extend_with_blank_between(&mut section_blocks, entry_blocks);

    if let Some(data) = section.data {
        extend_with_blank_between(&mut section_blocks, lower_value_blocks(data));
    }

    section_blocks
}

fn extend_with_blank_between(blocks: &mut Vec<Block>, mut extra: Vec<Block>) {
    if extra.is_empty() {
        return;
    }
    if !blocks.is_empty() {
        blocks.push(Block::Blank);
    }
    blocks.append(&mut extra);
}

fn guide_paragraph_blocks(paragraphs: &[String]) -> Vec<Block> {
    trimmed_lines(paragraphs)
        .into_iter()
        .map(|text| {
            Block::Paragraph(ParagraphBlock {
                text,
                indent: 0,
                inline_markup: true,
            })
        })
        .collect()
}

fn entry_blocks(entries: &[GuideEntry], format: OutputFormat) -> Vec<Block> {
    if entries.is_empty() {
        return Vec::new();
    }
    if matches!(format, OutputFormat::Guide) {
        vec![Block::GuideEntries(guide_entries_block(entries))]
    } else {
        vec![Block::KeyValue(KeyValueBlock {
            style: KeyValueStyle::Bulleted,
            rows: guide_entry_key_value_rows(entries),
        })]
    }
}

fn help_layout_blocks(
    sections: &[GuideSectionRef<'_>],
    layout: HelpLayout,
    show_footer_rule: bool,
    help_chrome: &ResolvedHelpChromeSettings,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    for section in sections {
        let Some(section_block) = help_layout_section_block(*section, layout, help_chrome) else {
            continue;
        };
        push_blank_lines(&mut blocks, help_chrome.section_spacing);
        blocks.push(Block::Section(section_block));
    }
    if show_footer_rule && !blocks.is_empty() {
        blocks.push(Block::Rule);
    }
    blocks
}

fn help_layout_section_block(
    section: GuideSectionRef<'_>,
    layout: HelpLayout,
    help_chrome: &ResolvedHelpChromeSettings,
) -> Option<SectionBlock> {
    let paragraph_indent = if section.kind == GuideSectionKind::Usage
        && section.title.trim_end_matches(':') != "Usage"
    {
        0
    } else {
        2
    };
    let lines = trimmed_lines(section.paragraphs);
    let inline_title_suffix = compact_usage_title_suffix(layout, section, &lines);
    let mut blocks = if lines.is_empty() || inline_title_suffix.is_some() {
        Vec::new()
    } else {
        vec![Block::Paragraph(ParagraphBlock {
            text: lines.join("\n"),
            indent: paragraph_indent,
            inline_markup: true,
        })]
    };

    if !section.entries.is_empty() {
        extend_with_blank_between(
            &mut blocks,
            vec![Block::GuideEntries(guide_entries_block_with_defaults(
                section.entries,
                help_chrome.entry_indent,
                help_chrome.entry_gap,
            ))],
        );
    }
    if let Some(data) = section.data {
        extend_with_blank_between(
            &mut blocks,
            help_layout_blocks_from_value(data, help_chrome),
        );
    }
    if blocks.is_empty() && inline_title_suffix.is_none() {
        None
    } else {
        Some(SectionBlock {
            title: Some(section.title.to_string()),
            title_chrome: help_layout_title_chrome(layout),
            body_indent: 0,
            inline_title_suffix,
            trailing_newline: false,
            blocks,
        })
    }
}

fn help_layout_blocks_from_value(
    value: &Value,
    help_chrome: &ResolvedHelpChromeSettings,
) -> Vec<Block> {
    match value {
        Value::Object(map) if should_render_as_key_value_group(map) => {
            vec![Block::KeyValue(key_value_from_guide_data_map(map))]
        }
        Value::Object(map) => vec![Block::KeyValue(KeyValueBlock {
            style: KeyValueStyle::Plain,
            rows: key_value_from_map(map, None).rows,
        })],
        Value::Array(items) if items.is_empty() => Vec::new(),
        Value::Array(items) => {
            if let Some(entries) = items
                .iter()
                .map(guide_entry_from_value)
                .collect::<Option<Vec<_>>>()
            {
                vec![Block::GuideEntries(guide_entries_block_with_defaults(
                    &entries,
                    help_chrome.entry_indent,
                    help_chrome.entry_gap,
                ))]
            } else if items.iter().all(is_scalar_like) {
                vec![Block::List(ListBlock {
                    items: items.iter().map(display_value).collect(),
                    indent: 2,
                    inline_markup: true,
                    auto_grid: true,
                })]
            } else if let Some(table) = table_from_value_array(items) {
                vec![Block::Table(table)]
            } else {
                vec![Block::List(ListBlock {
                    items: items.iter().map(display_value).collect(),
                    indent: 0,
                    inline_markup: false,
                    auto_grid: false,
                })]
            }
        }
        Value::Null => Vec::new(),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            vec![Block::Paragraph(ParagraphBlock {
                text: display_value(value),
                indent: 2,
                inline_markup: true,
            })]
        }
    }
}

fn compact_usage_title_suffix(
    layout: HelpLayout,
    section: GuideSectionRef<'_>,
    lines: &[String],
) -> Option<String> {
    if !matches!(layout, HelpLayout::Compact | HelpLayout::Minimal)
        || section.title.trim_end_matches(':') != "Usage"
        || lines.len() != 1
        || !section.entries.is_empty()
        || !matches!(section.data, None | Some(Value::Null))
    {
        return None;
    }
    Some(visible_inline_text(&lines[0]))
}

fn push_blank_lines(blocks: &mut Vec<Block>, count: usize) {
    if blocks.is_empty() {
        return;
    }
    blocks.extend((0..count).map(|_| Block::Blank));
}

fn key_value_from_map(
    map: &Map<String, Value>,
    preferred_keys: Option<&[String]>,
) -> KeyValueBlock {
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(order) = preferred_keys {
        for key in order {
            if seen.insert(key.clone()) && let Some(value) = map.get(key) {
                rows.push(KeyValueRow {
                    key: key.clone(),
                    value: display_value(value),
                    indent: None,
                    gap: None,
                });
            }
        }
    }
    for (key, value) in map {
        if seen.insert(key.clone()) {
            rows.push(KeyValueRow {
                key: key.clone(),
                value: display_value(value),
                indent: None,
                gap: None,
            });
        }
    }
    KeyValueBlock {
        style: KeyValueStyle::Plain,
        rows,
    }
}

fn table_from_rows(rows: &[Row], key_index: &[String]) -> TableBlock {
    let headers = headers_for_rows(rows, key_index);
    TableBlock {
        summary: Vec::new(),
        rows: rows
            .iter()
            .map(|row| {
                headers
                    .iter()
                    .map(|header| row.get(header).map(display_value).unwrap_or_default())
                    .collect()
            })
            .collect(),
        headers,
    }
}

fn table_from_group(group: &Group, preferred_keys: &[String]) -> TableBlock {
    let merged_rows = group_rows(group);
    let mut summary = Vec::new();
    let mut seen = BTreeSet::new();
    for key in preferred_keys {
        if seen.insert(key.clone())
            && let Some(value) = group.groups.get(key).or_else(|| group.aggregates.get(key))
        {
            summary.push(KeyValueRow {
                key: key.clone(),
                value: display_value(value),
                indent: None,
                gap: None,
            });
        }
    }
    for (key, value) in &group.groups {
        if seen.insert(key.clone()) {
            summary.push(KeyValueRow {
                key: key.clone(),
                value: display_value(value),
                indent: None,
                gap: None,
            });
        }
    }
    for (key, value) in &group.aggregates {
        if seen.insert(key.clone()) {
            summary.push(KeyValueRow {
                key: key.clone(),
                value: display_value(value),
                indent: None,
                gap: None,
            });
        }
    }
    let mut table = table_from_rows(&merged_rows, preferred_keys);
    table.summary = summary;
    table
}

fn headers_for_rows(rows: &[Row], preferred_keys: &[String]) -> Vec<String> {
    let mut headers = Vec::new();
    let mut seen = BTreeSet::new();
    for key in preferred_keys {
        if seen.insert(key.clone()) {
            headers.push(key.clone());
        }
    }
    for row in rows {
        for key in row.keys() {
            if seen.insert(key.clone()) {
                headers.push(key.clone());
            }
        }
    }
    headers
}

fn table_from_value_array(items: &[Value]) -> Option<TableBlock> {
    let mut rows = Vec::new();
    for item in items {
        let Value::Object(map) = item else {
            return None;
        };
        rows.push(map.clone());
    }
    Some(table_from_rows(&rows, &[]))
}

fn display_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
        }
    }
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
        display_indent: Some("  ".to_string()),
        display_gap: None,
    })
}

fn key_value_from_guide_data_map(map: &Map<String, Value>) -> KeyValueBlock {
    KeyValueBlock {
        style: KeyValueStyle::Plain,
        rows: map
            .iter()
            .map(|(key, value)| KeyValueRow {
                key: key.clone(),
                value: guide_data_display_value(value),
                indent: Some("  ".to_string()),
                gap: None,
            })
            .collect(),
    }
}

fn trimmed_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn guide_entries_block(entries: &[GuideEntry]) -> GuideEntriesBlock {
    guide_entries_block_with_defaults(entries, 0, None)
}

fn guide_entries_block_with_defaults(
    entries: &[GuideEntry],
    entry_indent: usize,
    entry_gap: Option<usize>,
) -> GuideEntriesBlock {
    GuideEntriesBlock {
        default_indent: " ".repeat(entry_indent),
        default_gap: entry_gap.map(|value| " ".repeat(value)),
        rows: entries.iter().map(guide_entry_row).collect(),
    }
}

fn help_layout_title_chrome(layout: HelpLayout) -> SectionTitleChrome {
    match layout {
        HelpLayout::Full => SectionTitleChrome::Ruled,
        HelpLayout::Compact | HelpLayout::Minimal => SectionTitleChrome::Plain,
    }
}

fn guide_entry_key_value_rows(entries: &[GuideEntry]) -> Vec<KeyValueRow> {
    entries
        .iter()
        .map(|entry| KeyValueRow {
            key: entry.name.clone(),
            value: entry.short_help.clone(),
            indent: None,
            gap: None,
        })
        .collect()
}

fn guide_entry_row(entry: &GuideEntry) -> GuideEntryRow {
    GuideEntryRow {
        key: entry.name.clone(),
        value: entry.short_help.clone(),
        indent_hint: entry.display_indent.clone(),
        gap_hint: entry.display_gap.clone(),
    }
}

fn guide_data_display_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Array(items) if items.is_empty() => "[]".to_string(),
        _ => display_value(value),
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

fn is_scalar_like(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn has_canonical_builtin_section_kind(view: &GuideView, kind: GuideSectionKind) -> bool {
    view.sections
        .iter()
        .any(|section| section.kind == kind && is_canonical_builtin_section(section))
}

fn uses_ordered_section_representation(view: &GuideView) -> bool {
    view.sections.iter().any(|section| {
        !is_canonical_builtin_section(section)
            || canonical_section_owns_ordered_content(view, section)
    })
}

fn canonical_section_owns_ordered_content(view: &GuideView, section: &GuideSection) -> bool {
    if !matches!(section.data, None | Some(Value::Null)) {
        return true;
    }

    match section.kind {
        GuideSectionKind::Usage => !section.paragraphs.is_empty() && view.usage.is_empty(),
        GuideSectionKind::Commands => !section.entries.is_empty() && view.commands.is_empty(),
        GuideSectionKind::Arguments => !section.entries.is_empty() && view.arguments.is_empty(),
        GuideSectionKind::Options => !section.entries.is_empty() && view.options.is_empty(),
        GuideSectionKind::CommonInvocationOptions => {
            !section.entries.is_empty() && view.common_invocation_options.is_empty()
        }
        GuideSectionKind::Notes => !section.paragraphs.is_empty() && view.notes.is_empty(),
        GuideSectionKind::Custom => false,
    }
}

fn is_canonical_builtin_section(section: &GuideSection) -> bool {
    let expected = match section.kind {
        GuideSectionKind::Usage => "Usage",
        GuideSectionKind::Commands => "Commands",
        GuideSectionKind::Arguments => "Arguments",
        GuideSectionKind::Options => "Options",
        GuideSectionKind::CommonInvocationOptions => "Common Invocation Options",
        GuideSectionKind::Notes => "Notes",
        GuideSectionKind::Custom => return false,
    };
    section.title.trim().eq_ignore_ascii_case(expected)
}
