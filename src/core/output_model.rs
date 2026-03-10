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
    pub fn new(kind: OutputDocumentKind, value: Value) -> Self {
        Self { kind, value }
    }

    /// Reprojects the payload over generic output items while keeping identity.
    ///
    /// The canonical DSL uses this to preserve payload identity without branching on
    /// concrete semantic types inside the executor. Whether the projected JSON
    /// still restores into the original payload kind is decided later by the
    /// payload codec, not by the pipeline engine itself.
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
    pub fn with_document(mut self, document: OutputDocument) -> Self {
        self.document = Some(document);
        self
    }

    /// Returns the underlying rows when the result is not grouped.
    pub fn as_rows(&self) -> Option<&[Row]> {
        match &self.items {
            OutputItems::Rows(rows) => Some(rows),
            OutputItems::Groups(_) => None,
        }
    }

    /// Consumes the result and returns its rows when the payload is row-based.
    pub fn into_rows(self) -> Option<Vec<Row>> {
        match self.items {
            OutputItems::Rows(rows) => Some(rows),
            OutputItems::Groups(_) => None,
        }
    }
}

/// Computes the stable first-seen column order across all rows.
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
    fn from_rows_keeps_first_seen_key_order() {
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

        let output = OutputResult::from_rows(rows);
        assert_eq!(output.meta.key_index, vec!["uid", "cn", "mail", "title"]);
    }

    #[test]
    fn grouped_output_does_not_expose_rows_views() {
        let output = OutputResult {
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

        assert_eq!(output.as_rows(), None);
        assert_eq!(output.into_rows(), None);
    }

    #[test]
    fn row_output_exposes_rows_views() {
        let rows = vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let output = OutputResult::from_rows(rows.clone());

        assert_eq!(output.as_rows(), Some(rows.as_slice()));
        assert_eq!(output.into_rows(), Some(rows));
    }

    #[test]
    fn with_document_attaches_semantic_payload_unit() {
        let output = OutputResult::from_rows(Vec::new()).with_document(OutputDocument::new(
            OutputDocumentKind::Guide,
            json!({"usage": ["osp"]}),
        ));

        assert!(matches!(
            output.document,
            Some(OutputDocument {
                kind: OutputDocumentKind::Guide,
                value: Value::Object(_),
            })
        ));
    }

    #[test]
    fn output_items_to_value_projects_rows_and_groups_unit() {
        let rows = OutputItems::Rows(vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
        ]);
        assert!(matches!(output_items_to_value(&rows), Value::Object(_)));

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
        assert!(matches!(output_items_to_value(&groups), Value::Array(_)));
    }

    #[test]
    fn output_items_from_value_round_trips_rows_and_groups_unit() {
        let rows = OutputItems::Rows(vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
        ]);
        assert_eq!(output_items_from_value(output_items_to_value(&rows)), rows);

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
        assert_eq!(
            output_items_from_value(output_items_to_value(&groups)),
            groups
        );
    }
}
