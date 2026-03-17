#![allow(missing_docs)]

//! Canonical UI pipeline.
//!
//! This module owns planning, lowering, and emission for human-facing output.
//! It is intentionally small in the core implementation slice:
//!
//! - one planner decides the effective output format
//! - one human-facing document IR carries structure
//! - one lowering pass turns payloads into that IR
//! - one emitter family renders terminal or markdown output
//! - sidecar subsystems such as messages live in their own folders

pub mod clipboard;
pub mod interact;
pub mod messages;
pub mod section_chrome;
pub mod style;
pub mod theme;
pub mod theme_catalog;

mod chrome;
mod doc;
mod emit;
mod lower;
mod plan;
pub(crate) mod settings;
mod text;

use crate::config::ResolvedConfig;
use crate::core::output::OutputFormat;
use crate::core::output_model::OutputResult;
use crate::guide::{GuideSection, GuideSectionKind, GuideView};

pub(crate) use messages::{
    render_messages_from_settings as render_messages, render_messages_without_config,
};
pub(crate) use plan::plan_output;
pub use settings::RenderBackend;
#[allow(unused_imports)]
pub use settings::{
    GuideDefaultFormat, HelpChromeSettings, HelpLayout, HelpTableChrome, PresentationEffect,
    RenderProfile, RenderRuntime, RenderRuntimeBuilder, RenderSettings, RenderSettingsBuilder,
    ResolvedHelpChromeSettings, ResolvedRenderSettings, TableBorderStyle, TableOverflow,
    UiPresentation, help_layout_from_config, resolve_settings,
};
pub(crate) use settings::{build_presentation_defaults_layer, explain_presentation_effect};
pub use style::{StyleOverrides, StyleToken, ThemeStyler};
pub(crate) use text::visible_inline_text;
pub use theme::DEFAULT_THEME_NAME;
pub use theme_catalog as theme_loader;

#[derive(Debug, Clone, Copy)]
pub(crate) struct StructuredGuideRenderOptions<'a> {
    pub(crate) source_guide: Option<&'a GuideView>,
    pub(crate) layout: HelpLayout,
    pub(crate) title_prefix: Option<&'a str>,
    pub(crate) show_footer_rule: Option<bool>,
}

fn render_output_with_profile(
    output: &OutputResult,
    settings: &RenderSettings,
    profile: RenderProfile,
) -> String {
    let plan = plan_output(output, settings, profile);
    emit::emit_doc(
        &lower::lower_output(output, &plan),
        plan.format,
        &plan.settings,
    )
}

pub fn render_rows(rows: &[crate::core::row::Row], settings: &RenderSettings) -> String {
    render_output(
        &OutputResult {
            items: crate::core::output_model::OutputItems::Rows(rows.to_vec()),
            document: None,
            meta: Default::default(),
        },
        settings,
    )
}

pub fn render_output(output: &OutputResult, settings: &RenderSettings) -> String {
    render_output_with_profile(output, settings, RenderProfile::Normal)
}

pub fn render_output_for_copy(output: &OutputResult, settings: &RenderSettings) -> String {
    render_output_with_profile(output, settings, RenderProfile::CopySafe)
}

pub(crate) fn render_json_value(value: &serde_json::Value, settings: &RenderSettings) -> String {
    let render_settings = settings.plain_copy_settings();
    let resolved = resolve_settings(&render_settings, RenderProfile::CopySafe);
    emit::emit_doc(
        &doc::Doc {
            blocks: vec![doc::Block::Json(doc::JsonBlock {
                text: serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string()),
            })],
        },
        OutputFormat::Json,
        &resolved,
    )
}

pub(crate) fn render_structured_output(
    config: &ResolvedConfig,
    settings: &RenderSettings,
    output: &OutputResult,
) -> String {
    render_structured_output_with_config(output, None, settings, config)
}

pub(crate) fn copy_output_to_clipboard(
    output: &OutputResult,
    settings: &RenderSettings,
    clipboard: &clipboard::ClipboardService,
) -> Result<(), clipboard::ClipboardError> {
    clipboard.copy_text(&render_output_for_copy(output, settings))
}

#[cfg(test)]
pub(crate) fn render_structured_output_with_layout(
    output: &OutputResult,
    settings: &RenderSettings,
    layout: HelpLayout,
) -> String {
    render_structured_output_with_source_guide(output, None, settings, layout)
}

pub(crate) fn render_structured_output_with_source_guide(
    output: &OutputResult,
    source_guide: Option<&GuideView>,
    settings: &RenderSettings,
    layout: HelpLayout,
) -> String {
    render_structured_output_with_guide_options(
        output,
        settings,
        StructuredGuideRenderOptions {
            source_guide,
            layout,
            title_prefix: None,
            show_footer_rule: None,
        },
    )
}

pub(crate) fn render_structured_output_with_config(
    output: &OutputResult,
    source_guide: Option<&GuideView>,
    settings: &RenderSettings,
    config: &ResolvedConfig,
) -> String {
    render_structured_output_with_guide_options(
        output,
        settings,
        StructuredGuideRenderOptions {
            source_guide,
            layout: help_layout_from_config(config),
            title_prefix: None,
            show_footer_rule: None,
        },
    )
}

pub(crate) fn render_structured_output_with_guide_options(
    output: &OutputResult,
    settings: &RenderSettings,
    options: StructuredGuideRenderOptions<'_>,
) -> String {
    let plan = plan_output(output, settings, RenderProfile::Normal);
    if !matches!(plan.format, OutputFormat::Guide | OutputFormat::Markdown) {
        return emit::emit_doc(
            &lower::lower_output(output, &plan),
            plan.format,
            &plan.settings,
        );
    }

    let Some(guide) = options
        .source_guide
        .cloned()
        .or_else(|| GuideView::try_from_output_result(output))
        .or_else(|| GuideView::try_from_row_projection(output))
    else {
        return render_output(output, settings);
    };

    let guide = apply_title_prefix(&guide, options.title_prefix);
    let guide_output = guide.to_output_result();
    let guide_plan = plan_output(&guide_output, settings, RenderProfile::Normal);

    if matches!(guide_plan.format, OutputFormat::Markdown) {
        emit::emit_doc(
            &lower::lower_output(&guide_output, &guide_plan),
            OutputFormat::Markdown,
            &guide_plan.settings,
        )
    } else {
        emit::emit_doc(
            &lower::lower_guide_help_layout(
                &guide,
                &guide_plan,
                options.layout,
                options
                    .show_footer_rule
                    .unwrap_or_else(|| default_show_footer_rule(options.layout)),
            ),
            OutputFormat::Guide,
            &guide_plan.settings,
        )
    }
}

#[cfg(test)]
pub(crate) fn render_guide_with_layout(
    guide: &GuideView,
    settings: &RenderSettings,
    layout: HelpLayout,
) -> String {
    render_guide_with_layout_with_chrome(
        guide,
        settings,
        layout,
        default_show_footer_rule(layout),
        None,
    )
}

#[cfg(test)]
pub(crate) fn render_guide_with_layout_with_chrome(
    guide: &GuideView,
    settings: &RenderSettings,
    layout: HelpLayout,
    show_footer_rule: bool,
    title_prefix: Option<&str>,
) -> String {
    let output = guide.to_output_result();
    render_structured_output_with_guide_options(
        &output,
        settings,
        StructuredGuideRenderOptions {
            source_guide: Some(guide),
            layout,
            title_prefix,
            show_footer_rule: Some(show_footer_rule),
        },
    )
}

fn default_show_footer_rule(layout: HelpLayout) -> bool {
    matches!(layout, HelpLayout::Full)
}

fn apply_title_prefix(view: &GuideView, title_prefix: Option<&str>) -> GuideView {
    let Some(prefix) = title_prefix else {
        return view.clone();
    };
    let mut updated = view.clone();
    if let Some(first) = updated.sections.first_mut() {
        first.title = format!("{prefix} · {}", first.title);
    } else if !updated.usage.is_empty() {
        updated.sections.insert(
            0,
            GuideSection {
                title: format!("{prefix} · Usage"),
                kind: GuideSectionKind::Usage,
                paragraphs: updated.usage.clone(),
                entries: Vec::new(),
                data: None,
            },
        );
        updated.usage.clear();
    }
    updated
}

#[cfg(test)]
mod tests;
