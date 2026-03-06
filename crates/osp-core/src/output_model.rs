use crate::row::Row;

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub groups: Row,
    pub aggregates: Row,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputMeta {
    pub key_index: Vec<String>,
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
        let mut key_index = Vec::new();
        for row in &rows {
            for key in row.keys() {
                if !key_index.contains(key) {
                    key_index.push(key.clone());
                }
            }
        }

        Self {
            items: OutputItems::Rows(rows),
            meta: OutputMeta {
                key_index,
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
