//! Internal lowering from caller-facing [`crate::ui::RenderSettings`] into
//! semantic render plans.
//!
//! The UI surface intentionally exposes a broad configuration object because
//! callers need to express product intent in one place. The formatter and
//! renderer layers should not consume that broad shape directly. This module is
//! the narrowing seam: it resolves runtime-aware rendering facts and the
//! guide/MREG-specific lowering knobs once, then downstream code consumes the
//! smaller resolved plan.

use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::output_model::OutputResult;
use crate::ui::chrome::{RuledSectionPolicy, SectionFrameStyle};
use crate::ui::theme;
use crate::ui::{
    HelpChromeSettings, RenderBackend, RenderSettings, StyleOverrides, TableBorderStyle,
    TableOverflow, ThemeDefinition,
};

/// Fully resolved rendering settings used by the document renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRenderSettings {
    /// Concrete renderer backend selected for this render pass.
    pub backend: RenderBackend,
    /// Whether ANSI styling is enabled.
    pub color: bool,
    /// Whether Unicode rendering is enabled.
    pub unicode: bool,
    /// Effective width constraint, if any.
    pub width: Option<usize>,
    /// Effective left margin.
    pub margin: usize,
    /// Effective indentation width.
    pub indent_size: usize,
    /// Effective short-list threshold.
    pub short_list_max: usize,
    /// Effective medium-list threshold.
    pub medium_list_max: usize,
    /// Effective grid padding.
    pub grid_padding: usize,
    /// Effective grid column override.
    pub grid_columns: Option<usize>,
    /// Effective adaptive grid weight.
    pub column_weight: usize,
    /// Effective table overflow policy.
    pub table_overflow: TableOverflow,
    /// Effective general table border style.
    pub table_border: TableBorderStyle,
    /// Effective help-table border style.
    pub help_table_border: TableBorderStyle,
    /// Effective theme name.
    pub theme_name: String,
    /// Effective resolved theme.
    pub theme: ThemeDefinition,
    /// Effective style overrides layered over the theme.
    pub style_overrides: StyleOverrides,
    /// Effective section frame style.
    pub chrome_frame: SectionFrameStyle,
}

/// Internal semantic render plan for one output payload.
///
/// This is intentionally broader than [`ResolvedRenderSettings`]: once the UI
/// has planned a render, downstream formatters should not re-read raw
/// [`RenderSettings`] to rediscover guide, MREG, or output-format choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedRenderPlan {
    /// Final output format selected for this payload.
    pub(crate) format: OutputFormat,
    /// Terminal-aware rendering settings used by the renderer.
    pub(crate) render: ResolvedRenderSettings,
    /// Guide/help lowering settings derived for this payload.
    pub(crate) guide: ResolvedGuideRenderSettings,
    /// MREG lowering settings derived for this payload.
    pub(crate) mreg: ResolvedMregBuildSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedHelpChromeSettings {
    pub(crate) table_border: TableBorderStyle,
    pub(crate) entry_indent: Option<usize>,
    pub(crate) entry_gap: Option<usize>,
    pub(crate) section_spacing: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedGuideRenderSettings {
    pub(crate) frame_style: SectionFrameStyle,
    pub(crate) ruled_section_policy: RuledSectionPolicy,
    pub(crate) help_chrome: ResolvedHelpChromeSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedMregBuildSettings {
    pub(crate) short_list_max: usize,
    pub(crate) medium_list_max: usize,
    pub(crate) indent_size: usize,
    pub(crate) stack_min_col_width: usize,
    pub(crate) stack_overflow_ratio: usize,
}

impl RenderSettings {
    pub(crate) fn resolve_guide_render_settings(&self) -> ResolvedGuideRenderSettings {
        ResolvedGuideRenderSettings {
            frame_style: self.chrome_frame,
            ruled_section_policy: self.ruled_section_policy,
            help_chrome: self.help_chrome.resolve(self.table_border),
        }
    }

    pub(crate) fn resolve_mreg_build_settings(&self) -> ResolvedMregBuildSettings {
        ResolvedMregBuildSettings {
            short_list_max: self.short_list_max.max(1),
            medium_list_max: self.medium_list_max.max(self.short_list_max.max(1) + 1),
            indent_size: self.indent_size.max(1),
            stack_min_col_width: self.mreg_stack_min_col_width.max(1),
            stack_overflow_ratio: self.mreg_stack_overflow_ratio.max(100),
        }
    }

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

    /// Resolves terminal-aware rendering settings from the configured
    /// preferences.
    ///
    /// Plain mode is a strict fallback: once selected, the resolved settings
    /// will not emit ANSI color or Unicode box-drawing even if the runtime can.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    /// use osp_cli::ui::{RenderBackend, RenderSettings};
    ///
    /// let mut settings = RenderSettings::test_plain(OutputFormat::Json);
    /// settings.mode = RenderMode::Auto;
    /// settings.color = ColorMode::Always;
    /// settings.unicode = UnicodeMode::Always;
    ///
    /// let resolved = settings.resolve_render_settings();
    ///
    /// assert_eq!(resolved.backend, RenderBackend::Rich);
    /// assert!(resolved.color);
    /// assert!(resolved.unicode);
    /// ```
    pub fn resolve_render_settings(&self) -> ResolvedRenderSettings {
        let backend = match self.mode {
            RenderMode::Plain => RenderBackend::Plain,
            RenderMode::Rich => RenderBackend::Rich,
            RenderMode::Auto => {
                if matches!(self.color, ColorMode::Always)
                    || matches!(self.unicode, UnicodeMode::Always)
                {
                    RenderBackend::Rich
                } else if !self.runtime.stdout_is_tty
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
                table_border: self.table_border,
                help_table_border: self.help_chrome.table_chrome.resolve(self.table_border),
                theme_name,
                theme: theme.clone(),
                style_overrides: self.style_overrides.clone(),
                chrome_frame: self.chrome_frame,
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
                table_border: self.table_border,
                help_table_border: self.help_chrome.table_chrome.resolve(self.table_border),
                theme_name,
                theme,
                style_overrides: self.style_overrides.clone(),
                chrome_frame: self.chrome_frame,
            },
        }
    }

    /// Resolves the full UI render plan for one output payload.
    pub(crate) fn resolve_render_plan(&self, output: &OutputResult) -> ResolvedRenderPlan {
        let format = crate::ui::format::resolve_output_format(output, self);
        // JSON is a machine-readable surface. Once selected, rendering must not
        // inherit terminal color/Unicode chrome from the host runtime.
        let render = if matches!(format, OutputFormat::Json) {
            self.plain_copy_settings().resolve_render_settings()
        } else {
            self.resolve_render_settings()
        };

        ResolvedRenderPlan {
            format,
            render,
            guide: self.resolve_guide_render_settings(),
            mreg: self.resolve_mreg_build_settings(),
        }
    }

    fn resolve_width(&self) -> Option<usize> {
        if let Some(width) = self.width {
            return (width > 0).then_some(width);
        }
        self.runtime.width.filter(|width| *width > 0)
    }

    pub(crate) fn plain_copy_settings(&self) -> Self {
        Self {
            format: self.format,
            format_explicit: self.format_explicit,
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
            table_border: self.table_border,
            help_chrome: self.help_chrome,
            mreg_stack_min_col_width: self.mreg_stack_min_col_width,
            mreg_stack_overflow_ratio: self.mreg_stack_overflow_ratio,
            theme_name: self.theme_name.clone(),
            theme: self.theme.clone(),
            style_overrides: self.style_overrides.clone(),
            chrome_frame: self.chrome_frame,
            ruled_section_policy: self.ruled_section_policy,
            guide_default_format: self.guide_default_format,
            runtime: self.runtime.clone(),
        }
    }
}

impl HelpChromeSettings {
    pub(crate) fn resolve(self, table_border: TableBorderStyle) -> ResolvedHelpChromeSettings {
        ResolvedHelpChromeSettings {
            table_border: self.table_chrome.resolve(table_border),
            entry_indent: self.entry_indent,
            entry_gap: self.entry_gap,
            section_spacing: self.section_spacing,
        }
    }
}

impl ResolvedGuideRenderSettings {
    #[cfg(test)]
    pub(crate) fn plain_help(
        frame_style: SectionFrameStyle,
        table_border: TableBorderStyle,
    ) -> Self {
        Self {
            frame_style,
            ruled_section_policy: RuledSectionPolicy::PerSection,
            help_chrome: HelpChromeSettings {
                table_chrome: match table_border {
                    TableBorderStyle::None => crate::ui::HelpTableChrome::None,
                    TableBorderStyle::Square => crate::ui::HelpTableChrome::Square,
                    TableBorderStyle::Round => crate::ui::HelpTableChrome::Round,
                },
                ..HelpChromeSettings::default()
            }
            .resolve(table_border),
        }
    }
}
