use crate::osp_core::row::Row;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlignment {
    #[default]
    Default,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub groups: Row,
    pub aggregates: Row,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputMeta {
    pub key_index: Vec<String>,
    pub column_align: Vec<ColumnAlignment>,
    pub wants_copy: bool,
    pub grouped: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OutputItems {
    Rows(Vec<Row>),
    Groups(Vec<Group>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputResult {
    pub items: OutputItems,
    pub meta: OutputMeta,
}

impl OutputResult {
    pub fn from_rows(rows: Vec<Row>) -> Self {
        let key_index = compute_key_index(&rows);
        Self {
            items: OutputItems::Rows(rows),
            meta: OutputMeta {
                key_index,
                column_align: Vec::new(),
                wants_copy: false,
                grouped: false,
            },
        }
    }

    pub fn as_rows(&self) -> Option<&[Row]> {
        match &self.items {
            OutputItems::Rows(rows) => Some(rows),
            OutputItems::Groups(_) => None,
        }
    }

    pub fn into_rows(self) -> Option<Vec<Row>> {
        match self.items {
            OutputItems::Rows(rows) => Some(rows),
            OutputItems::Groups(_) => None,
        }
    }
}

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
    use super::{Group, OutputItems, OutputMeta, OutputResult};
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
}
