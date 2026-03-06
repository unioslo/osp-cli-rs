pub mod clipboard;
mod display;
pub mod document;
pub mod format;
pub mod inline;
pub mod interactive;
mod layout;
pub mod messages;
mod renderer;
pub mod style;
pub mod theme;

use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_core::output_model::{OutputItems, OutputResult};
use osp_core::row::Row;

pub use document::{
    CodeBlock, Document, JsonBlock, LineBlock, LinePart, MregBlock, MregEntry, MregRow, MregValue,
    PanelBlock, PanelRules, TableAlign, TableBlock, TableStyle, ValueBlock,
};
pub use inline::{line_from_inline, parts_from_inline, render_inline};
pub use interactive::{Interactive, InteractiveResult, InteractiveRuntime, Spinner};
pub use style::StyleOverrides;
use theme::ThemeDefinition;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderRuntime {
    pub stdout_is_tty: bool,
    pub terminal: Option<String>,
    pub no_color: bool,
    pub width: Option<usize>,
    pub locale_utf8: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct RenderSettings {
    pub format: OutputFormat,
    pub mode: RenderMode,
    pub color: ColorMode,
    pub unicode: UnicodeMode,
    pub width: Option<usize>,
    pub margin: usize,
    pub indent_size: usize,
    pub short_list_max: usize,
    pub medium_list_max: usize,
    pub grid_padding: usize,
    pub grid_columns: Option<usize>,
    pub column_weight: usize,
    pub table_overflow: TableOverflow,
    pub mreg_stack_min_col_width: usize,
    pub mreg_stack_overflow_ratio: usize,
    pub theme_name: String,
    pub theme: Option<ThemeDefinition>,
    pub style_overrides: StyleOverrides,
    pub runtime: RenderRuntime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    Plain,
    Rich,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableOverflow {
    None,
    Clip,
    Ellipsis,
    Wrap,
}

impl TableOverflow {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "visible" => Some(Self::None),
            "clip" | "hidden" | "crop" => Some(Self::Clip),
            "ellipsis" | "truncate" => Some(Self::Ellipsis),
            "wrap" | "wrapped" => Some(Self::Wrap),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRenderSettings {
    pub backend: RenderBackend,
    pub color: bool,
    pub unicode: bool,
    pub width: Option<usize>,
    pub margin: usize,
    pub indent_size: usize,
    pub short_list_max: usize,
    pub medium_list_max: usize,
    pub grid_padding: usize,
    pub grid_columns: Option<usize>,
    pub column_weight: usize,
    pub table_overflow: TableOverflow,
    pub theme_name: String,
    pub theme: ThemeDefinition,
    pub style_overrides: StyleOverrides,
}

impl RenderSettings {
    fn resolve_color_mode(&self) -> bool {
        match self.color {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => !self.runtime.no_color && self.runtime.stdout_is_tty,
        }
    }

    fn resolve_unicode_mode(&self) -> bool {
        match self.unicode {
            UnicodeMode::Always => true,
            UnicodeMode::Never => false,
            UnicodeMode::Auto => {
                if !self.runtime.stdout_is_tty {
                    return false;
                }
                if matches!(self.runtime.terminal.as_deref(), Some("dumb")) {
                    return false;
                }
                match self.runtime.locale_utf8 {
                    Some(true) => true,
                    Some(false) => false,
                    None => true,
                }
            }
        }
    }

    pub fn resolve_render_settings(&self) -> ResolvedRenderSettings {
        let backend = match self.mode {
            RenderMode::Plain => RenderBackend::Plain,
            RenderMode::Rich => RenderBackend::Rich,
            RenderMode::Auto => {
                if !self.runtime.stdout_is_tty
                    || matches!(self.runtime.terminal.as_deref(), Some("dumb"))
                {
                    RenderBackend::Plain
                } else {
                    RenderBackend::Rich
                }
            }
        };

        let theme = self
            .theme
            .clone()
            .unwrap_or_else(|| theme::resolve_theme(&self.theme_name));
        let theme_name = theme::normalize_theme_name(&theme.id);

        match backend {
            // Plain mode is a strict no-color/no-unicode fallback.
            RenderBackend::Plain => ResolvedRenderSettings {
                backend,
                color: false,
                unicode: false,
                width: self.resolve_width(),
                margin: self.margin,
                indent_size: self.indent_size.max(1),
                short_list_max: self.short_list_max.max(1),
                medium_list_max: self.medium_list_max.max(self.short_list_max.max(1) + 1),
                grid_padding: self.grid_padding.max(1),
                grid_columns: self.grid_columns.filter(|value| *value > 0),
                column_weight: self.column_weight.max(1),
                table_overflow: self.table_overflow,
                theme_name,
                theme: theme.clone(),
                style_overrides: self.style_overrides.clone(),
            },
            RenderBackend::Rich => ResolvedRenderSettings {
                backend,
                color: self.resolve_color_mode(),
                unicode: self.resolve_unicode_mode(),
                width: self.resolve_width(),
                margin: self.margin,
                indent_size: self.indent_size.max(1),
                short_list_max: self.short_list_max.max(1),
                medium_list_max: self.medium_list_max.max(self.short_list_max.max(1) + 1),
                grid_padding: self.grid_padding.max(1),
                grid_columns: self.grid_columns.filter(|value| *value > 0),
                column_weight: self.column_weight.max(1),
                table_overflow: self.table_overflow,
                theme_name,
                theme,
                style_overrides: self.style_overrides.clone(),
            },
        }
    }

    fn resolve_width(&self) -> Option<usize> {
        if let Some(width) = self.width {
            return (width > 0).then_some(width);
        }
        self.runtime.width.filter(|width| *width > 0)
    }

    fn plain_copy_settings(&self) -> Self {
        Self {
            format: self.format,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: self.width,
            margin: self.margin,
            indent_size: self.indent_size,
            short_list_max: self.short_list_max,
            medium_list_max: self.medium_list_max,
            grid_padding: self.grid_padding,
            grid_columns: self.grid_columns,
            column_weight: self.column_weight,
            table_overflow: self.table_overflow,
            mreg_stack_min_col_width: self.mreg_stack_min_col_width,
            mreg_stack_overflow_ratio: self.mreg_stack_overflow_ratio,
            theme_name: self.theme_name.clone(),
            theme: self.theme.clone(),
            style_overrides: self.style_overrides.clone(),
            runtime: self.runtime.clone(),
        }
    }
}

pub fn render_rows(rows: &[Row], settings: &RenderSettings) -> String {
    render_output(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            meta: Default::default(),
        },
        settings,
    )
}

pub fn render_output(output: &OutputResult, settings: &RenderSettings) -> String {
    let document = format::build_document_from_output(output, settings);
    let resolved = settings.resolve_render_settings();
    renderer::render_document(&document, resolved)
}

pub fn render_rows_for_copy(rows: &[Row], settings: &RenderSettings) -> String {
    render_output_for_copy(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            meta: Default::default(),
        },
        settings,
    )
}

pub fn render_output_for_copy(output: &OutputResult, settings: &RenderSettings) -> String {
    let copy_settings = settings.plain_copy_settings();
    let document = format::build_document_from_output(output, &copy_settings);
    render_document_for_copy(&document, &copy_settings)
}

pub fn render_document_for_copy(document: &Document, settings: &RenderSettings) -> String {
    let copy_settings = settings.plain_copy_settings();
    let resolved = copy_settings.resolve_render_settings();
    renderer::render_document(document, resolved)
}

pub fn copy_rows_to_clipboard(
    rows: &[Row],
    settings: &RenderSettings,
    clipboard: &clipboard::ClipboardService,
) -> Result<(), clipboard::ClipboardError> {
    copy_output_to_clipboard(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            meta: Default::default(),
        },
        settings,
        clipboard,
    )
}

pub fn copy_output_to_clipboard(
    output: &OutputResult,
    settings: &RenderSettings,
    clipboard: &clipboard::ClipboardService,
) -> Result<(), clipboard::ClipboardError> {
    let copy_settings = settings.plain_copy_settings();
    let document = format::build_document_from_output(output, &copy_settings);
    clipboard.copy_document(&document, &copy_settings)
}

#[cfg(test)]
mod tests {
    use super::{RenderBackend, RenderRuntime, RenderSettings, format, render_rows_for_copy};
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
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: crate::style::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        }
    }

    #[test]
    fn auto_selects_value_for_value_rows() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("value".to_string(), json!("hello"));
            row
        }];

        let document = format::build_document(&rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Value(_)));
    }

    #[test]
    fn auto_selects_mreg_for_single_non_value_row() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row
        }];

        let document = format::build_document(&rows, &settings(OutputFormat::Auto));
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

        let document = format::build_document(&rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Table(_)));
    }

    #[test]
    fn mreg_block_models_scalar_and_vertical_list_values() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row.insert("groups".to_string(), json!(["a", "b"]));
            row
        }];

        let document = format::build_document(&rows, &settings(OutputFormat::Mreg));
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
                .any(|entry| matches!(entry.value, MregValue::VerticalList(_)))
        );
    }

    #[test]
    fn markdown_format_builds_markdown_table_block() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row
        }];

        let document = format::build_document(&rows, &settings(OutputFormat::Markdown));
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
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: crate::style::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
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
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: crate::style::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
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
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: crate::theme::DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: crate::style::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        };

        let rendered = render_rows_for_copy(&rows, &settings);
        assert!(!rendered.contains("\x1b["));
        assert!(!rendered.contains('┌'));
        assert!(rendered.contains('+'));
    }
}
