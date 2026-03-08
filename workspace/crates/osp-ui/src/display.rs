use serde_json::{Map, Value};

const OBJECT_KEY_PREVIEW: usize = 3;

pub(crate) fn value_to_display(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string().to_ascii_lowercase(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(values) => values
            .iter()
            .map(value_to_display)
            .collect::<Vec<String>>()
            .join(", "),
        Value::Object(map) => summarize_object(map, OBJECT_KEY_PREVIEW),
    }
}

fn summarize_object(map: &Map<String, Value>, max_keys: usize) -> String {
    if map.is_empty() {
        return "{}".to_string();
    }

    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();

    let mut out = String::from("{");
    let mut wrote = false;
    for key in keys.iter().take(max_keys.max(1)) {
        if wrote {
            out.push_str(", ");
        }
        out.push_str(key);
        wrote = true;
    }
    if map.len() > max_keys.max(1) {
        if wrote {
            out.push_str(", ");
        }
        out.push_str("...");
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::value_to_display;
    use serde_json::json;

    #[test]
    fn empty_objects_render_as_empty_braces() {
        assert_eq!(value_to_display(&json!({})), "{}");
    }

    #[test]
    fn object_preview_adds_ellipsis_when_keys_exceed_preview_limit() {
        assert_eq!(
            value_to_display(&json!({
                "uid": "alice",
                "mail": "alice@uio.no",
                "title": "Engineer",
                "group": "ops"
            })),
            "{group, mail, title, ...}"
        );
    }
}
