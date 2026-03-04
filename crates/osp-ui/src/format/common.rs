use serde_json::Value;

pub fn value_to_display(value: &Value) -> String {
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
        Value::Object(_) => value.to_string(),
    }
}
