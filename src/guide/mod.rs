pub(crate) mod template;

use crate::core::command_def::{ArgDef, CommandDef, FlagDef};
use crate::core::output_model::{OutputDocument, OutputItems, OutputResult, RenderRecommendation};
use crate::ui::document_model::DocumentModel;
use crate::ui::presentation::HelpLevel;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Structured help/guide representation used by the CLI, REPL, and docs views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuideView {
    /// Introductory paragraphs shown before structured sections.
    pub preamble: Vec<String>,
    /// Additional named sections that do not fit the standard buckets.
    pub sections: Vec<GuideSection>,
    /// Closing paragraphs shown after structured sections.
    pub epilogue: Vec<String>,
    /// Usage synopsis lines.
    pub usage: Vec<String>,
    /// Command entries available from this guide.
    pub commands: Vec<GuideEntry>,
    /// Positional argument entries.
    pub arguments: Vec<GuideEntry>,
    /// Option and flag entries.
    pub options: Vec<GuideEntry>,
    /// Global invocation options shared across commands.
    pub common_invocation_options: Vec<GuideEntry>,
    /// Free-form notes associated with the guide.
    pub notes: Vec<String>,
}

/// One named help entry such as a command, argument, or option row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GuideEntry {
    /// Display name for the entry.
    pub name: String,
    /// Short description shown alongside the name.
    pub short_help: String,
    /// Optional indentation override used during presentation.
    #[serde(skip)]
    pub display_indent: Option<String>,
    /// Optional spacing override between the name and description.
    #[serde(skip)]
    pub display_gap: Option<String>,
}

/// One logical section within a [`GuideView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuideSection {
    /// Section heading.
    pub title: String,
    /// Semantic kind used for filtering and rendering.
    pub kind: GuideSectionKind,
    /// Paragraph content rendered before any entries.
    pub paragraphs: Vec<String>,
    /// Structured entries rendered within the section.
    pub entries: Vec<GuideEntry>,
}

/// Canonical section kinds used by structured help output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideSectionKind {
    /// Usage synopsis content.
    Usage,
    /// Command listing content.
    Commands,
    /// Option and flag content.
    Options,
    /// Positional argument content.
    Arguments,
    /// Shared invocation option content.
    CommonInvocationOptions,
    /// Free-form note content.
    Notes,
    /// Any section outside the built-in guide categories.
    Custom,
}

impl Default for GuideSectionKind {
    fn default() -> Self {
        Self::Custom
    }
}

/// Backward-compatible alias for [`GuideView`].
pub type HelpView = GuideView;
/// Backward-compatible alias for [`GuideSection`].
pub type HelpSection = GuideSection;
/// Backward-compatible alias for [`GuideSectionKind`].
pub type HelpSectionKind = GuideSectionKind;
/// Backward-compatible alias for [`GuideEntry`].
pub type HelpEntry = GuideEntry;

impl GuideView {
    /// Parses plain help text into a structured guide view.
    pub fn from_text(help_text: &str) -> Self {
        parse_help_view(help_text)
    }

    /// Builds a guide view from a command definition.
    pub fn from_command_def(command: &CommandDef) -> Self {
        guide_view_from_command_def(command)
    }

    /// Converts the guide into row output with a guide sidecar document.
    pub fn to_output_result(&self) -> OutputResult {
        // Keep the semantic row form for DSL/history/cache, but attach the
        // first-class guide payload so renderers do not have to reconstruct it
        // from rows when no structural stages have destroyed that intent.
        let mut output = OutputResult::from_rows(vec![self.to_row()])
            .with_document(OutputDocument::Guide(self.to_json_value()));
        output.meta.render_recommendation = Some(RenderRecommendation::Guide);
        output
    }

    /// Serializes the guide to its JSON object form.
    pub fn to_json_value(&self) -> Value {
        Value::Object(self.to_row())
    }

    /// Attempts to recover a guide view from structured output.
    pub fn try_from_output_result(output: &OutputResult) -> Option<Self> {
        // Prefer the sidecar document when present. The row-based fallback is
        // still needed after structural DSL stages that clear document intent.
        if let Some(document) = output.document.as_ref()
            && let Some(view) = Self::try_from_output_document(document)
        {
            return Some(view);
        }

        let rows = match &output.items {
            OutputItems::Rows(rows) if rows.len() == 1 => rows,
            _ => return None,
        };
        Self::try_from_row(&rows[0])
    }

    /// Renders the guide as Markdown using the default width policy.
    pub fn to_markdown(&self) -> String {
        self.to_markdown_with_width(None)
    }

    /// Renders the guide as Markdown using an optional target width.
    pub fn to_markdown_with_width(&self, width: Option<usize>) -> String {
        DocumentModel::from_guide_view(self).to_markdown_with_width(width)
    }

    /// Flattens the guide into value-oriented text lines.
    pub fn to_value_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();

        lines.extend(self.preamble.iter().cloned());
        append_value_paragraph_section(&mut lines, "Usage", &self.usage);
        append_value_entry_section(&mut lines, "Commands", &self.commands);
        append_value_entry_section(&mut lines, "Arguments", &self.arguments);
        append_value_entry_section(&mut lines, "Options", &self.options);
        append_value_entry_section(
            &mut lines,
            "Common Invocation Options",
            &self.common_invocation_options,
        );
        append_value_paragraph_section(&mut lines, "Notes", &self.notes);

        for section in &self.sections {
            if !section.paragraphs.is_empty() || !section.entries.is_empty() {
                if !lines.is_empty() {
                    lines.push(String::new());
                }
                lines.push(section.title.clone());
                lines.extend(section.paragraphs.iter().cloned());
                lines.extend(section.entries.iter().map(value_line_for_entry));
            }
        }

        if !self.epilogue.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(self.epilogue.iter().cloned());
        }

        lines
    }

    /// Appends another guide view into this one, preserving section order.
    pub fn merge(&mut self, mut other: GuideView) {
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

    pub(crate) fn filtered_for_help_level(&self, level: HelpLevel) -> Self {
        let mut filtered = self.clone();
        filtered.usage = if level >= HelpLevel::Tiny {
            self.usage.clone()
        } else {
            Vec::new()
        };
        filtered.commands = if level >= HelpLevel::Normal {
            self.commands.clone()
        } else {
            Vec::new()
        };
        filtered.arguments = if level >= HelpLevel::Normal {
            self.arguments.clone()
        } else {
            Vec::new()
        };
        filtered.options = if level >= HelpLevel::Normal {
            self.options.clone()
        } else {
            Vec::new()
        };
        filtered.common_invocation_options = if level >= HelpLevel::Verbose {
            self.common_invocation_options.clone()
        } else {
            Vec::new()
        };
        filtered.notes = if level >= HelpLevel::Normal {
            self.notes.clone()
        } else {
            Vec::new()
        };
        filtered.sections = self
            .sections
            .iter()
            .filter(|section| level >= section.kind.min_help_level())
            .cloned()
            .collect();
        filtered
    }
}

impl GuideView {
    fn try_from_output_document(document: &OutputDocument) -> Option<Self> {
        match document {
            OutputDocument::Guide(value) => serde_json::from_value(value.clone()).ok(),
        }
    }

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
                Value::Array(self.sections.iter().map(GuideSection::to_value).collect()),
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

fn append_value_paragraph_section(lines: &mut Vec<String>, title: &str, paragraphs: &[String]) {
    if paragraphs.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(title.to_string());
    lines.extend(paragraphs.iter().cloned());
}

fn append_value_entry_section(lines: &mut Vec<String>, title: &str, entries: &[GuideEntry]) {
    if entries.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(title.to_string());
    lines.extend(entries.iter().map(value_line_for_entry));
}

fn value_line_for_entry(entry: &GuideEntry) -> String {
    if entry.short_help.trim().is_empty() {
        entry.name.clone()
    } else {
        format!("{}  {}", entry.name, entry.short_help)
    }
}

impl GuideSection {
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
    /// Creates a new guide section with a title and canonical kind.
    pub fn new(title: impl Into<String>, kind: GuideSectionKind) -> Self {
        Self {
            title: title.into(),
            kind,
            paragraphs: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Appends a paragraph to the section.
    pub fn paragraph(mut self, text: impl Into<String>) -> Self {
        self.paragraphs.push(text.into());
        self
    }

    /// Appends a named entry row to the section.
    pub fn entry(mut self, name: impl Into<String>, short_help: impl Into<String>) -> Self {
        self.entries.push(GuideEntry {
            name: name.into(),
            short_help: short_help.into(),
            display_indent: None,
            display_gap: None,
        });
        self
    }
}

impl GuideSectionKind {
    /// Returns the stable string form used in serialized guide payloads.
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

    pub(crate) fn min_help_level(self) -> HelpLevel {
        match self {
            GuideSectionKind::Usage => HelpLevel::Tiny,
            GuideSectionKind::CommonInvocationOptions => HelpLevel::Verbose,
            GuideSectionKind::Commands
            | GuideSectionKind::Options
            | GuideSectionKind::Arguments
            | GuideSectionKind::Notes
            | GuideSectionKind::Custom => HelpLevel::Normal,
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

fn payload_entry_value(entry: &GuideEntry) -> Value {
    json!({
        "name": entry.name,
        "short_help": entry.short_help,
    })
}

fn payload_entry_array(entries: &[GuideEntry]) -> Value {
    Value::Array(entries.iter().map(payload_entry_value).collect())
}

fn payload_entries(value: Option<&Value>) -> Option<Vec<GuideEntry>> {
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
        out.push(GuideEntry {
            name,
            short_help,
            display_indent: None,
            display_gap: None,
        });
    }
    Some(out)
}

fn payload_sections(value: Option<&Value>) -> Option<Vec<GuideSection>> {
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
        out.push(GuideSection {
            title,
            kind,
            paragraphs: row_string_array(section.get("paragraphs"))?,
            entries: payload_entries(section.get("entries"))?,
        });
    }
    Some(out)
}

fn guide_view_from_command_def(command: &CommandDef) -> GuideView {
    let usage = command
        .usage
        .clone()
        .or_else(|| default_usage(command))
        .map(|usage| vec![usage])
        .unwrap_or_default();

    let visible_subcommands = command
        .subcommands
        .iter()
        .filter(|subcommand| !subcommand.hidden)
        .collect::<Vec<_>>();
    let commands = visible_subcommands
        .into_iter()
        .map(|subcommand| GuideEntry {
            name: subcommand.name.clone(),
            short_help: subcommand.about.clone().unwrap_or_default(),
            display_indent: None,
            display_gap: None,
        })
        .collect();

    let visible_args = command
        .args
        .iter()
        .filter(|arg| !arg.id.is_empty())
        .collect::<Vec<_>>();
    let arguments = visible_args
        .into_iter()
        .map(|arg| GuideEntry {
            name: arg_label(arg),
            short_help: arg.help.clone().unwrap_or_default(),
            display_indent: None,
            display_gap: None,
        })
        .collect();

    let visible_flags = command
        .flags
        .iter()
        .filter(|flag| !flag.hidden)
        .collect::<Vec<_>>();
    let options = visible_flags
        .into_iter()
        .map(|flag| GuideEntry {
            name: flag_label(flag),
            short_help: flag.help.clone().unwrap_or_default(),
            display_indent: Some(if flag.short.is_some() {
                "  ".to_string()
            } else {
                "      ".to_string()
            }),
            display_gap: None,
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

    GuideView {
        preamble,
        sections: Vec::new(),
        epilogue,
        usage,
        commands,
        arguments,
        options,
        common_invocation_options: Vec::new(),
        notes: Vec::new(),
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

fn parse_help_view(help_text: &str) -> GuideView {
    let mut view = GuideView::default();
    let mut current: Option<GuideSection> = None;
    let mut saw_section = false;

    for raw_line in help_text.lines() {
        let line = raw_line.trim_end();
        if let Some((title, kind, body)) = parse_section_header(line) {
            if let Some(section) = current.take() {
                view.sections.push(section);
            }
            saw_section = true;
            let mut section = GuideSection::new(title, kind);
            if let Some(body) = body {
                section.paragraphs.push(body);
            }
            current = Some(section);
            continue;
        }

        if current
            .as_ref()
            .is_some_and(|section| line_belongs_to_epilogue(section.kind, line))
        {
            if let Some(section) = current.take() {
                view.sections.push(section);
            }
            view.epilogue.push(line.to_string());
            continue;
        }

        if let Some(section) = current.as_mut() {
            parse_section_line(section, line);
        } else if !line.is_empty() {
            if saw_section {
                view.epilogue.push(line.to_string());
            } else {
                view.preamble.push(line.to_string());
            }
        }
    }

    if let Some(section) = current {
        view.sections.push(section);
    }

    repartition_builtin_sections(view)
}

fn line_belongs_to_epilogue(kind: GuideSectionKind, line: &str) -> bool {
    if line.trim().is_empty() {
        return false;
    }

    matches!(
        kind,
        GuideSectionKind::Commands | GuideSectionKind::Options | GuideSectionKind::Arguments
    ) && !line.starts_with(' ')
}

fn parse_section_header(line: &str) -> Option<(String, GuideSectionKind, Option<String>)> {
    if let Some(usage) = line.strip_prefix("Usage:") {
        return Some((
            "Usage".to_string(),
            GuideSectionKind::Usage,
            Some(usage.trim().to_string()),
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

fn parse_section_line(section: &mut GuideSection, line: &str) {
    if line.trim().is_empty() {
        return;
    }

    if matches!(
        section.kind,
        GuideSectionKind::Commands
            | GuideSectionKind::Options
            | GuideSectionKind::Arguments
            | GuideSectionKind::CommonInvocationOptions
    ) {
        let indent_len = line.len().saturating_sub(line.trim_start().len());
        let (_, rest) = line.split_at(indent_len);
        let split = help_description_split(section.kind, rest).unwrap_or(rest.len());
        let (head, tail) = rest.split_at(split);
        let display_indent = Some(" ".repeat(indent_len));
        let display_gap = (!tail.is_empty()).then(|| {
            tail.chars()
                .take_while(|ch| ch.is_whitespace())
                .collect::<String>()
        });
        section.entries.push(GuideEntry {
            name: head.trim().to_string(),
            short_help: tail.trim().to_string(),
            display_indent,
            display_gap,
        });
        return;
    }

    section.paragraphs.push(line.to_string());
}

fn repartition_builtin_sections(mut view: GuideView) -> GuideView {
    let sections = std::mem::take(&mut view.sections);
    for section in sections {
        match section.kind {
            GuideSectionKind::Usage => view.usage.extend(section.paragraphs),
            GuideSectionKind::Commands => view.commands.extend(section.entries),
            GuideSectionKind::Arguments => view.arguments.extend(section.entries),
            GuideSectionKind::Options => view.options.extend(section.entries),
            GuideSectionKind::CommonInvocationOptions => {
                view.common_invocation_options.extend(section.entries);
            }
            GuideSectionKind::Notes => view.notes.extend(section.paragraphs),
            GuideSectionKind::Custom => view.sections.push(section),
        }
    }
    view
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
    use super::{GuideEntry, GuideSection, GuideSectionKind, GuideView};
    use crate::core::command_def::{ArgDef, CommandDef, FlagDef};
    use serde_json::json;
    use serde_json::Value;

    use crate::core::output_model::{OutputDocument, OutputResult};
    use crate::ui::presentation::HelpLevel;

    #[test]
    fn guide_view_from_text_preserves_usage_and_command_entries_unit() {
        let view = GuideView::from_text("Usage: osp theme <COMMAND>\n\nCommands:\n  list  Show\n");

        assert_eq!(view.usage, vec!["osp theme <COMMAND>".to_string()]);
        assert_eq!(view.commands[0].name, "list");
        assert_eq!(view.commands[0].short_help, "Show");
    }

    #[test]
    fn filtered_for_help_level_hides_verbose_sections_until_requested_unit() {
        let mut view = GuideView::from_text(
            "Usage: osp [COMMAND]\n\nCommands:\n  help  Show help\n\nCommon Invocation Options:\n  --json  Render as JSON\n",
        );
        view.sections
            .push(GuideSection::new("Notes", GuideSectionKind::Notes).paragraph("extra note"));

        let tiny = view.filtered_for_help_level(HelpLevel::Tiny);
        let normal = view.filtered_for_help_level(HelpLevel::Normal);
        let verbose = view.filtered_for_help_level(HelpLevel::Verbose);

        assert!(!tiny.usage.is_empty());
        assert!(tiny.commands.is_empty());
        assert!(normal.common_invocation_options.is_empty());
        assert!(!normal.commands.is_empty());
        assert!(!normal.sections.is_empty());
        assert!(!verbose.common_invocation_options.is_empty());
    }

    #[test]
    fn guide_view_from_command_def_builds_usage_commands_and_options_unit() {
        let view = GuideView::from_command_def(
            &CommandDef::new("theme")
                .about("Inspect and apply themes")
                .flag(FlagDef::new("raw").long("raw").help("Show raw values"))
                .arg(ArgDef::new("name").value_name("name"))
                .subcommand(CommandDef::new("list").about("List themes")),
        );

        assert_eq!(view.usage.len(), 1);
        assert_eq!(view.commands.len(), 1);
        assert_eq!(view.arguments.len(), 1);
        assert_eq!(view.options.len(), 1);
    }

    #[test]
    fn help_section_builder_collects_blocks_unit() {
        let section = GuideSection::new("Notes", GuideSectionKind::Notes)
            .paragraph("first")
            .entry("show", "Display");

        assert_eq!(section.paragraphs, vec!["first".to_string()]);
        assert_eq!(section.entries.len(), 1);
    }

    #[test]
    fn guide_view_projects_to_single_semantic_row_unit() {
        let view = GuideView::from_text("Commands:\n  list  Show\n");
        let rows = view.to_output_result().into_rows().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["commands"][0]["name"], "list");
        assert_eq!(rows[0]["commands"][0]["short_help"], "Show");
    }

    #[test]
    fn guide_view_json_value_is_semantic_not_internal_shape_unit() {
        let view = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");
        let value = view.to_json_value();

        assert_eq!(value["usage"][0], "osp history <COMMAND>");
        assert_eq!(value["commands"][0]["name"], "list");
        assert!(value.get("sections").is_none());
    }

    #[test]
    fn guide_view_round_trips_through_output_result_unit() {
        let view =
            GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  help  Print help\n");
        let output = view.to_output_result();
        let rebuilt = GuideView::try_from_output_result(&output).expect("guide output");

        assert_eq!(rebuilt.usage[0], "osp history <COMMAND>");
        assert_eq!(rebuilt.commands[0].name, "help");
    }

    #[test]
    fn guide_view_output_result_carries_document_sidecar_unit() {
        let view =
            GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  help  Print help\n");
        let output = view.to_output_result();

        assert!(matches!(
            output.document,
            Some(OutputDocument::Guide(Value::Object(_)))
        ));
    }

    #[test]
    fn guide_view_accepts_legacy_summary_field_when_rehydrating_unit() {
        let output = OutputResult::from_rows(vec![json!({
            "commands": [
                {
                    "name": "list",
                    "summary": "Show"
                }
            ]
        })
        .as_object()
        .cloned()
        .expect("object")]);

        let rebuilt = GuideView::try_from_output_result(&output).expect("guide output");
        assert_eq!(rebuilt.commands[0].name, "list");
        assert_eq!(rebuilt.commands[0].short_help, "Show");
    }

    #[test]
    fn guide_view_markdown_uses_headings_and_entry_tables_unit() {
        let view = GuideView {
            usage: vec!["history <COMMAND>".to_string()],
            commands: vec![GuideEntry {
                name: "list".to_string(),
                short_help: "List history entries".to_string(),
                display_indent: None,
                display_gap: None,
            }],
            options: vec![GuideEntry {
                name: "-h, --help".to_string(),
                short_help: "Print help".to_string(),
                display_indent: None,
                display_gap: None,
            }],
            ..GuideView::default()
        };

        let rendered = view.to_markdown();
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
    fn guide_view_markdown_bounds_padding_to_fit_width_unit() {
        let view = GuideView {
            commands: vec![
                GuideEntry {
                    name: "plugins".to_string(),
                    short_help: "subcommands: list, commands, enable, disable, doctor".to_string(),
                    display_indent: None,
                    display_gap: None,
                },
                GuideEntry {
                    name: "options".to_string(),
                    short_help: "per invocation: --format/--json/--table/--value/--md, --mode, --color, --unicode/--ascii, -v/-q/-d, --cache, --plugin-provider".to_string(),
                    display_indent: None,
                    display_gap: None,
                },
            ],
            ..GuideView::default()
        };

        let rendered = view.to_markdown_with_width(Some(90));
        let lines = rendered.lines().collect::<Vec<_>>();
        assert!(
            lines
                .iter()
                .any(|line| line.contains("| name") && line.contains("short_help")),
            "expected markdown header row in:\n{rendered}"
        );
        assert!(
            lines.iter().any(|line| line.contains("| plugins |")),
            "expected plugins row in:\n{rendered}"
        );
        assert!(
            lines.iter().any(|line| line.contains("| options |")),
            "expected options row in:\n{rendered}"
        );
    }
}
