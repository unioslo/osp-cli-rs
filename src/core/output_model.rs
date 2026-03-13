//! Structured output payload model shared across commands, DSL stages, and UI.
//!
//! This module exists to keep command results in a small canonical shape while
//! they move between execution, transformation, and rendering layers.
//!
//! High-level flow:
//!
//! - commands produce [`crate::core::output_model::OutputResult`]
//! - the DSL transforms its [`crate::core::output_model::OutputItems`] and
//!   optional semantic document
//! - the UI later lowers the result into rendered documents and terminal text
//!
//! Contract:
//!
//! - this module describes data shape, not rendering policy
//! - semantic sidecar documents should stay canonical here instead of leaking
//!   format-specific assumptions into the DSL or UI

use crate::core::output::OutputFormat;
use crate::core::row::Row;
use serde_json::Value;
use std::collections::HashSet;

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

/// Rendering metadata attached to an [`OutputResult`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputMeta {
    /// Stable first-seen column order for row rendering.
    pub key_index: Vec<String>,
    /// Per-column alignment hints.
    pub column_align: Vec<ColumnAlignment>,
    /// Whether the result should be easy to copy as plain text.
    pub wants_copy: bool,
    /// Whether the payload represents grouped data.
    pub grouped: bool,
    /// Preferred renderer for this result, when known.
    pub render_recommendation: Option<RenderRecommendation>,
}

/// Suggested render target for a command result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderRecommendation {
    /// Render using the specified output format.
    Format(OutputFormat),
    /// Render as structured guide/help content.
    Guide,
}

/// Stable identity for a semantic payload carried through the output pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputDocumentKind {
    /// Structured guide/help/intro payload.
    Guide,
}

/// Optional semantic document attached to rendered output.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputDocument {
    /// Semantic payload identity.
    pub kind: OutputDocumentKind,
    /// Canonical JSON substrate for the payload.
    pub value: Value,
}

impl OutputDocument {
    /// Builds a semantic payload from its identity and canonical JSON value.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output_model::{OutputDocument, OutputDocumentKind};
    /// use serde_json::json;
    ///
    /// let document = OutputDocument::new(OutputDocumentKind::Guide, json!({"title": "Help"}));
    ///
    /// assert_eq!(document.kind, OutputDocumentKind::Guide);
    /// assert_eq!(document.value["title"], "Help");
    /// ```
    pub fn new(kind: OutputDocumentKind, value: Value) -> Self {
        Self { kind, value }
    }

    /// Reprojects the payload over generic output items while keeping identity.
    ///
    /// The canonical DSL uses this to preserve payload identity without branching on
    /// concrete semantic types inside the executor. Whether the projected JSON
    /// still restores into the original payload kind is decided later by the
    /// payload codec, not by the pipeline engine itself.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output_model::{OutputDocument, OutputDocumentKind, OutputItems};
    /// use osp_cli::row;
    /// use serde_json::json;
    ///
    /// let document = OutputDocument::new(OutputDocumentKind::Guide, json!({"usage": ["osp"]}));
    /// let projected = document.project_over_items(&OutputItems::Rows(vec![
    ///     row! { "uid" => "alice" },
    /// ]));
    ///
    /// assert_eq!(projected.kind, OutputDocumentKind::Guide);
    /// assert_eq!(projected.value["uid"], "alice");
    /// ```
    pub fn project_over_items(&self, items: &OutputItems) -> Self {
        Self {
            kind: self.kind,
            value: output_items_to_value(items),
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
    /// Optional semantic sidecar document.
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
    /// use osp_cli::core::output_model::OutputResult;
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
                column_align: Vec::new(),
                wants_copy: false,
                grouped: false,
                render_recommendation: None,
            },
        }
    }

    /// Attaches a semantic document to the result and returns the updated value.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output_model::{OutputDocument, OutputDocumentKind, OutputResult};
    /// use serde_json::json;
    ///
    /// let output = OutputResult::from_rows(Vec::new()).with_document(OutputDocument::new(
    ///     OutputDocumentKind::Guide,
    ///     json!({"title": "Help"}),
    /// ));
    ///
    /// assert!(output.document.is_some());
    /// ```
    #[must_use]
    pub fn with_document(mut self, document: OutputDocument) -> Self {
        self.document = Some(document);
        self
    }

    /// Returns the underlying rows when the result is not grouped.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output_model::OutputResult;
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
    /// use osp_cli::core::output_model::OutputResult;
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

/// Computes the stable first-seen column order across all rows.
///
/// # Examples
///
/// ```
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
pub fn compute_key_index(rows: &[Row]) -> Vec<String> {
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
/// ```
/// use osp_cli::core::output_model::{OutputItems, output_items_to_value};
/// use osp_cli::row;
///
/// let value = output_items_to_value(&OutputItems::Rows(vec![row! { "uid" => "alice" }]));
///
/// assert_eq!(value["uid"], "alice");
/// ```
pub fn output_items_to_value(items: &OutputItems) -> Value {
    match items {
        OutputItems::Rows(rows) if rows.len() == 1 => rows
            .first()
            .cloned()
            .map(Value::Object)
            .unwrap_or_else(|| Value::Array(Vec::new())),
        OutputItems::Rows(rows) => {
            Value::Array(rows.iter().cloned().map(Value::Object).collect::<Vec<_>>())
        }
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

/// Projects a canonical JSON value back into generic output items.
///
/// This is the inverse substrate bridge used by the canonical DSL: semantic payloads stay
/// canonical as JSON, while the existing stage logic continues to operate over
/// rows and groups derived from that JSON.
///
/// # Examples
///
/// ```
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
pub fn output_items_from_value(value: Value) -> OutputItems {
    match value {
        Value::Array(items) => {
            if let Some(groups) = groups_from_values(&items) {
                OutputItems::Groups(groups)
            } else if items.iter().all(|item| matches!(item, Value::Object(_))) {
                OutputItems::Rows(
                    items
                        .into_iter()
                        .filter_map(|item| item.as_object().cloned())
                        .collect::<Vec<_>>(),
                )
            } else {
                OutputItems::Rows(vec![row_with_value(Value::Array(items))])
            }
        }
        Value::Object(map) => OutputItems::Rows(vec![map]),
        scalar => OutputItems::Rows(vec![row_with_value(scalar)]),
    }
}

fn row_with_value(value: Value) -> Row {
    let mut row = Row::new();
    row.insert("value".to_string(), value);
    row
}

fn groups_from_values(values: &[Value]) -> Option<Vec<Group>> {
    values.iter().map(group_from_value).collect()
}

fn group_from_value(value: &Value) -> Option<Group> {
    let Value::Object(map) = value else {
        return None;
    };
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
mod tests {
    use super::{
        Group, OutputDocument, OutputDocumentKind, OutputItems, OutputMeta, OutputResult,
        output_items_from_value, output_items_to_value,
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

        let document_output = OutputResult::from_rows(Vec::new()).with_document(
            OutputDocument::new(OutputDocumentKind::Guide, json!({"usage": ["osp"]})),
        );

        assert!(matches!(
            document_output.document,
            Some(OutputDocument {
                kind: OutputDocumentKind::Guide,
                value: Value::Object(_),
            })
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
}
