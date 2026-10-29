//! Semantic documents shared by help, intro, and structured command output.
//!
//! Documents describe content without prescribing its terminal or Markdown
//! layout. The UI lowers these blocks into its private rendering model.

mod template;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Detail level attached to a document section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelpLevel {
    /// Section is hidden at every help level.
    None,
    /// Section is shown in a compact synopsis.
    Tiny,
    /// Section is part of ordinary help.
    #[default]
    Normal,
    /// Section is shown only with verbose help.
    Verbose,
}

impl HelpLevel {
    /// All variants in canonical configuration order.
    pub const ALL: [Self; 4] = [Self::None, Self::Tiny, Self::Normal, Self::Verbose];

    /// Parses a configured help level.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "tiny" => Some(Self::Tiny),
            "normal" => Some(Self::Normal),
            "verbose" => Some(Self::Verbose),
            _ => None,
        }
    }

    /// Returns the canonical configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Tiny => "tiny",
            Self::Normal => "normal",
            Self::Verbose => "verbose",
        }
    }
}

/// Presentation role of a semantic section; titles never select layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SectionRole {
    /// Ordinary authored content.
    #[default]
    Content,
    /// A command synopsis, eligible for inline help layout.
    Usage,
    /// Options kept compact even in long help.
    CompactOptions,
}

/// Presentation role of a named entry, supplied by its producer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EntryRole {
    /// Ordinary named content.
    #[default]
    Text,
    /// An executable command name.
    Command,
    /// An option label with a short spelling.
    ShortOption,
    /// An option label with only a long spelling.
    LongOption,
}

/// Semantic identity of one document row, retained by row-preserving stages.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DocumentRowRole {
    /// Structured data, regardless of field names.
    #[default]
    Data,
    /// Authored paragraph or list text.
    Text,
    /// A named entry with its declared presentation role.
    Entry(EntryRole),
}

/// A semantic document whose blocks remain in authored order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// Ordered content blocks.
    pub blocks: Vec<DocBlock>,
}

impl Document {
    /// Parses passive intro templates, expanding help includes in authored order.
    pub fn from_markdown_template(text: &str, help: &Document) -> Self {
        template::parse(text, help)
    }

    /// Projects blocks into canonical rows in document order.
    pub fn rows(&self) -> Vec<crate::core::row::Row> {
        self.rows_with_roles().0
    }

    pub(crate) fn rows_with_roles(&self) -> (Vec<crate::core::row::Row>, Vec<DocumentRowRole>) {
        fn append(
            blocks: &[DocBlock],
            section: &str,
            rows: &mut Vec<crate::core::row::Row>,
            roles: &mut Vec<DocumentRowRole>,
        ) {
            for block in blocks {
                match block {
                    DocBlock::Section { title, blocks, .. } => append(blocks, title, rows, roles),
                    DocBlock::Entries { entries } => {
                        for entry in entries {
                            rows.push(crate::row! { "section" => section, "name" => entry.name.as_str(), "description" => entry.description.as_str() });
                            roles.push(DocumentRowRole::Entry(entry.role));
                        }
                    }
                    DocBlock::Paragraph { text } => {
                        for text in text.lines() {
                            rows.push(crate::row! { "section" => section, "text" => text });
                            roles.push(DocumentRowRole::Text);
                        }
                    }
                    DocBlock::Preformatted { lines } => {
                        for line in lines {
                            let text: String = line.iter().map(|span| span.text.as_str()).collect();
                            rows.push(crate::row! { "section" => section, "text" => text });
                            roles.push(DocumentRowRole::Text);
                        }
                    }
                    DocBlock::List { items } => {
                        for item in items {
                            for text in format!("- {item}").lines() {
                                rows.push(crate::row! { "section" => section, "text" => text });
                                roles.push(DocumentRowRole::Text);
                            }
                        }
                    }
                    DocBlock::Data { value } => {
                        for mut row in crate::core::output_model::rows_from_value(value.clone()) {
                            row.entry("section")
                                .or_insert_with(|| Value::String(section.into()));
                            rows.push(row);
                            roles.push(DocumentRowRole::Data);
                        }
                    }
                }
            }
        }
        let (mut rows, mut roles) = (Vec::new(), Vec::new());
        append(&self.blocks, "", &mut rows, &mut roles);
        (rows, roles)
    }

    /// Keeps the unstaged document alongside its canonical row projection.
    pub fn to_output_result(&self) -> crate::core::output_model::OutputResult {
        crate::core::output_model::OutputResult::from_rows(Vec::new()).with_document(
            crate::core::output_model::OutputDocument::Semantic(self.clone()),
        )
    }

    /// Builds help content from a Clap command's declared metadata.
    ///
    /// `long_help` selects long descriptions when Clap provides them. The
    /// command's rendered help text is never parsed.
    pub fn from_clap_command(command: &clap::Command, long_help: bool) -> Self {
        let mut command = command.clone();
        command.build();
        let mut blocks = Vec::new();
        let before_help = if long_help {
            command
                .get_before_long_help()
                .or_else(|| command.get_before_help())
        } else {
            command.get_before_help()
        }
        .map(ToString::to_string);
        if let Some(text) = before_help {
            push_text_blocks(&mut blocks, &text);
        }
        let about = if long_help {
            command.get_long_about().or_else(|| command.get_about())
        } else {
            command.get_about()
        };
        if let Some(text) = about {
            push_text_blocks(&mut blocks, &text.to_string());
        }

        let mut usage_command = command.clone();
        let usage = usage_command.render_usage().to_string();
        let usage = usage.strip_prefix("Usage:").unwrap_or(&usage).trim();
        if !usage.is_empty() {
            blocks.push(DocBlock::Section {
                role: SectionRole::Usage,
                title: "Usage".to_string(),
                level: HelpLevel::Tiny,
                blocks: vec![DocBlock::Paragraph {
                    text: usage.to_string(),
                }],
            });
        }

        let subcommands = command
            .get_subcommands()
            .filter(|subcommand| !subcommand.is_hide_set())
            // Clap owns parsing its generated help command; the host owns the
            // help surface. Explicit help commands remain ordinary entries.
            .filter(|subcommand| {
                subcommand.get_name() != "help" || command.is_disable_help_subcommand_set()
            })
            .map(|subcommand| DocumentEntry {
                role: EntryRole::Command,
                name: subcommand.get_name().to_string(),
                description: command_description(subcommand, false),
            })
            .collect::<Vec<_>>();
        if !subcommands.is_empty() {
            blocks.push(entries_section("Commands", subcommands));
        }

        let mut arguments = Vec::new();
        let mut option_groups: Vec<(String, Vec<DocumentEntry>)> = Vec::new();
        for arg in command.get_arguments().filter(|arg| {
            !arg.is_hide_set()
                && !(if long_help {
                    arg.is_hide_long_help_set()
                } else {
                    arg.is_hide_short_help_set()
                })
        }) {
            let entry = DocumentEntry {
                role: if arg.is_positional() {
                    EntryRole::Text
                } else if arg.get_short().is_some() {
                    EntryRole::ShortOption
                } else {
                    EntryRole::LongOption
                },
                name: clap_arg_label(arg),
                description: arg_description(arg, long_help),
            };
            if arg.is_positional() {
                arguments.push(entry);
            } else {
                let heading = arg.get_help_heading().unwrap_or("Options").to_string();
                if let Some((_, entries)) = option_groups
                    .iter_mut()
                    .find(|(current, _)| current == &heading)
                {
                    entries.push(entry);
                } else {
                    option_groups.push((heading, vec![entry]));
                }
            }
        }
        if !arguments.is_empty() {
            blocks.push(entries_section("Arguments", arguments));
        }
        for (heading, entries) in option_groups {
            blocks.push(entries_section(&heading, entries));
        }

        if let Some(text) = if long_help {
            command
                .get_after_long_help()
                .or_else(|| command.get_after_help())
        } else {
            command.get_after_help()
        } {
            push_text_blocks(&mut blocks, &text.to_string());
        }
        Self { blocks }
    }

    /// Parses external command help into semantic sections, paragraphs, and
    /// entry lists. Indented continuation lines extend the preceding entry.
    pub fn from_help_text(help_text: &str) -> Self {
        let mut document = Self::default();
        let mut section: Option<(String, Vec<DocBlock>)> = None;
        let mut entry_indent = None;
        // Where the current entry's description text starts; deeper lines,
        // even ones that begin with `-`, continue that description.
        let mut description_column: Option<usize> = None;
        let mut paragraph_break = false;

        for raw_line in help_text.lines() {
            let line = raw_line.trim_end();
            if line.trim().is_empty() {
                paragraph_break = true;
                continue;
            }
            let prefix = line.len().saturating_sub(line.trim_start().len());
            let indent = line[..prefix].chars().fold(0, |column, ch| {
                if ch == '\t' {
                    (column / 8 + 1) * 8
                } else {
                    column + 1
                }
            });
            let content = &line[prefix..];

            if indent == 0
                && let Some(usage) = content.strip_prefix("Usage:")
            {
                finish_help_section(&mut document.blocks, &mut section);
                entry_indent = None;
                description_column = None;
                section = Some(("Usage".to_string(), Vec::new()));
                let usage = usage.trim();
                if !usage.is_empty()
                    && let Some((_, blocks)) = section.as_mut()
                {
                    push_text_blocks(blocks, usage);
                }
                continue;
            }

            if indent == 0
                && let Some(title) = help_section_title(content)
            {
                finish_help_section(&mut document.blocks, &mut section);
                entry_indent = None;
                description_column = None;
                section = Some((title, Vec::new()));
                continue;
            }

            if indent == 0
                && section.as_ref().is_some_and(|(title, _)| {
                    matches!(
                        title.as_str(),
                        "Usage"
                            | "Commands"
                            | "Arguments"
                            | "Options"
                            | "Common Invocation Options"
                    )
                })
            {
                finish_help_section(&mut document.blocks, &mut section);
            }
            let option_label = content.starts_with('-')
                && content.split_whitespace().all(|token| {
                    token.starts_with(['-', '<', '['])
                        || token == "|"
                        || token.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_')
                });
            let bare_entry = option_label
                || section.as_ref().is_some_and(|(title, _)| {
                    matches!(title.as_str(), "Commands" | "Arguments") && !content.contains(' ')
                });
            let blocks = if let Some((_, blocks)) = section.as_mut() {
                blocks
            } else {
                &mut document.blocks
            };

            let mut entry = split_help_entry(content)
                .or_else(|| bare_entry.then(|| (content.to_string(), String::new())));
            if indent > 0
                && content.starts_with('-')
                && description_column.is_none_or(|column| indent < column)
                && let Some((name, description)) = entry.take()
            {
                entry_indent = Some(indent);
                description_column = description_start(indent, content, &description);
                paragraph_break = false;
                push_help_entry(
                    blocks,
                    DocumentEntry {
                        role: EntryRole::Text,
                        name,
                        description,
                    },
                );
            } else if let Some(base_indent) = entry_indent
                && indent > base_indent
                && append_help_continuation(blocks, content, std::mem::take(&mut paragraph_break))
            {
                description_column.get_or_insert(indent);
                continue;
            } else if indent > 0
                && let Some((name, description)) = entry
            {
                entry_indent = Some(indent);
                description_column = description_start(indent, content, &description);
                paragraph_break = false;
                push_help_entry(
                    blocks,
                    DocumentEntry {
                        role: EntryRole::Text,
                        name,
                        description,
                    },
                );
            } else {
                entry_indent = None;
                description_column = None;
                paragraph_break = false;
                push_text_blocks(blocks, content);
            }
        }

        finish_help_section(&mut document.blocks, &mut section);
        document
    }

    /// Keeps sections available at `level`, preserving all other block order.
    pub fn filtered_for_help_level(&self, level: HelpLevel) -> Self {
        fn filter_blocks(blocks: &[DocBlock], level: HelpLevel) -> Vec<DocBlock> {
            blocks
                .iter()
                .filter_map(|block| match block {
                    DocBlock::Section {
                        title,
                        role,
                        level: section_level,
                        blocks,
                    } if *section_level != HelpLevel::None && level >= *section_level => {
                        Some(DocBlock::Section {
                            title: title.clone(),
                            role: *role,
                            level: *section_level,
                            blocks: filter_blocks(blocks, level),
                        })
                    }
                    DocBlock::Section { .. } => None,
                    other => Some(other.clone()),
                })
                .collect()
        }

        Self {
            blocks: filter_blocks(&self.blocks, level),
        }
    }
}

fn entries_section(title: &str, entries: Vec<DocumentEntry>) -> DocBlock {
    DocBlock::Section {
        role: Default::default(),
        title: title.to_string(),
        level: HelpLevel::Normal,
        blocks: vec![DocBlock::Entries { entries }],
    }
}

fn command_description(command: &clap::Command, long_help: bool) -> String {
    if long_help {
        command.get_long_about().or_else(|| command.get_about())
    } else {
        command.get_about()
    }
    .map(ToString::to_string)
    .unwrap_or_default()
}

fn arg_description(arg: &clap::Arg, long_help: bool) -> String {
    let help = if long_help {
        arg.get_long_help().or_else(|| arg.get_help())
    } else {
        arg.get_help()
    };
    let mut description = help.map(ToString::to_string).unwrap_or_default();
    let mut annotations = Vec::new();
    if arg.get_action().takes_values()
        && !arg.is_hide_default_value_set()
        && !arg.get_default_values().is_empty()
    {
        annotations.push(format!(
            "[default: {}]",
            arg.get_default_values()
                .iter()
                .map(|value| value.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    if !arg.is_hide_possible_values_set() {
        let values = arg
            .get_possible_values()
            .into_iter()
            .filter(|value| !value.is_hide_set())
            .map(|value| value.get_name().to_string())
            .collect::<Vec<_>>();
        if !values.is_empty() {
            annotations.push(format!("[possible values: {}]", values.join(", ")));
        }
    }
    let aliases = arg
        .get_visible_aliases()
        .unwrap_or_default()
        .into_iter()
        .map(|alias| format!("--{alias}"))
        .chain(
            arg.get_visible_short_aliases()
                .unwrap_or_default()
                .into_iter()
                .map(|alias| format!("-{alias}")),
        )
        .collect::<Vec<_>>();
    if !aliases.is_empty() {
        annotations.push(format!("[aliases: {}]", aliases.join(", ")));
    }
    if !annotations.is_empty() {
        if !description.is_empty() {
            description.push_str(if long_help { "\n" } else { " " });
        }
        description.push_str(&annotations.join(" "));
    }
    description
}

fn clap_arg_label(arg: &clap::Arg) -> String {
    let mut label = arg.to_string();
    if arg.get_value_names().is_none() {
        let id = arg.get_id().as_str();
        label = label.replace(
            &format!("<{id}>"),
            &format!("<{}>", id.to_ascii_uppercase()),
        );
    }
    match (arg.get_short(), arg.get_long()) {
        (Some(short), Some(_)) => format!("-{short}, {label}"),
        _ => label,
    }
}

fn help_section_title(line: &str) -> Option<String> {
    let title = line.strip_suffix(':')?.trim();
    (!title.is_empty() && !title.contains('\n')).then(|| title.to_string())
}

fn split_help_entry(line: &str) -> Option<(String, String)> {
    let mut whitespace_start = None;
    let mut whitespace_len = 0;
    for (index, ch) in line.char_indices() {
        if ch.is_whitespace() {
            whitespace_start.get_or_insert(index);
            whitespace_len += ch.len_utf8();
        } else if whitespace_len >= 2 {
            let start = whitespace_start?;
            let name = line[..start].trim();
            if name.is_empty() {
                return None;
            }
            return Some((name.to_string(), line[index..].trim().to_string()));
        } else {
            whitespace_start = None;
            whitespace_len = 0;
        }
    }
    None
}

/// The column of an entry's inline description, if it has one.
fn description_start(indent: usize, content: &str, description: &str) -> Option<usize> {
    if description.is_empty() {
        return None;
    }
    content
        .rfind(description)
        .map(|offset| indent + content[..offset].chars().count())
}

fn push_help_entry(blocks: &mut Vec<DocBlock>, entry: DocumentEntry) {
    if let Some(DocBlock::Entries { entries }) = blocks.last_mut() {
        entries.push(entry);
    } else {
        blocks.push(DocBlock::Entries {
            entries: vec![entry],
        });
    }
}

fn append_help_continuation(blocks: &mut [DocBlock], text: &str, paragraph_break: bool) -> bool {
    let Some(DocBlock::Entries { entries }) = blocks.last_mut() else {
        return false;
    };
    let Some(entry) = entries.last_mut() else {
        return false;
    };
    if !entry.description.is_empty() {
        entry
            .description
            .push(if paragraph_break { '\n' } else { ' ' });
    }
    entry.description.push_str(text.trim());
    true
}

fn push_text_blocks(blocks: &mut Vec<DocBlock>, text: &str) {
    let normalized = text.trim();
    if normalized.is_empty() {
        return;
    }
    if let Some(DocBlock::Paragraph { text }) = blocks.last_mut() {
        text.push('\n');
        text.push_str(normalized);
    } else {
        blocks.push(DocBlock::Paragraph {
            text: normalized.to_string(),
        });
    }
}

fn finish_help_section(blocks: &mut Vec<DocBlock>, section: &mut Option<(String, Vec<DocBlock>)>) {
    let Some((title, mut section_blocks)) = section.take() else {
        return;
    };
    for block in &mut section_blocks {
        if let DocBlock::Entries { entries } = block {
            for entry in entries {
                entry.role = if title == "Commands" {
                    EntryRole::Command
                } else if entry.name.starts_with("--") {
                    EntryRole::LongOption
                } else if entry.name.starts_with('-') {
                    EntryRole::ShortOption
                } else {
                    EntryRole::Text
                };
            }
        }
    }
    blocks.push(DocBlock::Section {
        role: match title.as_str() {
            "Usage" => SectionRole::Usage,
            "Common Invocation Options" | "Product options" => SectionRole::CompactOptions,
            _ => SectionRole::Content,
        },
        title: title.clone(),
        level: match title.as_str() {
            "Usage" => HelpLevel::Tiny,
            "Common Invocation Options" => HelpLevel::Normal,
            _ => HelpLevel::Normal,
        },
        blocks: section_blocks,
    });
}

/// A semantic document block.
///
/// The untagged serialized form uses each block's content key directly. This
/// keeps the JSON contract compact: a section is `{"section":"Options", ...}`
/// and an entries block is `{"entries":[...]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum DocBlock {
    /// A titled block group.
    Section {
        /// Authored presentation role; excluded from canonical data exports.
        #[serde(skip)]
        role: SectionRole,
        /// User-facing section title.
        #[serde(rename = "section")]
        title: String,
        /// Minimum help level required to include this section.
        #[serde(default, skip_serializing_if = "is_normal_level")]
        level: HelpLevel,
        /// Ordered content within the section.
        blocks: Vec<DocBlock>,
    },
    /// Free-form text.
    Paragraph {
        /// Paragraph contents.
        text: String,
    },
    /// Authored lines with explicit semantic styling and literal spacing.
    Preformatted {
        /// Ordered lines; spans on each line are concatenated without separators.
        lines: Vec<Vec<TextSpan>>,
    },
    /// Named entries such as commands, arguments, and options.
    Entries {
        /// Ordered entries.
        entries: Vec<DocumentEntry>,
    },
    /// A list of author-provided items.
    List {
        /// Ordered list items.
        items: Vec<String>,
    },
    /// Structured JSON data.
    Data {
        /// The JSON payload.
        value: Value,
    },
}

/// Producer-declared meaning of a text span, resolved through the UI theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextRole {
    /// Ordinary text.
    #[default]
    Text,
    /// Secondary context or inactive content.
    Muted,
    /// Informational activity.
    Info,
    /// Successful completion.
    Success,
    /// A warning or condition needing attention.
    Warning,
    /// A failed operation.
    Error,
    /// A label or identifier.
    Key,
}

/// Literal text with a semantic role, never a terminal escape sequence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextSpan {
    /// Literal content; terminal controls are sanitized by human projection.
    pub text: String,
    /// Presentation intent, excluded from canonical document exports.
    #[serde(skip)]
    pub role: TextRole,
}

impl TextSpan {
    /// Associates literal content with a theme-resolved semantic role.
    pub fn new(text: impl Into<String>, role: TextRole) -> Self {
        Self {
            text: text.into(),
            role,
        }
    }
}

/// One named entry in a document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentEntry {
    /// Authored presentation role; excluded from canonical data exports.
    #[serde(skip)]
    pub role: EntryRole,
    /// Stable entry label.
    pub name: String,
    /// Description paired with the label.
    pub description: String,
}

impl DocBlock {
    /// Sets the presentation role of a section without interpreting its title.
    pub fn with_role(mut self, value: SectionRole) -> Self {
        if let Self::Section { role, level, .. } = &mut self {
            *role = value;
            if value == SectionRole::Usage {
                *level = HelpLevel::Tiny;
            }
        }
        self
    }

    /// Groups content under a heading, with the conventional help detail level.
    pub fn section(title: impl Into<String>, blocks: Vec<DocBlock>) -> Self {
        let title = title.into();
        Self::Section {
            title,
            role: SectionRole::Content,
            level: HelpLevel::Normal,
            blocks,
        }
    }
}

fn is_normal_level(level: &HelpLevel) -> bool {
    *level == HelpLevel::Normal
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::{DocBlock, Document, DocumentEntry, HelpLevel};
    use serde_json::json;

    #[test]
    fn document_contract_covers_serde_and_help_sources_unit() {
        let document = Document {
            blocks: vec![DocBlock::Section {
                role: Default::default(),
                title: "Options".to_string(),
                level: HelpLevel::Normal,
                blocks: vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "--json".to_string(),
                        description: "Emit JSON".to_string(),
                    }],
                }],
            }],
        };
        let value = json!({
            "blocks": [{
                "section": "Options",
                "blocks": [{
                    "entries": [{"name": "--json", "description": "Emit JSON"}]
                }]
            }]
        });

        assert_eq!(serde_json::to_value(&document).unwrap(), value);
        assert_eq!(serde_json::from_value::<Document>(value).unwrap(), document);

        let parsed = Document::from_help_text(
            "Usage: osp config\n\nOptions:\n  --user <USER>  Select the user\n            across profiles\n  explanatory text\n",
        );
        let DocBlock::Section { blocks, .. } = &parsed.blocks[1] else {
            panic!("expected parsed options section");
        };
        let DocBlock::Entries { entries } = &blocks[0] else {
            panic!("expected parsed option entries");
        };
        assert_eq!(entries[0].description, "Select the user across profiles");
        assert!(matches!(
            blocks.get(1),
            Some(DocBlock::Paragraph { text }) if text == "explanatory text"
        ));

        let template = Document::from_markdown_template(
            "- parent\n  - child\n- next\n\n- loose\n\n- item\n",
            &Document::default(),
        );
        let items = template
            .blocks
            .iter()
            .filter_map(|block| match block {
                DocBlock::List { items } => Some(items.as_slice()),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        assert!(
            items
                .iter()
                .any(|item| item.contains("parent") && item.contains("child"))
        );
        assert!(items.iter().any(|item| *item == "next"));
        assert!(items.iter().any(|item| *item == "loose"));
        assert!(items.iter().any(|item| *item == "item"));

        let command = clap::Command::new("demo")
            .about("Short summary")
            .long_about("Long command summary")
            .arg(
                clap::Arg::new("user")
                    .long("user")
                    .help("Short option help")
                    .long_help("Long option help"),
            );
        let metadata = Document::from_clap_command(&command, true);
        assert!(matches!(
            metadata.blocks.first(),
            Some(DocBlock::Paragraph { text }) if text == "Long command summary"
        ));
        assert!(metadata.blocks.iter().any(|block| matches!(
            block,
            DocBlock::Section { title, blocks, .. }
                if title == "Options"
                    && matches!(blocks.first(), Some(DocBlock::Entries { entries })
                        if entries.iter().any(|entry| entry.name == "--user <USER>"
                            && entry.description == "Long option help"))
        )));
    }
}
