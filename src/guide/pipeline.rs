//! Guide content enters the DSL as ordinary rows; section titles and layout
//! remain presentation metadata. No guide-specific verb execution lives here.
use super::{GuideEntry, GuideSectionKind, GuideView};
use crate::core::{output_model::rows_from_value, row::Row};
use serde_json::{Value, json};

fn entry_row(entry: &GuideEntry) -> Row {
    json!({"name": entry.name, "short_help": entry.short_help})
        .as_object()
        .unwrap()
        .clone()
}

fn text_row(text: &str) -> Row {
    Row::from_iter([("value".to_string(), Value::String(text.to_string()))])
}

impl GuideView {
    pub(crate) fn pipeline_document(
        document: &crate::core::output_model::OutputDocument,
    ) -> Option<Self> {
        if document.kind != crate::core::output_model::OutputDocumentKind::Guide {
            return None;
        }
        let view: Self = serde_json::from_value(document.value.clone()).ok()?;
        view.is_semantically_valid().then_some(view)
    }

    pub(crate) fn pipeline_rows(&self) -> Vec<Row> {
        let mut guide = self.pipeline_view();
        let mut rows = Vec::new();
        guide.visit_pipeline_rows(|row| {
            rows.push(row.clone());
            Some(0)
        });
        rows
    }

    pub(crate) fn select_pipeline_rows(&self, rows: &[Row]) -> Self {
        let mut guide = self.pipeline_view();
        let mut remaining: Vec<_> = rows.iter().enumerate().collect();
        guide.visit_pipeline_rows(|row| {
            let index = remaining
                .iter()
                .position(|(_, candidate)| *candidate == row)?;
            Some(remaining.remove(index).0)
        });
        guide.sections.retain(|section| {
            !section.entries.is_empty() || !section.paragraphs.is_empty() || section.data.is_some()
        });
        guide
    }

    fn pipeline_view(&self) -> Self {
        let mut view = self.clone();
        // Restoring a guide may mirror sections into canonical buckets. Each
        // authored entry must enter the row stream only once.
        for section in &self.sections {
            if !section.is_canonical_builtin_section() {
                continue;
            }
            match section.kind {
                GuideSectionKind::Usage => view.usage.clear(),
                GuideSectionKind::Commands => view.commands.clear(),
                GuideSectionKind::Arguments => view.arguments.clear(),
                GuideSectionKind::Options => view.options.clear(),
                GuideSectionKind::CommonInvocationOptions => view.common_invocation_options.clear(),
                GuideSectionKind::Notes => view.notes.clear(),
                _ => {}
            }
        }
        view
    }

    fn visit_pipeline_rows(&mut self, mut select: impl FnMut(&Row) -> Option<usize>) {
        fn retain_sorted<T>(items: &mut Vec<T>, mut select: impl FnMut(&T) -> Option<usize>) {
            let mut selected: Vec<_> = std::mem::take(items)
                .into_iter()
                .filter_map(|item| select(&item).map(|rank| (rank, item)))
                .collect();
            selected.sort_by_key(|(rank, _)| *rank);
            *items = selected.into_iter().map(|(_, item)| item).collect();
        }
        for texts in [
            &mut self.preamble,
            &mut self.usage,
            &mut self.notes,
            &mut self.epilogue,
        ] {
            retain_sorted(texts, |text| select(&text_row(text)));
        }
        for entries in [
            &mut self.commands,
            &mut self.arguments,
            &mut self.options,
            &mut self.common_invocation_options,
        ] {
            retain_sorted(entries, |entry| select(&entry_row(entry)));
        }
        for section in &mut self.sections {
            retain_sorted(&mut section.paragraphs, |text| select(&text_row(text)));
            retain_sorted(&mut section.entries, |entry| select(&entry_row(entry)));
            if let Some(data) = section.data.take() {
                let mut rows = rows_from_value(data);
                retain_sorted(&mut rows, |row| select(row));
                if !rows.is_empty() {
                    section.data =
                        Some(Value::Array(rows.into_iter().map(Value::Object).collect()));
                }
            }
        }
    }
}
