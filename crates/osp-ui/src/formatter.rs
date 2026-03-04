use std::collections::BTreeSet;

use osp_core::output::OutputFormat;
use osp_core::row::Row;
use serde_json::Value;

use crate::RenderSettings;
use crate::document::{
    Block, Document, JsonBlock, MregBlock, MregEntry, MregRow, MregValue, TableBlock, TableStyle,
    ValueBlock,
};

pub fn build_document(rows: &[Row], settings: &RenderSettings) -> Document {
    let format = resolve_format(rows, settings.format);
    let block = match format {
        OutputFormat::Json => Block::Json(build_json_block(rows)),
        OutputFormat::Table => Block::Table(build_table_block(rows, TableStyle::Grid)),
        OutputFormat::Markdown => Block::Table(build_table_block(rows, TableStyle::Markdown)),
        OutputFormat::Mreg => Block::Mreg(build_mreg_block(rows)),
        OutputFormat::Value => Block::Value(build_value_block(rows)),
        OutputFormat::Auto => unreachable!("auto format is resolved above"),
    };

    Document {
        blocks: vec![block],
    }
}

fn resolve_format(rows: &[Row], format: OutputFormat) -> OutputFormat {
    if !matches!(format, OutputFormat::Auto) {
        return format;
    }

    if rows
        .iter()
        .all(|row| row.len() == 1 && row.contains_key("value"))
    {
        OutputFormat::Value
    } else if rows.len() <= 1 {
        OutputFormat::Mreg
    } else {
        OutputFormat::Table
    }
}

fn build_json_block(rows: &[Row]) -> JsonBlock {
    JsonBlock {
        payload: Value::Array(rows.iter().cloned().map(Value::Object).collect()),
    }
}

fn build_value_block(rows: &[Row]) -> ValueBlock {
    ValueBlock {
        values: rows
            .iter()
            .filter_map(|row| row.get("value"))
            .map(value_to_display)
            .collect(),
    }
}

fn build_mreg_block(rows: &[Row]) -> MregBlock {
    let mut block_rows = Vec::new();

    for row in rows {
        let mut keys = row.keys().cloned().collect::<Vec<String>>();
        keys.sort();

        let entries = keys
            .into_iter()
            .filter_map(|key| row.get(&key).map(|value| (key, value)))
            .map(|(key, value)| MregEntry {
                key,
                value: match value {
                    Value::Array(items) if !items.is_empty() => {
                        MregValue::List(items.iter().map(value_to_display).collect())
                    }
                    _ => MregValue::Scalar(value_to_display(value)),
                },
            })
            .collect::<Vec<MregEntry>>();

        block_rows.push(MregRow { entries });
    }

    MregBlock { rows: block_rows }
}

fn build_table_block(rows: &[Row], style: TableStyle) -> TableBlock {
    let headers = collect_headers(rows);
    let rendered_rows = rows
        .iter()
        .map(|row| {
            headers
                .iter()
                .map(|key| row.get(key).map(value_to_display).unwrap_or_default())
                .collect::<Vec<String>>()
        })
        .collect::<Vec<Vec<String>>>();

    TableBlock {
        style,
        headers,
        rows: rendered_rows,
    }
}

fn collect_headers(rows: &[Row]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for row in rows {
        for key in row.keys() {
            set.insert(key.clone());
        }
    }
    set.into_iter().collect()
}

fn value_to_display(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
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
