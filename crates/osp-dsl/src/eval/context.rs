use std::collections::HashSet;

use osp_core::row::Row;

#[derive(Debug, Clone, Default)]
pub struct RowContext {
    key_index: Vec<String>,
}

impl RowContext {
    pub fn from_rows(rows: &[Row]) -> Self {
        let mut seen = HashSet::new();
        let mut key_index = Vec::new();

        for row in rows {
            let mut keys = row.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if seen.insert(key.clone()) {
                    key_index.push(key);
                }
            }
        }

        Self { key_index }
    }

    pub fn key_index(&self) -> &[String] {
        &self.key_index
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RowContext;

    #[test]
    fn keeps_first_seen_key_order() {
        let rows = vec![
            json!({"uid": "oistes", "cn": "Oistein"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"mail": "o@uio.no", "uid": "oistes"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let context = RowContext::from_rows(&rows);
        assert_eq!(context.key_index(), &["cn", "uid", "mail"]);
    }
}
