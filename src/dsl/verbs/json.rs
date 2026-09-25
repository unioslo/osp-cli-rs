//! Structural emptiness used while expanding nested row values.
use crate::dsl::eval::resolve::is_sparse_hole;
use serde_json::Value;

pub(crate) fn is_structurally_empty(value: &Value) -> bool {
    match value {
        value if is_sparse_hole(value) => true,
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}
