mod grid;
mod guide_entries;
mod json;
mod key_value;
mod markdown;
pub(crate) mod shared;
mod table;
mod terminal;

use crate::core::output::OutputFormat;

use super::doc::{Block, Doc, JsonBlock};
use super::settings::ResolvedRenderSettings;

pub fn emit_doc(doc: &Doc, format: OutputFormat, settings: &ResolvedRenderSettings) -> String {
    match format {
        OutputFormat::Markdown => markdown::emit_doc(doc),
        OutputFormat::Json => {
            let Some(Block::Json(JsonBlock { text })) = doc.blocks.first() else {
                return String::new();
            };
            serde_json::from_str(text)
                .ok()
                .map(|value| json::emit_value(&value, settings))
                .unwrap_or_else(|| format!("{text}\n"))
        }
        _ => terminal::emit_doc(doc, settings),
    }
}
