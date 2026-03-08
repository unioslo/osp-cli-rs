use osp_core::row::Row;

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
macro_rules! row {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut builder = $crate::rows::row::RowBuilder::new();
        $(builder.insert($key, $value);)*
        builder.build()
    }};
}

// Style: prefer `row!{...}` for short, unconditional rows; use `RowBuilder` when
// values are conditional or built up across multiple branches.
// Example: let row = row! { "id" => 1, "name" => "alice" };
