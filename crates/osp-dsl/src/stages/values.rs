use anyhow::Result;
use osp_core::row::Row;
use serde_json::{Map, Value};

use super::common::parse_terms;

pub fn apply(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let keys = parse_terms(spec);
    let mut out: Vec<Row> = Vec::new();

    for row in rows {
        if keys.is_empty() {
            for value in row.values() {
                emit_value_rows(&mut out, value);
            }
            continue;
        }

        for key in &keys {
            if let Some(value) = row.get(key) {
                emit_value_rows(&mut out, value);
            }
        }
    }

    Ok(out)
}

fn emit_value_rows(out: &mut Vec<Row>, value: &Value) {
    match value {
        Value::Array(values) => {
            for item in values {
                let mut row = Map::new();
                row.insert("value".to_string(), item.clone());
                out.push(row);
            }
        }
        _ => {
            let mut row = Map::new();
            row.insert("value".to_string(), value.clone());
            out.push(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::apply;

    #[test]
    fn explodes_array_values() {
        let rows = vec![
            json!({"members": ["a", "b"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "members").expect("values should work");
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn emits_requested_scalar_values_and_ignores_missing_keys() {
        let rows = vec![
            json!({"uid": "oistes", "mail": "oistes@example.org"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "uid missing").expect("values should work");
        assert_eq!(output.len(), 1);
        assert_eq!(
            output[0].get("value").and_then(|value| value.as_str()),
            Some("oistes")
        );
    }

    #[test]
    fn empty_spec_emits_all_scalar_and_array_values_in_order() {
        let rows = vec![
            json!({"uid": "oistes", "members": ["a", "b"], "active": true})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "").expect("empty values stage should enumerate all fields");
        let mut values = output
            .iter()
            .map(|row| {
                row.get("value")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
                    .to_string()
            })
            .collect::<Vec<_>>();
        values.sort();

        assert_eq!(values, vec!["\"a\"", "\"b\"", "\"oistes\"", "true"]);
    }
}
