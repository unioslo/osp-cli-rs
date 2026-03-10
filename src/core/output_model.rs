use crate::core::output::OutputFormat;
use crate::core::row::Row;
use serde_json::Value;
use std::collections::HashSet;

/// Alignment hint for a rendered output column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlignment {
    #[default]
    Default,
    Left,
    Center,
    Right,
}

/// Grouped output with grouping keys, aggregate values, and member rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub groups: Row,
    pub aggregates: Row,
    pub rows: Vec<Row>,
}

/// Rendering metadata attached to an [`OutputResult`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputMeta {
    pub key_index: Vec<String>,
    pub column_align: Vec<ColumnAlignment>,
    pub wants_copy: bool,
    pub grouped: bool,
    pub render_recommendation: Option<RenderRecommendation>,
}

/// Suggested render target for a command result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderRecommendation {
    Format(OutputFormat),
    Guide,
}

/// Optional semantic document attached to rendered output.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputDocument {
    Guide(Value),
}

/// Result payload as either flat rows or grouped rows.
#[derive(Clone, Debug, PartialEq)]
pub enum OutputItems {
    Rows(Vec<Row>),
    Groups(Vec<Group>),
}

/// Structured command output plus rendering metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputResult {
    pub items: OutputItems,
    pub document: Option<OutputDocument>,
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

#[cfg(test)]
mod tests {
    use super::{Group, OutputDocument, OutputItems, OutputMeta, OutputResult};
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
        let output = OutputResult::from_rows(Vec::new())
            .with_document(OutputDocument::Guide(json!({"usage": ["osp"]})));

        assert!(matches!(
            output.document,
            Some(OutputDocument::Guide(Value::Object(_)))
        ));
    }
}
