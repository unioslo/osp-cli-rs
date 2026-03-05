use osp_core::row::Row;
use serde_json::{Map, Value};

use crate::parse::path::{Selector, parse_path};

pub fn flatten_row(row: &Row) -> Row {
    let mut out = Map::new();
    for (key, value) in row {
        flatten_value(Some(key.as_str()), value, &mut out);
    }
    out
}

pub fn flatten_rows(rows: &[Row]) -> Vec<Row> {
    rows.iter().map(flatten_row).collect()
}

pub fn coalesce_flat_row(row: &Row) -> Row {
    let mut root = Value::Object(Map::new());
    for (key, value) in row {
        let Ok(path) = parse_path(key) else {
            continue;
        };
        let mut steps = Vec::new();
        for segment in path.segments {
            let Some(name) = segment.name else {
                steps.clear();
                break;
            };
            steps.push(Step::Key(name));
            for selector in segment.selectors {
                match selector {
                    Selector::Index(index) if index >= 0 => steps.push(Step::Index(index as usize)),
                    _ => {
                        steps.clear();
                        break;
                    }
                }
            }
        }
        if steps.is_empty() {
            continue;
        }
        insert_value(&mut root, &steps, value.clone());
    }

    match root {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

#[derive(Debug, Clone)]
enum Step {
    Key(String),
    Index(usize),
}

fn insert_value(root: &mut Value, steps: &[Step], value: Value) {
    if steps.is_empty() {
        *root = value;
        return;
    }

    let next_step = steps.get(1);
    match &steps[0] {
        Step::Key(key) => {
            ensure_object(root);
            if let Value::Object(map) = root {
                let entry = map.entry(key.clone()).or_insert(Value::Null);
                if steps.len() == 1 {
                    *entry = value;
                    return;
                }
                ensure_container(entry, next_step);
                insert_value(entry, &steps[1..], value);
            }
        }
        Step::Index(index) => {
            ensure_array(root);
            if let Value::Array(items) = root {
                if items.len() <= *index {
                    items.resize(*index + 1, Value::Null);
                }
                let entry = &mut items[*index];
                if steps.len() == 1 {
                    *entry = value;
                    return;
                }
                ensure_container(entry, next_step);
                insert_value(entry, &steps[1..], value);
            }
        }
    }
}

fn ensure_container(value: &mut Value, next_step: Option<&Step>) {
    match next_step {
        Some(Step::Key(_)) => ensure_object(value),
        Some(Step::Index(_)) => ensure_array(value),
        None => {}
    }
}

fn ensure_object(value: &mut Value) {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
}

fn ensure_array(value: &mut Value) {
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
}

fn flatten_value(prefix: Option<&str>, value: &Value, out: &mut Row) {
    match value {
        Value::Object(map) => {
            for (key, nested_value) in map {
                let next_prefix = match prefix {
                    Some(parent) => format!("{parent}.{key}"),
                    None => key.clone(),
                };
                flatten_value(Some(next_prefix.as_str()), nested_value, out);
            }
        }
        Value::Array(values) => {
            for (index, nested_value) in values.iter().enumerate() {
                let next_prefix = match prefix {
                    Some(parent) => format!("{parent}[{index}]"),
                    None => format!("[{index}]"),
                };
                flatten_value(Some(next_prefix.as_str()), nested_value, out);
            }
        }
        _ => {
            if let Some(key) = prefix {
                out.insert(key.to_string(), value.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{coalesce_flat_row, flatten_row};

    #[test]
    fn flattens_nested_objects_and_lists() {
        let row = json!({
            "uid": "oistes",
            "person": { "mail": "o@uio.no" },
            "members": ["a", "b"]
        })
        .as_object()
        .cloned()
        .expect("object");

        let flattened = flatten_row(&row);
        assert_eq!(
            flattened.get("uid").and_then(|v| v.as_str()),
            Some("oistes")
        );
        assert_eq!(
            flattened.get("person.mail").and_then(|v| v.as_str()),
            Some("o@uio.no")
        );
        assert_eq!(
            flattened.get("members[0]").and_then(|v| v.as_str()),
            Some("a")
        );
    }

    #[test]
    fn coalesces_flattened_row_back_to_nested_structure() {
        let row = json!({
            "id": 55753,
            "txts.id": 27994,
            "ipaddresses[0].id": 57171,
            "ipaddresses[1].id": 57172,
            "metadata.asset.id": 42
        })
        .as_object()
        .cloned()
        .expect("object");

        let coalesced = coalesce_flat_row(&row);
        assert_eq!(coalesced.get("id"), Some(&json!(55753)));
        assert_eq!(coalesced.get("txts"), Some(&json!({"id": 27994})));
        assert_eq!(
            coalesced.get("ipaddresses"),
            Some(&json!([{"id": 57171}, {"id": 57172}]))
        );
        assert_eq!(
            coalesced.get("metadata"),
            Some(&json!({"asset": {"id": 42}}))
        );
    }
}
