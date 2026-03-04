use osp_core::output::OutputFormat;
use osp_core::row::Row;

use crate::RenderSettings;
use crate::document::{Block, Document, JsonBlock, TableStyle};

mod common;
pub mod message;
mod mreg;
mod table;
mod value;

pub use message::{MessageContent, MessageFormatter, MessageKind, MessageOptions, MessageRules};

pub fn build_document(rows: &[Row], settings: &RenderSettings) -> Document {
    let format = resolve_format(rows, settings.format);
    let block = match format {
        OutputFormat::Json => Block::Json(build_json_block(rows)),
        OutputFormat::Table => Block::Table(table::build_table_block(rows, TableStyle::Grid)),
        OutputFormat::Markdown => {
            Block::Table(table::build_table_block(rows, TableStyle::Markdown))
        }
        OutputFormat::Mreg => Block::Mreg(mreg::build_mreg_block(rows)),
        OutputFormat::Value => Block::Value(value::build_value_block(rows)),
        OutputFormat::Auto => unreachable!("auto format is resolved above"),
    };

    Document {
        blocks: vec![block],
    }
}

pub fn resolve_format(rows: &[Row], format: OutputFormat) -> OutputFormat {
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
        payload: serde_json::Value::Array(
            rows.iter()
                .cloned()
                .map(serde_json::Value::Object)
                .collect(),
        ),
    }
}
