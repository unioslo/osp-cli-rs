pub(crate) mod template;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use unicode_width::UnicodeWidthStr;

use crate::core::command_def::{ArgDef, CommandDef, FlagDef};
use crate::core::output_model::{OutputItems, OutputResult, RenderRecommendation};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GuideDoc {
    pub preamble: Vec<String>,
    pub sections: Vec<GuideSection>,
    pub epilogue: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuideSection {
    pub title: String,
    pub kind: GuideSectionKind,
    pub blocks: Vec<GuideBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideSectionKind {
    Usage,
    Commands,
    Options,
    Arguments,
    CommonInvocationOptions,
    Notes,
    Custom,
}

impl Default for GuideSectionKind {
    fn default() -> Self {
        Self::Custom
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuideBlock {
    Paragraph { text: String },
    Entry(GuideEntry),
    Blank,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuideEntry {
    pub indent: String,
    pub head: String,
    pub tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuidePayload {
    pub preamble: Vec<String>,
    pub usage: Vec<String>,
    pub commands: Vec<GuidePayloadEntry>,
    pub arguments: Vec<GuidePayloadEntry>,
    pub options: Vec<GuidePayloadEntry>,
    pub common_invocation_options: Vec<GuidePayloadEntry>,
    pub notes: Vec<String>,
    pub sections: Vec<GuidePayloadSection>,
    pub epilogue: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GuidePayloadEntry {
    pub name: String,
    pub short_help: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuidePayloadSection {
    pub title: String,
    pub kind: GuideSectionKind,
    pub paragraphs: Vec<String>,
    pub entries: Vec<GuidePayloadEntry>,
}

pub type HelpDoc = GuideDoc;
pub type HelpPayload = GuidePayload;
pub type HelpSection = GuideSection;
pub type HelpSectionKind = GuideSectionKind;
pub type HelpBlock = GuideBlock;
pub type HelpEntry = GuideEntry;

impl From<GuideDoc> for GuidePayload {
    fn from(value: GuideDoc) -> Self {
        Self::from_doc(&value)
    }
}

impl From<&GuideDoc> for GuidePayload {
    fn from(value: &GuideDoc) -> Self {
        Self::from_doc(value)
    }
}

impl GuideDoc {
    pub fn from_text(help_text: &str) -> Self {
        parse_help_doc(help_text)
    }

    pub fn from_command_def(command: &CommandDef) -> Self {
        GuidePayload::from_command_def(command).to_doc()
    }

    pub fn from_payload(payload: GuidePayload) -> Self {
        payload.to_doc()
    }

    pub fn to_payload(&self) -> GuidePayload {
        GuidePayload::from_doc(self)
    }

    pub fn to_output_result(&self) -> OutputResult {
        self.to_payload().to_output_result()
    }

    pub fn to_json_value(&self) -> Value {
        self.to_payload().to_json_value()
    }
}

impl GuidePayload {
    pub fn from_command_def(command: &CommandDef) -> Self {
        guide_payload_from_command_def(command)
    }

    pub fn from_doc(doc: &GuideDoc) -> Self {
        let mut payload = GuidePayload {
            preamble: doc.preamble.clone(),
            epilogue: doc.epilogue.clone(),
            ..Self::default()
        };

        for section in &doc.sections {
            match section.kind {
                GuideSectionKind::Usage => {
                    payload.usage.extend(section_paragraphs(section));
                }
                GuideSectionKind::Commands => {
                    payload.commands.extend(section_entries(section));
                }
                GuideSectionKind::Arguments => {
                    payload.arguments.extend(section_entries(section));
                }
                GuideSectionKind::Options => {
                    payload.options.extend(section_entries(section));
                }
                GuideSectionKind::CommonInvocationOptions => {
                    payload
                        .common_invocation_options
                        .extend(section_entries(section));
                }
                GuideSectionKind::Notes => {
                    payload.notes.extend(section_paragraphs(section));
                }
                GuideSectionKind::Custom => {
                    payload.sections.push(custom_section_payload(section));
                }
            }
        }

        payload
    }

    pub fn to_output_result(&self) -> OutputResult {
        let mut output = OutputResult::from_rows(vec![self.to_row()]);
        output.meta.render_recommendation = Some(RenderRecommendation::Guide);
        output
    }

    pub fn to_json_value(&self) -> Value {
        Value::Object(self.to_row())
    }

    pub fn try_from_output_result(output: &OutputResult) -> Option<Self> {
        let rows = match &output.items {
            OutputItems::Rows(rows) if rows.len() == 1 => rows,
            _ => return None,
        };
        Self::try_from_row(&rows[0])
    }

    pub fn to_doc(&self) -> GuideDoc {
        let mut doc = GuideDoc {
            preamble: self.preamble.clone(),
            sections: Vec::new(),
            epilogue: self.epilogue.clone(),
        };

        if let Some(section) = usage_section_from_payload(&self.usage) {
            doc.sections.push(section);
        }
        if let Some(section) =
            entry_section_from_payload("Commands", GuideSectionKind::Commands, &self.commands)
        {
            doc.sections.push(section);
        }
        if let Some(section) =
            entry_section_from_payload("Arguments", GuideSectionKind::Arguments, &self.arguments)
        {
            doc.sections.push(section);
        }
        if let Some(section) =
            entry_section_from_payload("Options", GuideSectionKind::Options, &self.options)
        {
            doc.sections.push(section);
        }
        if let Some(section) = entry_section_from_payload(
            "Common Invocation Options",
            GuideSectionKind::CommonInvocationOptions,
            &self.common_invocation_options,
        ) {
            doc.sections.push(section);
        }
        if let Some(section) = notes_section_from_payload(&self.notes) {
            doc.sections.push(section);
        }
        doc.sections
            .extend(self.sections.iter().map(GuidePayloadSection::to_doc));

        doc
    }

    pub fn to_markdown(&self) -> String {
        self.to_markdown_with_width(None)
    }

    pub fn to_markdown_with_width(&self, width: Option<usize>) -> String {
        let mut sections = Vec::new();

        push_markdown_paragraphs(&mut sections, &self.preamble);
        push_markdown_text_section(&mut sections, "Usage", &self.usage);
        push_markdown_entry_section(&mut sections, "Commands", &self.commands, width);
        push_markdown_entry_section(&mut sections, "Arguments", &self.arguments, width);
        push_markdown_entry_section(&mut sections, "Options", &self.options, width);
        push_markdown_entry_section(
            &mut sections,
            "Common Invocation Options",
            &self.common_invocation_options,
            width,
        );
        push_markdown_text_section(&mut sections, "Notes", &self.notes);

        for section in &self.sections {
            let mut lines = vec![format!("## {}", section.title)];
            if !section.paragraphs.is_empty() {
                lines.push(String::new());
                lines.extend(section.paragraphs.iter().cloned());
            }
            if !section.entries.is_empty() {
                if lines.len() > 1 || !section.paragraphs.is_empty() {
                    lines.push(String::new());
                }
                lines.extend(markdown_entry_table(&section.entries, width));
            }
            sections.push(lines.join("\n"));
        }

        push_markdown_paragraphs(&mut sections, &self.epilogue);

        let mut rendered = sections.join("\n\n");
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered
    }

    pub fn merge(&mut self, mut other: GuidePayload) {
        self.preamble.append(&mut other.preamble);
        self.usage.append(&mut other.usage);
        self.commands.append(&mut other.commands);
        self.arguments.append(&mut other.arguments);
        self.options.append(&mut other.options);
        self.common_invocation_options
            .append(&mut other.common_invocation_options);
        self.notes.append(&mut other.notes);
        self.sections.append(&mut other.sections);
        self.epilogue.append(&mut other.epilogue);
    }
}

impl GuidePayload {
    fn to_row(&self) -> Map<String, Value> {
        let mut row = Map::new();

        if !self.preamble.is_empty() {
            row.insert("preamble".to_string(), string_array(&self.preamble));
        }

        if !self.usage.is_empty() {
            row.insert("usage".to_string(), string_array(&self.usage));
        }
        if !self.commands.is_empty() {
            row.insert("commands".to_string(), payload_entry_array(&self.commands));
        }
        if !self.arguments.is_empty() {
            row.insert(
                "arguments".to_string(),
                payload_entry_array(&self.arguments),
            );
        }
        if !self.options.is_empty() {
            row.insert("options".to_string(), payload_entry_array(&self.options));
        }
        if !self.common_invocation_options.is_empty() {
            row.insert(
                "common_invocation_options".to_string(),
                payload_entry_array(&self.common_invocation_options),
            );
        }
        if !self.notes.is_empty() {
            row.insert("notes".to_string(), string_array(&self.notes));
        }
        if !self.sections.is_empty() {
            row.insert(
                "sections".to_string(),
                Value::Array(
                    self.sections
                        .iter()
                        .map(GuidePayloadSection::to_value)
                        .collect(),
                ),
            );
        }
        if !self.epilogue.is_empty() {
            row.insert("epilogue".to_string(), string_array(&self.epilogue));
        }

        row
    }

    fn try_from_row(row: &Map<String, Value>) -> Option<Self> {
        Some(Self {
            preamble: row_string_array(row.get("preamble"))?,
            usage: row_string_array(row.get("usage"))?,
            commands: payload_entries(row.get("commands"))?,
            arguments: payload_entries(row.get("arguments"))?,
            options: payload_entries(row.get("options"))?,
            common_invocation_options: payload_entries(row.get("common_invocation_options"))?,
            notes: row_string_array(row.get("notes"))?,
            sections: payload_sections(row.get("sections"))?,
            epilogue: row_string_array(row.get("epilogue"))?,
        })
    }
}

impl GuidePayloadSection {
    fn to_doc(&self) -> GuideSection {
        if matches!(
            self.kind,
            GuideSectionKind::Commands
                | GuideSectionKind::Arguments
                | GuideSectionKind::Options
                | GuideSectionKind::CommonInvocationOptions
        ) && !self.entries.is_empty()
        {
            if let Some(mut section) =
                entry_section_from_payload(&self.title, self.kind, &self.entries)
            {
                for paragraph in self.paragraphs.iter().rev() {
                    section.blocks.insert(
                        0,
                        GuideBlock::Paragraph {
                            text: paragraph.clone(),
                        },
                    );
                }
                return section;
            }
        }

        let mut section = GuideSection::new(self.title.clone(), self.kind);
        for paragraph in &self.paragraphs {
            section = section.paragraph(paragraph.clone());
        }
        for entry in &self.entries {
            section = section.entry(entry.name.clone(), entry.short_help.clone());
        }
        section
    }

    fn to_value(&self) -> Value {
        json!({
            "title": self.title,
            "kind": self.kind.as_str(),
            "paragraphs": self.paragraphs,
            "entries": self.entries.iter().map(payload_entry_value).collect::<Vec<_>>(),
        })
    }
}

impl GuideSection {
    pub fn new(title: impl Into<String>, kind: GuideSectionKind) -> Self {
        Self {
            title: title.into(),
            kind,
            blocks: Vec::new(),
        }
    }

    pub fn paragraph(mut self, text: impl Into<String>) -> Self {
        self.blocks
            .push(GuideBlock::Paragraph { text: text.into() });
        self
    }

    pub fn blank(mut self) -> Self {
        self.blocks.push(GuideBlock::Blank);
        self
    }

    pub fn entry(mut self, head: impl Into<String>, tail: impl Into<String>) -> Self {
        self.blocks.push(GuideBlock::Entry(GuideEntry {
            indent: "  ".to_string(),
            head: head.into(),
            tail: tail.into(),
        }));
        self
    }
}

impl GuideSectionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            GuideSectionKind::Usage => "usage",
            GuideSectionKind::Commands => "commands",
            GuideSectionKind::Options => "options",
            GuideSectionKind::Arguments => "arguments",
            GuideSectionKind::CommonInvocationOptions => "common_invocation_options",
            GuideSectionKind::Notes => "notes",
            GuideSectionKind::Custom => "custom",
        }
    }
}

fn string_array(values: &[String]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| Value::String(value.trim().to_string()))
            .collect(),
    )
}

fn section_paragraphs(section: &GuideSection) -> Vec<String> {
    section
        .blocks
        .iter()
        .filter_map(|block| match block {
            GuideBlock::Paragraph { text } => Some(text.trim().to_string()),
            GuideBlock::Entry(_) | GuideBlock::Blank => None,
        })
        .collect()
}

fn section_entries(section: &GuideSection) -> Vec<GuidePayloadEntry> {
    section
        .blocks
        .iter()
        .filter_map(|block| match block {
            GuideBlock::Entry(entry) => Some(GuidePayloadEntry {
                name: entry.head.trim().to_string(),
                short_help: entry.tail.trim().to_string(),
            }),
            GuideBlock::Paragraph { .. } | GuideBlock::Blank => None,
        })
        .collect()
}

fn custom_section_payload(section: &GuideSection) -> GuidePayloadSection {
    GuidePayloadSection {
        title: section.title.clone(),
        kind: section.kind,
        paragraphs: section_paragraphs(section),
        entries: section_entries(section),
    }
}

fn row_string_array(value: Option<&Value>) -> Option<Vec<String>> {
    let Some(value) = value else {
        return Some(Vec::new());
    };
    let Value::Array(values) = value else {
        return None;
    };
    values
        .iter()
        .map(|value| value.as_str().map(ToOwned::to_owned))
        .collect()
}

fn payload_entry_value(entry: &GuidePayloadEntry) -> Value {
    json!({
        "name": entry.name,
        "short_help": entry.short_help,
    })
}

fn payload_entry_array(entries: &[GuidePayloadEntry]) -> Value {
    Value::Array(entries.iter().map(payload_entry_value).collect())
}

fn payload_entries(value: Option<&Value>) -> Option<Vec<GuidePayloadEntry>> {
    let Some(value) = value else {
        return Some(Vec::new());
    };
    let Value::Array(entries) = value else {
        return None;
    };

    let mut out = Vec::new();
    for entry in entries {
        let Value::Object(entry) = entry else {
            return None;
        };
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let short_help = entry
            .get("short_help")
            .or_else(|| entry.get("summary"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        out.push(GuidePayloadEntry { name, short_help });
    }
    Some(out)
}

fn payload_sections(value: Option<&Value>) -> Option<Vec<GuidePayloadSection>> {
    let Some(value) = value else {
        return Some(Vec::new());
    };
    let Value::Array(sections) = value else {
        return None;
    };

    let mut out = Vec::new();
    for section in sections {
        let Value::Object(section) = section else {
            return None;
        };
        let title = section.get("title")?.as_str()?.to_string();
        let kind = match section
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("custom")
        {
            "custom" => GuideSectionKind::Custom,
            "notes" => GuideSectionKind::Notes,
            "usage" => GuideSectionKind::Usage,
            "commands" => GuideSectionKind::Commands,
            "arguments" => GuideSectionKind::Arguments,
            "options" => GuideSectionKind::Options,
            "common_invocation_options" => GuideSectionKind::CommonInvocationOptions,
            _ => return None,
        };
        out.push(GuidePayloadSection {
            title,
            kind,
            paragraphs: row_string_array(section.get("paragraphs"))?,
            entries: payload_entries(section.get("entries"))?,
        });
    }
    Some(out)
}

fn usage_section_from_payload(lines: &[String]) -> Option<GuideSection> {
    if lines.is_empty() {
        return None;
    }
    let mut section = GuideSection::new("Usage", GuideSectionKind::Usage);
    for line in lines.iter().cloned() {
        section = section.paragraph(format!("  {}", line.trim()));
    }
    Some(section)
}

fn notes_section_from_payload(lines: &[String]) -> Option<GuideSection> {
    if lines.is_empty() {
        return None;
    }
    let mut section = GuideSection::new("Notes", GuideSectionKind::Notes);
    for line in lines.iter().cloned() {
        section = section.paragraph(format!("  {}", line.trim()));
    }
    Some(section)
}

fn entry_section_from_payload(
    title: &str,
    kind: GuideSectionKind,
    entries: &[GuidePayloadEntry],
) -> Option<GuideSection> {
    if entries.is_empty() {
        return None;
    }

    let mut section = GuideSection::new(title, kind);
    let max_name_width = entries
        .iter()
        .map(|entry| entry.name.chars().count())
        .max()
        .unwrap_or(0);
    for entry in entries {
        let gap = if entry.short_help.is_empty() {
            String::new()
        } else {
            " ".repeat(max_name_width.saturating_sub(entry.name.chars().count()) + 2)
        };
        section = section.entry(entry.name.clone(), format!("{gap}{}", entry.short_help));
    }
    Some(section)
}

fn push_markdown_paragraphs(sections: &mut Vec<String>, paragraphs: &[String]) {
    if paragraphs.is_empty() {
        return;
    }
    sections.push(paragraphs.join("\n"));
}

fn push_markdown_text_section(sections: &mut Vec<String>, title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }

    sections.push(format!("## {title}\n\n{}", lines.join("\n")));
}

fn push_markdown_entry_section(
    sections: &mut Vec<String>,
    title: &str,
    entries: &[GuidePayloadEntry],
    width: Option<usize>,
) {
    if entries.is_empty() {
        return;
    }

    let mut lines = vec![format!("## {title}"), String::new()];
    lines.extend(markdown_entry_table(entries, width));
    sections.push(lines.join("\n"));
}

fn markdown_entry_table(entries: &[GuidePayloadEntry], width: Option<usize>) -> Vec<String> {
    let name_header = "name";
    let help_header = "short_help";
    let escaped_rows = entries
        .iter()
        .map(|entry| {
            (
                markdown_table_cell(&entry.name),
                markdown_table_cell(&entry.short_help),
            )
        })
        .collect::<Vec<_>>();
    let name_width = escaped_rows
        .iter()
        .map(|(name, _)| UnicodeWidthStr::width(name.as_str()))
        .max()
        .unwrap_or(0)
        .max(UnicodeWidthStr::width(name_header));
    let help_width = escaped_rows
        .iter()
        .map(|(_, help)| UnicodeWidthStr::width(help.as_str()))
        .max()
        .unwrap_or(0)
        .max(UnicodeWidthStr::width(help_header));
    let bounded_help_width =
        markdown_bounded_help_width(width, name_width, help_width, help_header, &escaped_rows);

    let mut lines = vec![
        format!(
            "| {} | {} |",
            pad_markdown_cell(name_header, name_width),
            pad_markdown_cell(help_header, bounded_help_width)
        ),
        format!(
            "|{}|{}|",
            "-".repeat(name_width + 2),
            "-".repeat(bounded_help_width + 2)
        ),
    ];

    for (name, short_help) in escaped_rows {
        lines.push(format!(
            "| {} | {} |",
            pad_markdown_cell(&name, name_width),
            pad_markdown_cell(&short_help, bounded_help_width)
        ));
    }

    lines
}

fn markdown_bounded_help_width(
    table_width: Option<usize>,
    name_width: usize,
    fallback_help_width: usize,
    help_header: &str,
    rows: &[(String, String)],
) -> usize {
    let Some(table_width) = table_width else {
        return fallback_help_width;
    };

    let min_help_width = UnicodeWidthStr::width(help_header);
    let available_help_width = table_width.saturating_sub(name_width + 7);
    if available_help_width <= min_help_width {
        return min_help_width;
    }

    rows.iter()
        .map(|(_, help)| UnicodeWidthStr::width(help.as_str()))
        .filter(|width| *width <= available_help_width)
        .max()
        .unwrap_or(min_help_width)
        .max(min_help_width)
}

fn markdown_table_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "<br>")
}

fn pad_markdown_cell(value: &str, width: usize) -> String {
    let current = UnicodeWidthStr::width(value);
    if current >= width {
        return value.to_string();
    }
    format!("{value}{}", " ".repeat(width - current))
}

fn guide_payload_from_command_def(command: &CommandDef) -> GuidePayload {
    let usage = command
        .usage
        .clone()
        .or_else(|| default_usage(command))
        .map(|usage| vec![format!("  {usage}")])
        .unwrap_or_default();

    let visible_subcommands = command
        .subcommands
        .iter()
        .filter(|subcommand| !subcommand.hidden)
        .collect::<Vec<_>>();
    let commands = visible_subcommands
        .into_iter()
        .map(|subcommand| GuidePayloadEntry {
            name: subcommand.name.clone(),
            short_help: subcommand.about.clone().unwrap_or_default(),
        })
        .collect();

    let visible_args = command
        .args
        .iter()
        .filter(|arg| !arg.id.is_empty())
        .collect::<Vec<_>>();
    let arguments = visible_args
        .into_iter()
        .map(|arg| GuidePayloadEntry {
            name: arg_label(arg),
            short_help: arg.help.clone().unwrap_or_default(),
        })
        .collect();

    let visible_flags = command
        .flags
        .iter()
        .filter(|flag| !flag.hidden)
        .collect::<Vec<_>>();
    let options = visible_flags
        .into_iter()
        .map(|flag| GuidePayloadEntry {
            name: flag_label(flag),
            short_help: flag.help.clone().unwrap_or_default(),
        })
        .collect();

    let preamble = command
        .before_help
        .iter()
        .flat_map(|text| text.lines().map(ToString::to_string))
        .collect();
    let epilogue = command
        .after_help
        .iter()
        .flat_map(|text| text.lines().map(ToString::to_string))
        .collect();

    GuidePayload {
        preamble,
        usage,
        commands,
        arguments,
        options,
        common_invocation_options: Vec::new(),
        notes: Vec::new(),
        sections: Vec::new(),
        epilogue,
    }
}

fn default_usage(command: &CommandDef) -> Option<String> {
    if command.name.trim().is_empty() {
        return None;
    }

    let mut parts = vec![command.name.clone()];
    if !command
        .flags
        .iter()
        .filter(|flag| !flag.hidden)
        .collect::<Vec<_>>()
        .is_empty()
    {
        parts.push("[OPTIONS]".to_string());
    }
    for arg in command.args.iter().filter(|arg| !arg.id.is_empty()) {
        let label = arg_label(arg);
        if arg.required {
            parts.push(label);
        } else {
            parts.push(format!("[{label}]"));
        }
    }
    if !command
        .subcommands
        .iter()
        .filter(|subcommand| !subcommand.hidden)
        .collect::<Vec<_>>()
        .is_empty()
    {
        parts.push("<COMMAND>".to_string());
    }
    Some(parts.join(" "))
}

fn arg_label(arg: &ArgDef) -> String {
    arg.value_name.clone().unwrap_or_else(|| arg.id.clone())
}

fn flag_label(flag: &FlagDef) -> String {
    let mut labels = Vec::new();
    if let Some(short) = flag.short {
        labels.push(format!("-{short}"));
    }
    if let Some(long) = flag.long.as_deref() {
        labels.push(format!("--{long}"));
    }
    if flag.takes_value
        && let Some(value_name) = flag.value_name.as_deref()
    {
        labels.push(format!("<{value_name}>"));
    }
    labels.join(", ")
}

fn parse_help_doc(help_text: &str) -> GuideDoc {
    let mut doc = GuideDoc::default();
    let mut current: Option<GuideSection> = None;

    for raw_line in help_text.lines() {
        let line = raw_line.trim_end();
        if let Some((title, kind, body)) = parse_section_header(line) {
            if let Some(section) = current.take() {
                doc.sections.push(section);
            }
            let mut section = GuideSection::new(title, kind);
            if let Some(body) = body {
                section.blocks.push(GuideBlock::Paragraph { text: body });
            }
            current = Some(section);
            continue;
        }

        if let Some(section) = current.as_mut() {
            section.blocks.push(parse_section_line(section.kind, line));
        } else if !line.is_empty() {
            doc.preamble.push(line.to_string());
        }
    }

    if let Some(section) = current {
        doc.sections.push(section);
    }

    doc
}

fn parse_section_header(line: &str) -> Option<(String, GuideSectionKind, Option<String>)> {
    if let Some(usage) = line.strip_prefix("Usage:") {
        return Some((
            "Usage".to_string(),
            GuideSectionKind::Usage,
            Some(format!("  {}", usage.trim())),
        ));
    }

    let (title, kind) = match line {
        "Commands:" => ("Commands".to_string(), GuideSectionKind::Commands),
        "Options:" => ("Options".to_string(), GuideSectionKind::Options),
        "Arguments:" => ("Arguments".to_string(), GuideSectionKind::Arguments),
        "Common Invocation Options:" => (
            "Common Invocation Options".to_string(),
            GuideSectionKind::CommonInvocationOptions,
        ),
        "Notes:" => ("Notes".to_string(), GuideSectionKind::Notes),
        _ if !line.starts_with(' ') && line.ends_with(':') => (
            line.trim_end_matches(':').trim().to_string(),
            GuideSectionKind::Custom,
        ),
        _ => return None,
    };

    Some((title, kind, None))
}

fn parse_section_line(kind: GuideSectionKind, line: &str) -> GuideBlock {
    if line.trim().is_empty() {
        return GuideBlock::Blank;
    }

    if matches!(
        kind,
        GuideSectionKind::Commands
            | GuideSectionKind::Options
            | GuideSectionKind::Arguments
            | GuideSectionKind::CommonInvocationOptions
    ) {
        let indent_len = line.len().saturating_sub(line.trim_start().len());
        let (indent, rest) = line.split_at(indent_len);
        let split = help_description_split(kind, rest).unwrap_or(rest.len());
        let (head, tail) = rest.split_at(split);
        return GuideBlock::Entry(GuideEntry {
            indent: indent.to_string(),
            head: head.to_string(),
            tail: tail.to_string(),
        });
    }

    GuideBlock::Paragraph {
        text: line.to_string(),
    }
}

fn help_description_split(kind: GuideSectionKind, line: &str) -> Option<usize> {
    let mut saw_non_whitespace = false;
    let mut run_start = None;
    let mut run_len = 0usize;

    for (idx, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if saw_non_whitespace {
                run_start.get_or_insert(idx);
                run_len += 1;
            }
            continue;
        }

        if saw_non_whitespace && run_len >= 2 {
            return run_start;
        }

        saw_non_whitespace = true;
        run_start = None;
        run_len = 0;
    }

    if matches!(
        kind,
        GuideSectionKind::Commands | GuideSectionKind::Arguments
    ) {
        return line.find(char::is_whitespace);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        GuideDoc, GuidePayload, GuidePayloadEntry, GuideSection, GuideSectionKind, HelpBlock,
    };
    use crate::core::command_def::{ArgDef, CommandDef, FlagDef};
    use crate::core::output_model::OutputResult;
    use serde_json::json;

    #[test]
    fn help_doc_from_text_preserves_usage_and_command_entries_unit() {
        let doc = GuideDoc::from_text("Usage: osp theme <COMMAND>\n\nCommands:\n  list  Show\n");

        assert_eq!(doc.sections.len(), 2);
        assert_eq!(doc.sections[0].title, "Usage");
        match &doc.sections[1].blocks[0] {
            HelpBlock::Entry(entry) => {
                assert_eq!(entry.head.trim(), "list");
                assert_eq!(entry.tail.trim(), "Show");
            }
            other => panic!("expected entry, got {other:?}"),
        }
    }

    #[test]
    fn help_doc_from_command_def_builds_usage_commands_and_options_unit() {
        let doc = GuideDoc::from_command_def(
            &CommandDef::new("theme")
                .about("Inspect and apply themes")
                .flag(FlagDef::new("raw").long("raw").help("Show raw values"))
                .arg(ArgDef::new("name").value_name("name"))
                .subcommand(CommandDef::new("list").about("List themes")),
        );

        assert_eq!(doc.sections[0].title, "Usage");
        assert_eq!(doc.sections[1].title, "Commands");
        assert_eq!(doc.sections[2].title, "Arguments");
        assert_eq!(doc.sections[3].title, "Options");
    }

    #[test]
    fn help_section_builder_collects_blocks_unit() {
        let section = GuideSection::new("Notes", GuideSectionKind::Notes)
            .paragraph("first")
            .blank()
            .entry("show", "Display");

        assert_eq!(section.blocks.len(), 3);
    }

    #[test]
    fn guide_doc_projects_to_single_semantic_row_unit() {
        let doc = GuideDoc::from_text("Commands:\n  list  Show\n");
        let rows = doc.to_output_result().into_rows().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["commands"][0]["name"], "list");
        assert_eq!(rows[0]["commands"][0]["short_help"], "Show");
    }

    #[test]
    fn guide_doc_json_value_is_semantic_not_internal_shape_unit() {
        let doc = GuideDoc::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");
        let value = doc.to_json_value();

        assert_eq!(value["usage"][0], "osp history <COMMAND>");
        assert_eq!(value["commands"][0]["name"], "list");
        assert!(value.get("sections").is_none());
    }

    #[test]
    fn guide_doc_round_trips_through_output_result_unit() {
        let doc =
            GuideDoc::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  help  Print help\n");
        let output = doc.to_output_result();
        let rebuilt = GuidePayload::try_from_output_result(&output)
            .expect("guide output")
            .to_doc();

        assert_eq!(rebuilt.sections[0].title, "Usage");
        assert_eq!(rebuilt.sections[1].title, "Commands");
    }

    #[test]
    fn guide_payload_accepts_legacy_summary_field_when_rehydrating_unit() {
        let output = OutputResult::from_rows(vec![
            json!({
                "commands": [
                    {
                        "name": "list",
                        "summary": "Show"
                    }
                ]
            })
            .as_object()
            .cloned()
            .expect("object"),
        ]);

        let rebuilt = GuidePayload::try_from_output_result(&output)
            .expect("guide output")
            .to_doc();

        match &rebuilt.sections[0].blocks[0] {
            HelpBlock::Entry(entry) => {
                assert_eq!(entry.head.trim(), "list");
                assert_eq!(entry.tail.trim(), "Show");
            }
            other => panic!("expected entry, got {other:?}"),
        }
    }

    #[test]
    fn guide_payload_markdown_uses_headings_and_entry_tables_unit() {
        let payload = GuidePayload {
            usage: vec!["history <COMMAND>".to_string()],
            commands: vec![GuidePayloadEntry {
                name: "list".to_string(),
                short_help: "List history entries".to_string(),
            }],
            options: vec![GuidePayloadEntry {
                name: "-h, --help".to_string(),
                short_help: "Print help".to_string(),
            }],
            ..GuidePayload::default()
        };

        let rendered = payload.to_markdown();
        assert!(rendered.contains("## Usage"));
        assert!(rendered.contains("history <COMMAND>"));
        assert!(rendered.contains("## Commands"));
        assert!(rendered.contains("| name"));
        assert!(rendered.contains("short_help |"));
        assert!(rendered.contains("| list"));
        assert!(rendered.contains("List history entries |"));
        assert!(rendered.contains("## Options"));
    }

    #[test]
    fn guide_payload_markdown_bounds_padding_to_fit_width_unit() {
        let payload = GuidePayload {
            commands: vec![
                GuidePayloadEntry {
                    name: "plugins".to_string(),
                    short_help: "subcommands: list, commands, enable, disable, doctor".to_string(),
                },
                GuidePayloadEntry {
                    name: "options".to_string(),
                    short_help: "per invocation: --format/--json/--table/--value/--md, --mode, --color, --unicode/--ascii, -v/-q/-d, --cache, --plugin-provider".to_string(),
                },
            ],
            ..GuidePayload::default()
        };

        let rendered = payload.to_markdown_with_width(Some(90));
        assert!(
            rendered.contains("| name    | short_help                                           |")
        );
        assert!(
            rendered.contains("| plugins | subcommands: list, commands, enable, disable, doctor |")
        );
        assert!(rendered.contains(
            "| options | per invocation: --format/--json/--table/--value/--md, --mode, --color"
        ));
    }
}
