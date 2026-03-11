use crate::core::row::Row;

pub(crate) struct RowBuilder(Row);

impl RowBuilder {
    pub(crate) fn new() -> Self {
        Self(Row::new())
    }

    pub(crate) fn insert<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
    {
        self.0.insert(key.into(), value.into());
        self
    }

    pub(crate) fn build(self) -> Row {
        self.0
    }
}

#[macro_export]
/// Builds a [`Row`](crate::core::row::Row) from literal key/value pairs.
///
/// This is the terse path for fixed row literals in command/render code.
///
/// # Examples
///
/// ```
/// use osp_cli::row;
/// use serde_json::json;
///
/// let row = row! {
///     "id" => 7,
///     "name" => "alice",
/// };
///
/// assert_eq!(row.get("id"), Some(&json!(7)));
/// assert_eq!(row.get("name"), Some(&json!("alice")));
/// ```
macro_rules! row {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut row = $crate::core::row::Row::new();
        $(row.insert(($key).into(), ($value).into());)*
        row
    }};
}
