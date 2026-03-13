//! Structured help and guide payload model.
//!
//! This module exists so help, intro, and command-reference content can travel
//! through the app as semantic data instead of ad hoc rendered strings.
//!
//! High level flow:
//!
//! - collect guide content from command definitions or parsed help text
//! - keep it in [`crate::guide::GuideView`] form while other systems inspect,
//!   filter, or render it
//! - lower it later into rows, documents, or markdown as needed
//!
//! Contract:
//!
//! - guide data should stay semantic here
//! - presentation-specific layout belongs in the UI layer
//!
//! Public API shape:
//!
//! - [`crate::guide::GuideView`] and related section/entry types stay
//!   intentionally direct to compose because they are semantic payloads
//! - common generation paths use factories like
//!   [`crate::guide::GuideView::from_text`] and
//!   [`crate::guide::GuideView::from_command_def`]
//! - rendering/layout policy stays outside this module so the guide model
//!   remains reusable

pub(crate) mod template;

use crate::core::command_def::{ArgDef, CommandDef, FlagDef};
use crate::core::output_model::{
    OutputDocument, OutputDocumentKind, OutputItems, OutputResult, RenderRecommendation,
};
use crate::ui::document_model::DocumentModel;
use crate::ui::presentation::HelpLevel;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Structured help/guide payload shared by the CLI, REPL, renderers, and
/// semantic output pipeline.
///
/// Canonical help sections such as usage, commands, and options are exposed as
/// dedicated buckets for ergonomic access. The generic [`GuideView::sections`]
/// list exists to preserve authored section order during serialization and
/// transforms, including canonical sections that were authored inline with
/// custom content. Restore logic may backfill the dedicated buckets from those
/// canonical sections, but renderers and serializers treat the ordered section
/// list as authoritative whenever it already carries that content.
///
/// Public API note: this is intentionally an open semantic DTO. Callers may
/// compose it directly for bespoke help payloads, while common generation paths
/// are exposed as factory methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuideView {
    /// Introductory paragraphs shown before structured sections.
    pub preamble: Vec<String>,
    /// Extra sections preserved outside the canonical buckets.
    pub sections: Vec<GuideSection>,
    /// Closing paragraphs shown after structured sections.
    pub epilogue: Vec<String>,
    /// Canonical usage synopsis lines.
    pub usage: Vec<String>,
    /// Canonical command-entry bucket.
    pub commands: Vec<GuideEntry>,
    /// Canonical positional-argument bucket.
    pub arguments: Vec<GuideEntry>,
    /// Canonical option/flag bucket.
    pub options: Vec<GuideEntry>,
    /// Canonical shared invocation-option bucket.
    pub common_invocation_options: Vec<GuideEntry>,
    /// Canonical note paragraphs.
    pub notes: Vec<String>,
}

/// One named row within a guide section or canonical bucket.
///
/// The serialized form intentionally keeps only semantic content. Display-only
/// spacing overrides are carried separately so renderers can adjust layout
/// without affecting the semantic payload used by DSL, cache, or export flows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuideEntry {
    /// Stable label for the entry.
    pub name: String,
    /// Short explanatory text paired with the label.
    pub short_help: String,
    /// Presentation-only indentation override.
    #[serde(skip)]
    pub display_indent: Option<String>,
    /// Presentation-only spacing override between label and description.
    #[serde(skip)]
    pub display_gap: Option<String>,
}

/// One logical section within a [`GuideView`].
///
/// Custom sections live here directly. Canonical sections may also be
/// represented here when authored inline with custom content. Restore logic may
/// mirror canonical sections into the dedicated [`GuideView`] buckets for
/// ergonomic access, but it should not reorder or delete the authored section
/// list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GuideSection {
    /// User-facing section heading.
    pub title: String,
    /// Semantic kind used for normalization and rendering policy.
    pub kind: GuideSectionKind,
    /// Paragraph content rendered before any entries.
    pub paragraphs: Vec<String>,
    /// Structured rows rendered within the section.
    pub entries: Vec<GuideEntry>,
    /// Arbitrary semantic data rendered through the normal value/document path.
    ///
    /// Markdown template imports use this for fenced `osp` blocks so authors
    /// can embed structured data without forcing the final output to be literal
    /// source JSON.
    pub data: Option<Value>,
}

/// Canonical section kinds used by structured help output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    #[default]
    Custom,
}

impl GuideView {
    /// Parses plain help text into a structured guide view.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::guide::GuideView;
    ///
    /// let guide = GuideView::from_text("Usage: osp theme <COMMAND>\n\nCommands:\n  list  Show\n");
    ///
    /// assert_eq!(guide.usage, vec!["osp theme <COMMAND>".to_string()]);
    /// assert_eq!(guide.commands[0].name, "list");
    /// assert_eq!(guide.commands[0].short_help, "Show");
    /// ```
    pub fn from_text(help_text: &str) -> Self {
        parse_help_view(help_text)
    }

    /// Builds a guide view from a command definition.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::command_def::CommandDef;
    /// use osp_cli::guide::GuideView;
    ///
    /// let command = CommandDef::new("theme")
    ///     .about("Inspect themes")
    ///     .subcommand(CommandDef::new("show").about("Show available themes"));
    /// let guide = GuideView::from_command_def(&command);
    ///
    /// assert_eq!(guide.usage, vec!["theme <COMMAND>".to_string()]);
    /// assert_eq!(guide.commands[0].name, "show");
    /// assert!(guide.arguments.is_empty());
    /// assert!(guide.options.is_empty());
    /// ```
    pub fn from_command_def(command: &CommandDef) -> Self {
        guide_view_from_command_def(command)
    }

    /// Converts the guide into row output with a guide sidecar document.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output_model::OutputDocumentKind;
    /// use osp_cli::guide::GuideView;
    ///
    /// let guide = GuideView {
    ///     usage: vec!["theme show".to_string()],
    ///     ..GuideView::default()
    /// };
    /// let output = guide.to_output_result();
    ///
    /// assert_eq!(output.document.as_ref().map(|doc| doc.kind), Some(OutputDocumentKind::Guide));
    /// assert_eq!(output.meta.render_recommendation.is_some(), true);
    /// let rows = output.as_rows().expect("guide output should keep row projection");
    /// assert_eq!(rows.len(), 1);
    /// assert_eq!(rows[0]["usage"][0], "theme show");
    /// ```
    pub fn to_output_result(&self) -> OutputResult {
        // Keep the semantic row form for DSL/history/cache, but attach the
        // first-class guide payload so renderers do not have to reconstruct it
        // from rows when no structural stages have destroyed that intent.
        let mut output = OutputResult::from_rows(vec![self.to_row()]).with_document(
            OutputDocument::new(OutputDocumentKind::Guide, self.to_json_value()),
        );
        output.meta.render_recommendation = Some(RenderRecommendation::Guide);
        output
    }

    /// Serializes the guide to its JSON object form.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::guide::GuideView;
    ///
    /// let guide = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");
    /// let value = guide.to_json_value();
    ///
    /// assert_eq!(value["usage"][0], "osp history <COMMAND>");
    /// assert_eq!(value["commands"][0]["name"], "list");
    /// assert!(value.get("sections").is_none());
    /// ```
    pub fn to_json_value(&self) -> Value {
        Value::Object(self.to_row())
    }

    /// Attempts to recover a guide view from structured output.
    ///
    /// A carried semantic document is authoritative. When `output.document` is
    /// present, this function only attempts to restore from that document and
    /// does not silently fall back to the row projection.
    pub fn try_from_output_result(output: &OutputResult) -> Option<Self> {
        // A carried semantic document is authoritative. If the canonical JSON
        // no longer restores as a guide after DSL, do not silently guess from
        // the row projection and pretend the payload is still semantic guide
        // content.
        if let Some(document) = output.document.as_ref() {
            return Self::try_from_output_document(document);
        }

        let rows = match &output.items {
            OutputItems::Rows(rows) if rows.len() == 1 => rows,
            _ => return None,
        };
        Self::try_from_row(&rows[0])
    }

    /// Renders the guide as Markdown using the default width policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::guide::GuideView;
    ///
    /// let guide = GuideView {
    ///     usage: vec!["theme show".to_string()],
    ///     commands: vec![osp_cli::guide::GuideEntry {
    ///         name: "list".to_string(),
    ///         short_help: "List themes".to_string(),
    ///         display_indent: None,
    ///         display_gap: None,
    ///     }],
    ///     ..GuideView::default()
    /// };
    ///
    /// let markdown = guide.to_markdown();
    ///
    /// assert!(markdown.contains("## Usage"));
    /// assert!(markdown.contains("theme show"));
    /// assert!(markdown.contains("- `list` List themes"));
    /// ```
    pub fn to_markdown(&self) -> String {
        self.to_markdown_with_width(None)
    }

    /// Renders the guide as Markdown using an optional target width.
    pub fn to_markdown_with_width(&self, width: Option<usize>) -> String {
        DocumentModel::from_guide_view(self).to_markdown_with_width(width)
    }

    /// Flattens the guide into value-oriented text lines.
    pub fn to_value_lines(&self) -> Vec<String> {
        let normalized = Self::normalize_restored_sections(self.clone());
        let mut lines = Vec::new();
        let use_ordered_sections = normalized.uses_ordered_section_representation();

        append_value_paragraphs(&mut lines, &normalized.preamble);
        if !(use_ordered_sections
            && normalized.has_canonical_builtin_section_kind(GuideSectionKind::Usage))
        {
            append_value_paragraphs(&mut lines, &normalized.usage);
        }
        if !(use_ordered_sections
            && normalized.has_canonical_builtin_section_kind(GuideSectionKind::Commands))
        {
            append_value_entries(&mut lines, &normalized.commands);
        }
        if !(use_ordered_sections
            && normalized.has_canonical_builtin_section_kind(GuideSectionKind::Arguments))
        {
            append_value_entries(&mut lines, &normalized.arguments);
        }
        if !(use_ordered_sections
            && normalized.has_canonical_builtin_section_kind(GuideSectionKind::Options))
        {
            append_value_entries(&mut lines, &normalized.options);
        }
        if !(use_ordered_sections
            && normalized
                .has_canonical_builtin_section_kind(GuideSectionKind::CommonInvocationOptions))
        {
            append_value_entries(&mut lines, &normalized.common_invocation_options);
        }
        if !(use_ordered_sections
            && normalized.has_canonical_builtin_section_kind(GuideSectionKind::Notes))
        {
            append_value_paragraphs(&mut lines, &normalized.notes);
        }

        for section in &normalized.sections {
            if !use_ordered_sections && section.is_canonical_builtin_section() {
                continue;
            }
            append_value_paragraphs(&mut lines, &section.paragraphs);
            append_value_entries(&mut lines, &section.entries);
            if let Some(data) = section.data.as_ref() {
                append_value_data(&mut lines, data);
            }
        }

        append_value_paragraphs(&mut lines, &normalized.epilogue);

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
        match document.kind {
            OutputDocumentKind::Guide => {
                let view = Self::normalize_restored_sections(
                    serde_json::from_value(document.value.clone()).ok()?,
                );
                view.is_semantically_valid().then_some(view)
            }
        }
    }

    fn is_semantically_valid(&self) -> bool {
        let entries_are_valid =
            |entries: &[GuideEntry]| entries.iter().all(GuideEntry::is_semantically_valid);
        let sections_are_valid = self
            .sections
            .iter()
            .all(GuideSection::is_semantically_valid);
        let has_content = !self.preamble.is_empty()
            || !self.epilogue.is_empty()
            || !self.usage.is_empty()
            || !self.notes.is_empty()
            || !self.commands.is_empty()
            || !self.arguments.is_empty()
            || !self.options.is_empty()
            || !self.common_invocation_options.is_empty()
            || !self.sections.is_empty();

        has_content
            && entries_are_valid(&self.commands)
            && entries_are_valid(&self.arguments)
            && entries_are_valid(&self.options)
            && entries_are_valid(&self.common_invocation_options)
            && sections_are_valid
    }

    fn to_row(&self) -> Map<String, Value> {
        let mut row = Map::new();
        let use_ordered_sections = self.uses_ordered_section_representation();

        if !self.preamble.is_empty() {
            row.insert("preamble".to_string(), string_array(&self.preamble));
        }

        if !(self.usage.is_empty()
            || use_ordered_sections
                && self.has_canonical_builtin_section_kind(GuideSectionKind::Usage))
        {
            row.insert("usage".to_string(), string_array(&self.usage));
        }
        if !(self.commands.is_empty()
            || use_ordered_sections
                && self.has_canonical_builtin_section_kind(GuideSectionKind::Commands))
        {
            row.insert("commands".to_string(), payload_entry_array(&self.commands));
        }
        if !(self.arguments.is_empty()
            || use_ordered_sections
                && self.has_canonical_builtin_section_kind(GuideSectionKind::Arguments))
        {
            row.insert(
                "arguments".to_string(),
                payload_entry_array(&self.arguments),
            );
        }
        if !(self.options.is_empty()
            || use_ordered_sections
                && self.has_canonical_builtin_section_kind(GuideSectionKind::Options))
        {
            row.insert("options".to_string(), payload_entry_array(&self.options));
        }
        if !(self.common_invocation_options.is_empty()
            || use_ordered_sections
                && self
                    .has_canonical_builtin_section_kind(GuideSectionKind::CommonInvocationOptions))
        {
            row.insert(
                "common_invocation_options".to_string(),
                payload_entry_array(&self.common_invocation_options),
            );
        }
        if !(self.notes.is_empty()
            || use_ordered_sections
                && self.has_canonical_builtin_section_kind(GuideSectionKind::Notes))
        {
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
        let view = Self::normalize_restored_sections(Self {
            preamble: row_string_array(row.get("preamble"))?,
            usage: row_string_array(row.get("usage"))?,
            commands: payload_entries(row.get("commands"))?,
            arguments: payload_entries(row.get("arguments"))?,
            options: payload_entries(row.get("options"))?,
            common_invocation_options: payload_entries(row.get("common_invocation_options"))?,
            notes: row_string_array(row.get("notes"))?,
            sections: payload_sections(row.get("sections"))?,
            epilogue: row_string_array(row.get("epilogue"))?,
        });
        view.is_semantically_valid().then_some(view)
    }

    fn normalize_restored_sections(mut view: Self) -> Self {
        // There are two valid guide representations:
        //
        // - ordered sections are authoritative when custom/non-canonical
        //   sections are interleaved with builtin ones (for example intro
        //   payloads)
        // - canonical buckets are authoritative when the payload only carries
        //   builtin guide sections and those sections are merely structural
        //   carriers for DSL addressing
        //
        // Restore must preserve authored mixed/custom section order, but it may
        // collapse canonical-only section lists back into the dedicated buckets
        // so ordinary help payloads keep their stable semantic shape.
        let use_ordered_sections = view.uses_ordered_section_representation();
        let has_custom_sections = view
            .sections
            .iter()
            .any(|section| !section.is_canonical_builtin_section());
        let mut canonical_usage = Vec::new();
        let mut canonical_commands = Vec::new();
        let mut canonical_arguments = Vec::new();
        let mut canonical_options = Vec::new();
        let mut canonical_common_invocation_options = Vec::new();
        let mut canonical_notes = Vec::new();

        for section in &view.sections {
            if !section.is_canonical_builtin_section() {
                continue;
            }

            match section.kind {
                GuideSectionKind::Usage => {
                    canonical_usage.extend(section.paragraphs.iter().cloned())
                }
                GuideSectionKind::Commands => {
                    canonical_commands.extend(section.entries.iter().cloned());
                }
                GuideSectionKind::Arguments => {
                    canonical_arguments.extend(section.entries.iter().cloned());
                }
                GuideSectionKind::Options => {
                    canonical_options.extend(section.entries.iter().cloned())
                }
                GuideSectionKind::CommonInvocationOptions => {
                    canonical_common_invocation_options.extend(section.entries.iter().cloned());
                }
                GuideSectionKind::Notes => {
                    canonical_notes.extend(section.paragraphs.iter().cloned())
                }
                GuideSectionKind::Custom => {}
            }
        }

        if !use_ordered_sections || !has_custom_sections {
            if view.has_canonical_builtin_section_kind(GuideSectionKind::Usage)
                || view.usage.is_empty() && !canonical_usage.is_empty()
            {
                view.usage = canonical_usage;
            }

            if view.has_canonical_builtin_section_kind(GuideSectionKind::Commands)
                || view.commands.is_empty() && !canonical_commands.is_empty()
            {
                view.commands = canonical_commands;
            }

            if view.has_canonical_builtin_section_kind(GuideSectionKind::Arguments)
                || view.arguments.is_empty() && !canonical_arguments.is_empty()
            {
                view.arguments = canonical_arguments;
            }

            if view.has_canonical_builtin_section_kind(GuideSectionKind::Options)
                || view.options.is_empty() && !canonical_options.is_empty()
            {
                view.options = canonical_options;
            }

            if view.has_canonical_builtin_section_kind(GuideSectionKind::CommonInvocationOptions)
                || view.common_invocation_options.is_empty()
                    && !canonical_common_invocation_options.is_empty()
            {
                view.common_invocation_options = canonical_common_invocation_options;
            }

            if view.has_canonical_builtin_section_kind(GuideSectionKind::Notes)
                || view.notes.is_empty() && !canonical_notes.is_empty()
            {
                view.notes = canonical_notes;
            }

            view.sections
                .retain(|section| !section.is_canonical_builtin_section());
        } else {
            if view.usage.is_empty() && !canonical_usage.is_empty() {
                view.usage = canonical_usage;
            }
            if view.commands.is_empty() && !canonical_commands.is_empty() {
                view.commands = canonical_commands;
            }
            if view.arguments.is_empty() && !canonical_arguments.is_empty() {
                view.arguments = canonical_arguments;
            }
            if view.options.is_empty() && !canonical_options.is_empty() {
                view.options = canonical_options;
            }
            if view.common_invocation_options.is_empty()
                && !canonical_common_invocation_options.is_empty()
            {
                view.common_invocation_options = canonical_common_invocation_options;
            }
            if view.notes.is_empty() && !canonical_notes.is_empty() {
                view.notes = canonical_notes;
            }
        }
        view
    }

    pub(crate) fn has_canonical_builtin_section_kind(&self, kind: GuideSectionKind) -> bool {
        self.sections
            .iter()
            .any(|section| section.kind == kind && section.is_canonical_builtin_section())
    }

    pub(crate) fn uses_ordered_section_representation(&self) -> bool {
        self.sections.iter().any(|section| {
            !section.is_canonical_builtin_section()
                || canonical_section_owns_ordered_content(self, section)
        })
    }
}

fn canonical_section_owns_ordered_content(view: &GuideView, section: &GuideSection) -> bool {
    let has_data = !matches!(section.data, None | Some(Value::Null));
    (match section.kind {
        // Canonical sections normally mirror the dedicated top-level buckets.
        // If the bucket is empty but the section carries content, the section
        // list is the authoritative authored shape and later lowering must keep
        // that order instead of discarding the canonical section as a duplicate.
        GuideSectionKind::Usage => !section.paragraphs.is_empty() && view.usage.is_empty(),
        GuideSectionKind::Commands => !section.entries.is_empty() && view.commands.is_empty(),
        GuideSectionKind::Arguments => !section.entries.is_empty() && view.arguments.is_empty(),
        GuideSectionKind::Options => !section.entries.is_empty() && view.options.is_empty(),
        GuideSectionKind::CommonInvocationOptions => {
            !section.entries.is_empty() && view.common_invocation_options.is_empty()
        }
        GuideSectionKind::Notes => !section.paragraphs.is_empty() && view.notes.is_empty(),
        GuideSectionKind::Custom => false,
    }) || has_data
}

impl GuideEntry {
    fn is_semantically_valid(&self) -> bool {
        !self.name.is_empty() || !self.short_help.is_empty()
    }
}

impl GuideSection {
    fn is_semantically_valid(&self) -> bool {
        let has_data = !matches!(self.data, None | Some(Value::Null));
        let has_content =
            !self.title.is_empty() || !self.paragraphs.is_empty() || !self.entries.is_empty();
        (has_content || has_data) && self.entries.iter().all(GuideEntry::is_semantically_valid)
    }

    pub(crate) fn is_canonical_builtin_section(&self) -> bool {
        let expected = match self.kind {
            GuideSectionKind::Usage => "Usage",
            GuideSectionKind::Commands => "Commands",
            GuideSectionKind::Arguments => "Arguments",
            GuideSectionKind::Options => "Options",
            GuideSectionKind::CommonInvocationOptions => "Common Invocation Options",
            GuideSectionKind::Notes => "Notes",
            GuideSectionKind::Custom => return false,
        };

        self.title.trim().eq_ignore_ascii_case(expected)
    }
}

fn append_value_paragraphs(lines: &mut Vec<String>, paragraphs: &[String]) {
    if paragraphs.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.extend(paragraphs.iter().cloned());
}

fn append_value_entries(lines: &mut Vec<String>, entries: &[GuideEntry]) {
    let values = entries
        .iter()
        .filter_map(value_line_for_entry)
        .collect::<Vec<_>>();

    if values.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.extend(values);
}

fn append_value_data(lines: &mut Vec<String>, data: &Value) {
    let values = data_value_lines(data);
    if values.is_empty() {
        return;
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.extend(values);
}

fn data_value_lines(value: &Value) -> Vec<String> {
    if let Some(entries) = payload_entry_array_as_entries(value) {
        return entries.iter().filter_map(value_line_for_entry).collect();
    }

    match value {
        Value::Null => Vec::new(),
        Value::Array(items) => items.iter().flat_map(data_value_lines).collect(),
        Value::Object(map) => map
            .values()
            .filter(|value| !value.is_null())
            .map(guide_value_to_display)
            .collect(),
        scalar => vec![guide_value_to_display(scalar)],
    }
}

fn payload_entry_array_as_entries(value: &Value) -> Option<Vec<GuideEntry>> {
    let Value::Array(items) = value else {
        return None;
    };

    items.iter().map(payload_entry_value_as_entry).collect()
}

fn payload_entry_value_as_entry(value: &Value) -> Option<GuideEntry> {
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

fn guide_value_to_display(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string().to_ascii_lowercase(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(guide_value_to_display)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(map) => {
            if map.is_empty() {
                return "{}".to_string();
            }
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let preview = keys
                .into_iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if map.len() > 3 {
                format!("{{{preview}, ...}}")
            } else {
                format!("{{{preview}}}")
            }
        }
    }
}

fn value_line_for_entry(entry: &GuideEntry) -> Option<String> {
    if !entry.short_help.trim().is_empty() {
        return Some(entry.short_help.clone());
    }
    if !entry.name.trim().is_empty() {
        return Some(entry.name.clone());
    }
    None
}

impl GuideSection {
    fn to_value(&self) -> Value {
        let mut section = Map::new();
        section.insert("title".to_string(), Value::String(self.title.clone()));
        section.insert(
            "kind".to_string(),
            Value::String(self.kind.as_str().to_string()),
        );
        section.insert("paragraphs".to_string(), string_array(&self.paragraphs));
        section.insert(
            "entries".to_string(),
            Value::Array(
                self.entries
                    .iter()
                    .map(payload_entry_value)
                    .collect::<Vec<_>>(),
            ),
        );
        if let Some(data) = self.data.as_ref() {
            section.insert("data".to_string(), data.clone());
        }
        Value::Object(section)
    }
}

impl GuideSection {
    /// Creates a new guide section with a title and canonical kind.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::guide::{GuideSection, GuideSectionKind};
    ///
    /// let section = GuideSection::new("Notes", GuideSectionKind::Notes)
    ///     .paragraph("first")
    ///     .entry("show", "Display");
    ///
    /// assert_eq!(section.paragraphs, vec!["first".to_string()]);
    /// assert_eq!(section.entries[0].name, "show");
    /// assert_eq!(section.entries[0].short_help, "Display");
    /// ```
    pub fn new(title: impl Into<String>, kind: GuideSectionKind) -> Self {
        Self {
            title: title.into(),
            kind,
            paragraphs: Vec::new(),
            entries: Vec::new(),
            data: None,
        }
    }

    /// Appends a paragraph to the section.
    pub fn paragraph(mut self, text: impl Into<String>) -> Self {
        self.paragraphs.push(text.into());
        self
    }

    /// Attaches semantic data to the section.
    ///
    /// Renderers may choose the best presentation for this payload instead of
    /// showing the authoring JSON literally.
    pub fn data(mut self, value: Value) -> Self {
        self.data = Some(value);
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
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::guide::GuideSectionKind;
    ///
    /// assert_eq!(GuideSectionKind::Commands.as_str(), "commands");
    /// assert_eq!(
    ///     GuideSectionKind::CommonInvocationOptions.as_str(),
    ///     "common_invocation_options"
    /// );
    /// ```
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
            data: section.get("data").cloned(),
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
mod tests;
