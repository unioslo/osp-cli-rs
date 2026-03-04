pub mod document;
mod formatter;
pub mod messages;
mod renderer;
pub mod style;
pub mod theme;

use std::io::IsTerminal;

use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_core::row::Row;

pub use document::{
    Document, JsonBlock, MregBlock, MregEntry, MregRow, MregValue, TableBlock, TableStyle,
    ValueBlock,
};

#[derive(Debug, Clone)]
pub struct RenderSettings {
    pub format: OutputFormat,
    pub mode: RenderMode,
    pub color: ColorMode,
    pub unicode: UnicodeMode,
    pub width: Option<usize>,
    pub theme_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    Plain,
    Rich,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRenderSettings {
    pub backend: RenderBackend,
    pub color: bool,
    pub unicode: bool,
    pub width: Option<usize>,
    pub theme_name: String,
}

impl RenderSettings {
    fn resolve_color_mode(&self) -> bool {
        match self.color {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => {
                let no_color = std::env::var("NO_COLOR").is_ok();
                !no_color && std::io::stdout().is_terminal()
            }
        }
    }

    fn resolve_unicode_mode(&self) -> bool {
        match self.unicode {
            UnicodeMode::Always => true,
            UnicodeMode::Never => false,
            UnicodeMode::Auto => {
                let term = std::env::var("TERM").unwrap_or_default();
                term != "dumb"
            }
        }
    }

    pub fn resolve_render_settings(&self) -> ResolvedRenderSettings {
        let backend = match self.mode {
            RenderMode::Plain => RenderBackend::Plain,
            RenderMode::Rich => RenderBackend::Rich,
            RenderMode::Auto => {
                if std::io::stdout().is_terminal() {
                    RenderBackend::Rich
                } else {
                    RenderBackend::Plain
                }
            }
        };

        match backend {
            // Plain mode is a strict no-color/no-unicode fallback.
            RenderBackend::Plain => ResolvedRenderSettings {
                backend,
                color: false,
                unicode: false,
                width: self.resolve_width(),
                theme_name: theme::normalize_theme_name(&self.theme_name),
            },
            RenderBackend::Rich => ResolvedRenderSettings {
                backend,
                color: self.resolve_color_mode(),
                unicode: self.resolve_unicode_mode(),
                width: self.resolve_width(),
                theme_name: theme::normalize_theme_name(&self.theme_name),
            },
        }
    }

    fn resolve_width(&self) -> Option<usize> {
        if let Some(width) = self.width {
            return (width > 0).then_some(width);
        }

        std::env::var("COLUMNS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|width| *width > 0)
    }
}

pub fn render_rows(rows: &[Row], settings: &RenderSettings) -> String {
    let document = formatter::build_document(rows, settings);
    let resolved = settings.resolve_render_settings();
    renderer::render_document(&document, resolved)
}

pub fn render_rows_for_copy(rows: &[Row], settings: &RenderSettings) -> String {
    let copy_settings = RenderSettings {
        format: settings.format,
        mode: RenderMode::Plain,
        color: ColorMode::Never,
        unicode: UnicodeMode::Never,
        width: settings.width,
        theme_name: settings.theme_name.clone(),
    };
    let document = formatter::build_document(rows, &copy_settings);
    let resolved = copy_settings.resolve_render_settings();
    renderer::render_document(&document, resolved)
}

#[cfg(test)]
mod tests {
    use super::{RenderBackend, RenderSettings, formatter, render_rows_for_copy};
    use crate::document::{Block, MregValue, TableStyle};
    use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use osp_core::row::Row;
    use serde_json::json;

    fn settings(format: OutputFormat) -> RenderSettings {
        RenderSettings {
            format,
            mode: RenderMode::Auto,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: None,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
        }
    }

    #[test]
    fn auto_selects_value_for_value_rows() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("value".to_string(), json!("hello"));
            row
        }];

        let document = formatter::build_document(&rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Value(_)));
    }

    #[test]
    fn auto_selects_mreg_for_single_non_value_row() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row
        }];

        let document = formatter::build_document(&rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Mreg(_)));
    }

    #[test]
    fn auto_selects_table_for_multi_row_result() {
        let rows = vec![
            {
                let mut row = Row::new();
                row.insert("uid".to_string(), json!("one"));
                row
            },
            {
                let mut row = Row::new();
                row.insert("uid".to_string(), json!("two"));
                row
            },
        ];

        let document = formatter::build_document(&rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Table(_)));
    }

    #[test]
    fn mreg_block_models_scalar_and_list_values() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row.insert("groups".to_string(), json!(["a", "b"]));
            row
        }];

        let document = formatter::build_document(&rows, &settings(OutputFormat::Mreg));
        let Block::Mreg(block) = &document.blocks[0] else {
            panic!("expected mreg block");
        };
        assert_eq!(block.rows.len(), 1);
        assert!(
            block.rows[0]
                .entries
                .iter()
                .any(|entry| matches!(entry.value, MregValue::Scalar(_)))
        );
        assert!(
            block.rows[0]
                .entries
                .iter()
                .any(|entry| matches!(entry.value, MregValue::List(_)))
        );
    }

    #[test]
    fn markdown_format_builds_markdown_table_block() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row
        }];

        let document = formatter::build_document(&rows, &settings(OutputFormat::Markdown));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(table.style, TableStyle::Markdown);
    }

    #[test]
    fn plain_mode_disables_color_and_unicode_even_when_forced() {
        let settings = RenderSettings {
            format: OutputFormat::Table,
            mode: RenderMode::Plain,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            width: None,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
        };

        let resolved = settings.resolve_render_settings();
        assert_eq!(resolved.backend, RenderBackend::Plain);
        assert!(!resolved.color);
        assert!(!resolved.unicode);
    }

    #[test]
    fn rich_mode_keeps_forced_color_and_unicode() {
        let settings = RenderSettings {
            format: OutputFormat::Table,
            mode: RenderMode::Rich,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            width: None,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
        };

        let resolved = settings.resolve_render_settings();
        assert_eq!(resolved.backend, RenderBackend::Rich);
        assert!(resolved.color);
        assert!(resolved.unicode);
    }

    #[test]
    fn copy_render_forces_plain_without_ansi_or_unicode_borders() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row.insert(
                "description".to_string(),
                json!("very long text that will be shown"),
            );
            row
        }];

        let settings = RenderSettings {
            format: OutputFormat::Table,
            mode: RenderMode::Rich,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            width: None,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
        };

        let rendered = render_rows_for_copy(&rows, &settings);
        assert!(!rendered.contains("\x1b["));
        assert!(!rendered.contains('┌'));
        assert!(rendered.contains('+'));
    }
}
