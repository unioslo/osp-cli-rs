//! Structured output payload model shared across commands, DSL stages, and UI.
//!
//! This module exists to keep command results in a small canonical shape while
//! they move between execution, transformation, and rendering layers.
//!
//! High-level flow:
//!
//! - commands produce [`crate::core::OutputResult`]
//! - the DSL transforms its [`crate::core::OutputItems`] and
//!   optional semantic document
//! - the UI later lowers the result into rendered documents and terminal text
//!
//! Contract:
//!
//! - this module describes data shape, not rendering policy
//! - semantic sidecar documents should stay canonical here instead of leaking
//!   format-specific assumptions into the DSL or UI

use crate::core::document::Document;
use crate::core::output::OutputFormat;
use crate::core::row::Row;
use serde_json::Value;
use std::collections::HashSet;

/// Producer-declared human display changes. Conditions use original row values;
/// rules never change canonical data or DSL field values. Collection rules accept
/// `*` for array members and numeric indices for one member. Unless a literal key
/// is required, selectors accept top-level keys, dotted paths, or JSON Pointers.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayRule {
    /// Join scalar array values without changing the typed array.
    Join {
        /// Canonical array selector.
        field: String,
        /// Literal separator for human values.
        separator: String,
    },
    /// Replace a literal spelling in human values only.
    Replace {
        /// Canonical string selector.
        field: String,
        /// Literal spelling to replace.
        from: String,
        /// Human spelling.
        to: String,
    },
    /// Replace the standard empty human result text without adding payload data.
    EmptyText {
        /// Plain human text for an empty result.
        text: String,
    },
    /// Humanize a numeric duration expressed in seconds.
    DurationSeconds {
        /// Canonical numeric selector.
        field: String,
    },
    /// Show elapsed time since a canonical RFC3339 timestamp.
    ElapsedTimestamp {
        /// Canonical timestamp selector.
        field: String,
    },
    /// Display a numeric quotient without changing the canonical value or unit.
    Divide {
        /// Canonical numeric selector.
        field: String,
        /// Nonzero divisor for the displayed number.
        divisor: u64,
    },
    /// Omit a matching trailing qualifier in human values only.
    StripQualifier {
        /// Canonical string selector.
        field: String,
        /// Delimiter between the name and its single qualifier.
        separator: String,
        /// Complete qualifier to omit when it matches.
        qualifier: String,
    },
    /// Label nested record keys without changing canonical selectors.
    Labels {
        /// Canonical collection selector.
        field: String,
        /// Canonical keys mapped to human labels recursively.
        labels: std::collections::BTreeMap<String, String>,
    },
    /// Show an absolute timestamp followed by its relative age or remaining time.
    Timestamp {
        /// Canonical RFC3339 timestamp selector.
        field: String,
    },
    /// Attribute differing source values, leaving agreement unadorned.
    SourceValues {
        /// Canonical selected value.
        field: String,
        /// Selector for the source names of the selected value.
        sources: String,
        /// Selector for differing source/value records.
        conflicts: String,
        /// Source label for an authoritative value without explicit provenance.
        default_source: String,
        /// Source names mapped to human labels.
        labels: std::collections::BTreeMap<String, String>,
    },
    /// Sum inclusive IP ranges without adding an aggregate to canonical JSON.
    AddressRangeTotal {
        /// Canonical array of range records.
        ranges: String,
        /// Start-address field within each range.
        start: String,
        /// End-address field within each range.
        end: String,
        /// Human-only total field to insert.
        field: String,
    },
    /// Derive remaining address capacity from a CIDR and an exact used count.
    AddressCountRemaining {
        /// Canonical CIDR field.
        network: String,
        /// Canonical used-address integer field.
        used: String,
        /// Human-only field to insert.
        field: String,
    },
    /// Substitute a human label for null without changing the payload.
    NullText {
        /// Canonical field selector.
        field: String,
        /// Text displayed only for null values.
        text: String,
    },
    /// Format an ISO date or datetime using a chrono strftime pattern.
    Date {
        /// Canonical ISO date or datetime field selector.
        field: String,
        /// Chrono strftime pattern for human output.
        pattern: String,
    },
    /// Display a number with a fixed decimal precision.
    Decimal {
        /// Canonical numeric field selector.
        field: String,
        /// Fractional digits, capped at 4096 by the renderer.
        precision: usize,
    },
    /// Limit a non-null value's displayed character count.
    TextLimit {
        /// Canonical field selector.
        field: String,
        /// Maximum displayed Unicode character count.
        length: usize,
    },
    /// Display a value as fields interspersed with literal wording.
    /// Selectors in parts are relative to that value, after scalar rules.
    /// An empty part selector references the entire value. Root composition
    /// renders through the ordinary scalar `value` row.
    Compose {
        /// Canonical object or array field selector.
        field: String,
        /// Ordered literal and field spans in the displayed value.
        parts: Vec<DisplayPart>,
    },
    /// Render a top-level field as an unlabelled notice above an MREG card.
    Notice {
        /// Literal top-level field name, after human display composition.
        field: String,
    },
    /// Render a field with the theme's warning style in human output only.
    Warning {
        /// Canonical field selector.
        field: String,
    },
    /// Paint a colour/style specification using its own style in terminal cells.
    ValueStyle {
        /// Canonical string field containing a validated style specification.
        field: String,
        /// Emphasize the value in addition to its declared style.
        bold: bool,
    },
    /// Show a valid colour as a two-cell block in coloured human output.
    /// Plain output retains the original value; machine data is unchanged.
    Swatch {
        /// Canonical string field containing a colour/style specification.
        field: String,
    },
    /// Mark a field as a command line for human styling.
    Command {
        /// Canonical command field selector.
        field: String,
    },
    /// Link an HTTP(S) URL value in rich terminal output only.
    Hyperlink {
        /// Canonical string field containing the URL and visible link text.
        field: String,
    },
    /// Hide array members marked by a boolean field in human output only.
    HideArrayItemsWhenTrue {
        /// Canonical array field path to project.
        field: String,
        /// Boolean path within each array member.
        when: String,
    },
    /// Append a literal suffix to a string when a boolean field is true.
    SuffixWhenTrue {
        /// Canonical field path to decorate.
        field: String,
        /// Canonical boolean field path controlling the decoration.
        when: String,
        /// Literal suffix, including any desired spacing or punctuation.
        suffix: String,
    },
    /// Display a field as blank when another field exists and is not null.
    BlankWhenPresent {
        /// Canonical field path to blank.
        field: String,
        /// Canonical field path whose non-null presence blanks the field.
        when: String,
    },
    /// Format a non-negative integer byte count using binary units.
    Bytes {
        /// Canonical numeric field path to format.
        field: String,
    },
    /// Format an RFC3339 or Unix-seconds timestamp relative to the current time.
    RelativeTimestamp {
        /// Canonical timestamp field path to format.
        field: String,
    },
}

/// Inline content references data; producers never interpolate field values.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayPart {
    /// Literal punctuation or wording surrounding a value.
    Literal {
        /// Wording that contains no interpolated response data.
        text: String,
    },
    /// A typed value within the composed object.
    Field {
        /// Selector relative to the composed object.
        field: String,
    },
}

/// Alignment hint for a rendered output column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlignment {
    /// Use renderer defaults for alignment.
    #[default]
    Default,
    /// Left-align the column.
    Left,
    /// Center-align the column.
    Center,
    /// Right-align the column.
    Right,
}

/// Grouped output with grouping keys, aggregate values, and member rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Values that identify the group.
    pub groups: Row,
    /// Aggregate values computed for the group.
    pub aggregates: Row,
    /// Member rows belonging to the group.
    pub rows: Vec<Row>,
}

/// Output identity and rendering metadata attached to an [`OutputResult`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputMeta {
    /// Human-only notes after the displayed result; absent from data exports.
    pub display_notes: Vec<String>,
    /// Remembered arguments used by the producing command.
    pub context: std::collections::BTreeMap<String, String>,
    /// Stable first-seen column order for row rendering.
    pub key_index: Vec<String>,
    /// Optional curated field paths for human display, in priority order.
    /// Dotted paths and JSON Pointers both support human field selection.
    pub display_columns: Option<Vec<String>>,
    /// Optional literal record field to consume as a human card heading.
    pub title_field: Option<String>,
    /// Optional human labels aligned positionally with `display_columns`.
    pub display_column_labels: Option<Vec<String>>,
    /// Producer-declared numeric timestamp paths, measured in Unix seconds.
    /// Uses the same selectors as `display_columns`.
    pub unix_timestamp_columns: Vec<String>,
    /// Human-only conditional field formatting, evaluated against raw rows.
    pub display_rules: Vec<DisplayRule>,
    /// Per-column alignment hints.
    pub column_align: Vec<ColumnAlignment>,
    /// Whether the result should be easy to copy as plain text.
    pub wants_copy: bool,
    /// Explicitly page rendered output, regardless of its length.
    pub wants_page: bool,
    /// Open rendered output in an editor before any copy sink.
    pub wants_editor: bool,
    /// Whether the payload represents grouped data, including collections inside
    /// a semantic document. Pipeline continuations preserve this identity;
    /// service field names alone must never set it.
    pub grouped: bool,
    /// Rows explicitly produced by a semantic document, with section context.
    pub document_rows: bool,
    /// Semantic row roles aligned with document rows; never inferred from keys.
    pub document_row_roles: Vec<crate::core::document::DocumentRowRole>,
    /// Optional minimum gap after aligned document entry names.
    pub document_entry_gap: Option<usize>,
    /// Commands that require literal spelling in this help listing.
    pub document_literal_commands: Vec<String>,
    /// This help listing describes commands handled by the prefix resolver.
    pub document_command_prefixes: bool,
    /// Use clap long-help entry layout for builtin option descriptions.
    pub document_long_help: bool,
    /// Preferred renderer for this result, when known.
    pub render_recommendation: Option<RenderRecommendation>,
    /// Human-only cap on rows shown; pipeline stages clear it because they
    /// work on, and show, every row.
    pub display_limit: Option<usize>,
}

/// Suggested render target for a command result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderRecommendation {
    /// Render using the specified output format.
    Format(OutputFormat),
    /// Render as semantic document content.
    Document,
}

/// Unstaged semantic content or an original service JSON envelope.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputDocument {
    /// Ordered semantic blocks for direct human rendering.
    Semantic(Document),
    /// Canonical JSON supplied by a command, before row projection.
    Json(Value),
}

impl OutputDocument {
    /// Returns the semantic document when this payload contains one.
    pub fn semantic(&self) -> Option<&Document> {
        match self {
            Self::Semantic(document) => Some(document),
            Self::Json(_) => None,
        }
    }

    /// Serializes semantic content only at the JSON output boundary.
    pub fn json_value(&self) -> Value {
        match self {
            Self::Semantic(document) => serde_json::json!(document),
            Self::Json(value) => value.clone(),
        }
    }
}

/// Result payload as either flat rows or grouped rows.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputItems {
    /// Ungrouped row output.
    Rows(Vec<Row>),
    /// Grouped output with aggregates and member rows.
    Groups(Vec<Group>),
}

/// Structured command output plus rendering metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputResult {
    /// Primary payload to render or further transform.
    pub items: OutputItems,
    /// Unstaged semantic view or original service JSON envelope. Pipeline stages
    /// discard it before transforming `items`. Semantic views serialize as
    /// documents only when `meta.document_rows` marks their row projection;
    /// presentation annotations keep canonical row JSON instead.
    pub document: Option<OutputDocument>,
    /// Rendering metadata derived during result construction.
    pub meta: OutputMeta,
}

impl OutputResult {
    /// Builds a row-based result and derives its key index from first-seen columns.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::OutputResult;
    /// use osp_cli::row;
    ///
    /// let output = OutputResult::from_rows(vec![
    ///     row! { "uid" => "alice", "mail" => "a@example.com" },
    ///     row! { "uid" => "bob", "cn" => "Bob" },
    /// ]);
    ///
    /// assert_eq!(output.meta.key_index, vec!["uid", "mail", "cn"]);
    /// ```
    pub fn from_rows(rows: Vec<Row>) -> Self {
        let key_index = compute_key_index(&rows);
        Self {
            items: OutputItems::Rows(rows),
            document: None,
            meta: OutputMeta {
                key_index,
                ..OutputMeta::default()
            },
        }
    }

    /// Attaches a semantic document to the result and returns the updated value.
    ///
    /// Replaces existing payload rows with rows derived from the document and
    /// rebuilds the key index. This is a payload replacement, not an annotation.
    ///
    #[must_use]
    pub fn with_document(mut self, document: OutputDocument) -> Self {
        self.meta.document_rows = matches!(document, OutputDocument::Semantic(_));
        if self.meta.document_rows {
            self.meta.render_recommendation = Some(RenderRecommendation::Document);
        } else if self.meta.render_recommendation == Some(RenderRecommendation::Document) {
            self.meta.render_recommendation = None;
        }
        self.items = OutputItems::Rows(match &document {
            OutputDocument::Semantic(document) => {
                let (rows, roles) = document.rows_with_roles();
                self.meta.document_row_roles = roles;
                rows
            }
            OutputDocument::Json(value) => {
                self.meta.document_row_roles.clear();
                rows_from_value(value.clone())
            }
        });
        self.meta.key_index = compute_key_index(self.as_rows().unwrap_or_default());
        self.document = Some(document);
        self
    }

    /// Returns the underlying rows when the result is not grouped.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::OutputResult;
    /// use osp_cli::row;
    ///
    /// let output = OutputResult::from_rows(vec![row! { "uid" => "alice" }]);
    ///
    /// assert_eq!(output.as_rows().unwrap()[0]["uid"], "alice");
    /// ```
    pub fn as_rows(&self) -> Option<&[Row]> {
        match &self.items {
            OutputItems::Rows(rows) => Some(rows),
            OutputItems::Groups(_) => None,
        }
    }

    /// Consumes the result and returns its rows when the payload is row-based.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::OutputResult;
    /// use osp_cli::row;
    ///
    /// let rows = OutputResult::from_rows(vec![row! { "uid" => "alice" }])
    ///     .into_rows()
    ///     .unwrap();
    ///
    /// assert_eq!(rows[0]["uid"], "alice");
    /// ```
    pub fn into_rows(self) -> Option<Vec<Row>> {
        match self.items {
            OutputItems::Rows(rows) => Some(rows),
            OutputItems::Groups(_) => None,
        }
    }
}

pub(crate) fn output_items_to_rows(items: &OutputItems) -> Vec<Row> {
    match items {
        OutputItems::Rows(rows) => rows.clone(),
        OutputItems::Groups(groups) => groups.iter().flat_map(group_rows).collect(),
    }
}

pub(crate) fn group_rows(group: &Group) -> Vec<Row> {
    if group.rows.is_empty() {
        return vec![group_header_row(group)];
    }

    group
        .rows
        .iter()
        .map(|row| group_member_row(group, row))
        .collect()
}

pub(crate) fn group_header_row(group: &Group) -> Row {
    let mut row = group.groups.clone();
    row.extend(group.aggregates.clone());
    row
}

pub(crate) fn group_member_row(group: &Group, row: &Row) -> Row {
    let mut merged = group_header_row(group);
    merged.extend(row.clone());
    merged
}

/// Computes the stable first-seen column order across all rows.
///
/// # Examples
///
/// ```text
/// use osp_cli::core::output_model::compute_key_index;
/// use osp_cli::row;
///
/// let rows = vec![
///     row! { "uid" => "alice", "mail" => "a@example.com" },
///     row! { "uid" => "bob", "cn" => "Bob" },
/// ];
///
/// assert_eq!(compute_key_index(&rows), vec!["uid", "mail", "cn"]);
/// ```
pub(crate) fn compute_key_index(rows: &[Row]) -> Vec<String> {
    let mut key_index = Vec::new();
    let mut seen = HashSet::new();

    for row in rows {
        for key in row.keys() {
            if seen.insert(key.clone()) {
                key_index.push(key.clone());
            }
        }
    }

    key_index
}

/// Projects output items into a canonical JSON value.
///
/// # Examples
///
/// ```text
/// use osp_cli::core::output_model::{OutputItems, output_items_to_value};
/// use osp_cli::row;
///
/// let value = output_items_to_value(&OutputItems::Rows(vec![row! { "uid" => "alice" }]));
///
/// assert_eq!(value["uid"], "alice");
/// ```
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn output_items_to_value(items: &OutputItems) -> Value {
    match items {
        OutputItems::Rows(rows) => match rows.as_slice() {
            [row] => Value::Object(row.clone()),
            _ => Value::Array(rows.iter().cloned().map(Value::Object).collect()),
        },
        OutputItems::Groups(groups) => Value::Array(
            groups
                .iter()
                .map(|group| {
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
                })
                .collect::<Vec<_>>(),
        ),
    }
}

/// Decodes JSON that may contain engine-generated group envelopes.
///
/// Use this inverse bridge only when group identity is already known, such as
/// after an explicit DSL grouping stage. It recognizes the serialized group
/// shape; arbitrary service JSON must use [`rows_from_value`] instead, since
/// a service may legitimately use the same field names.
///
/// # Examples
///
/// ```text
/// use osp_cli::core::output_model::{OutputItems, output_items_from_value};
/// use serde_json::json;
///
/// let items = output_items_from_value(json!({"uid": "alice"}));
///
/// assert_eq!(
///     items,
///     OutputItems::Rows(vec![json!({"uid": "alice"}).as_object().cloned().unwrap()])
/// );
/// ```
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn output_items_from_value(value: Value) -> OutputItems {
    if let Value::Array(items) = &value
        && !items.is_empty()
        && let Some(groups) = groups_from_values(items)
    {
        return OutputItems::Groups(groups);
    }
    OutputItems::Rows(rows_from_value(value))
}

/// Projects any JSON value into a row stream for pipeline-oriented processing.
///
/// Arrays expand into one row per item. Objects become single rows directly.
/// Scalars become a single row under the canonical `value` column.
pub(crate) fn rows_from_value(value: Value) -> Vec<Row> {
    match value {
        Value::Array(values) => values.into_iter().map(row_from_value).collect(),
        other => vec![row_from_value(other)],
    }
}

fn row_from_value(value: Value) -> Row {
    match value {
        Value::Object(map) => map,
        other => row_with_value(other),
    }
}

fn row_with_value(value: Value) -> Row {
    let mut row = Row::new();
    row.insert("value".to_string(), value);
    row
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
fn groups_from_values(values: &[Value]) -> Option<Vec<Group>> {
    values.iter().map(group_from_value).collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
fn group_from_value(value: &Value) -> Option<Group> {
    let Value::Object(map) = value else {
        return None;
    };
    // This bridge reads engine group envelopes, not arbitrary service records.
    // Never discard additional fields just because three familiar keys exist.
    if map.len() != 3 {
        return None;
    }
    let groups = map.get("groups")?.as_object()?.clone();
    let aggregates = map.get("aggregates")?.as_object()?.clone();
    let Value::Array(rows) = map.get("rows")? else {
        return None;
    };
    let rows = rows
        .iter()
        .map(|row| row.as_object().cloned())
        .collect::<Option<Vec<_>>>()?;

    Some(Group {
        groups,
        aggregates,
        rows,
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {

    #[test]
    fn internal_example_2() {
        use crate::core::output_model::{OutputItems, output_items_to_value};
        use crate::row;

        let value = output_items_to_value(&OutputItems::Rows(vec![row! { "uid" => "alice" }]));

        assert_eq!(value["uid"], "alice");
    }

    #[test]
    fn internal_example_3() {
        use crate::core::output_model::{OutputItems, output_items_from_value};
        use serde_json::json;

        let items = output_items_from_value(json!({"uid": "alice"}));

        assert_eq!(
            items,
            OutputItems::Rows(vec![json!({"uid": "alice"}).as_object().cloned().unwrap()])
        );
    }

    use super::{
        Group, OutputDocument, OutputItems, OutputMeta, OutputResult, output_items_from_value,
        output_items_to_rows, output_items_to_value,
    };
    use serde_json::Value;
    use serde_json::json;

    #[test]
    fn row_results_keep_first_seen_key_order_and_expose_row_views_unit() {
        let rows = vec![
            json!({"uid": "oistes", "cn": "Oistein"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"mail": "o@uio.no", "uid": "oistes", "title": "Engineer"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = OutputResult::from_rows(rows.clone());
        assert_eq!(output.meta.key_index, vec!["uid", "cn", "mail", "title"]);
        assert_eq!(output.as_rows(), Some(rows.as_slice()));
        assert_eq!(output.into_rows(), Some(rows));
    }

    #[test]
    fn grouped_results_and_semantic_documents_cover_non_row_views_unit() {
        let grouped_output = OutputResult {
            items: OutputItems::Groups(vec![Group {
                groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
                aggregates: json!({"count": 1}).as_object().cloned().expect("object"),
                rows: vec![
                    json!({"user": "alice"})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ],
            }]),
            document: None,
            meta: OutputMeta::default(),
        };

        assert_eq!(grouped_output.as_rows(), None);
        assert_eq!(grouped_output.into_rows(), None);

        let document_output =
            OutputResult::from_rows(Vec::new()).with_document(OutputDocument::Semantic(
                serde_json::from_value(
                    json!({"blocks": [{"section": "Usage", "blocks": [{"text": "osp"}]}]}),
                )
                .expect("semantic document"),
            ));

        assert!(matches!(
            document_output.document,
            Some(OutputDocument::Semantic(_))
        ));
    }

    #[test]
    fn output_items_projection_round_trips_rows_and_groups_unit() {
        let rows = OutputItems::Rows(vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
        ]);
        let rows_value = output_items_to_value(&rows);
        assert!(matches!(rows_value, Value::Object(_)));
        assert_eq!(output_items_from_value(rows_value), rows);

        let groups = OutputItems::Groups(vec![Group {
            groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
            aggregates: json!({"count": 1}).as_object().cloned().expect("object"),
            rows: vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
        }]);
        let groups_value = output_items_to_value(&groups);
        assert!(matches!(groups_value, Value::Array(_)));
        assert_eq!(output_items_from_value(groups_value), groups);
    }

    #[test]
    fn output_items_from_value_fans_out_scalar_arrays_and_empty_arrays_unit() {
        assert_eq!(
            output_items_from_value(json!(["alice", "bob"])),
            OutputItems::Rows(vec![
                json!({"value": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object"),
                json!({"value": "bob"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ])
        );
        assert_eq!(
            output_items_from_value(json!([])),
            OutputItems::Rows(Vec::new())
        );
    }

    #[test]
    fn output_items_to_rows_merges_group_context_once_unit() {
        let items = OutputItems::Groups(vec![
            Group {
                groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
                aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
                rows: vec![
                    json!({"user": "alice"})
                        .as_object()
                        .cloned()
                        .expect("object"),
                    json!({"user": "bob"}).as_object().cloned().expect("object"),
                ],
            },
            Group {
                groups: json!({"team": "dev"}).as_object().cloned().expect("object"),
                aggregates: json!({"count": 0}).as_object().cloned().expect("object"),
                rows: Vec::new(),
            },
        ]);

        let rows = output_items_to_rows(&items);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["team"], "ops");
        assert_eq!(rows[0]["count"], 2);
        assert_eq!(rows[0]["user"], "alice");
        assert_eq!(rows[1]["user"], "bob");
        assert_eq!(rows[2]["team"], "dev");
        assert_eq!(rows[2]["count"], 0);
        assert!(rows[2].get("user").is_none());
    }
}
