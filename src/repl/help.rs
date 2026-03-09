use crate::core::output::OutputFormat;
use crate::guide::{GuideDoc, GuidePayload};
use crate::ui::{RenderSettings, ResolvedRenderSettings, render_output};
use crate::ui::{
    format::help::{GuideRenderOptions, build_guide_document_from_doc},
    render_document_resolved,
};

#[cfg(test)]
use super::ReplViewContext;
use crate::ui::presentation::{HelpLayout, help_layout};

#[cfg(test)]
pub(crate) fn render_repl_help_with_chrome(view: ReplViewContext<'_>, help_text: &str) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let layout = help_layout(view.config);
    render_help_with_chrome(help_text, &resolved, layout)
}

#[cfg(test)]
pub(crate) fn render_help_with_chrome(
    help_text: &str,
    resolved: &ResolvedRenderSettings,
    layout: HelpLayout,
) -> String {
    let parsed = GuideDoc::from_text(help_text);
    if parsed.sections.is_empty() {
        return help_text.to_string();
    }
    let document = crate::ui::format::help::build_help_document_from_doc(
        &parsed,
        None,
        layout,
        resolved.chrome_frame,
    );
    render_help_document(document, resolved)
}

pub(crate) fn render_guide_doc_with_chrome(
    guide: &GuideDoc,
    resolved: &ResolvedRenderSettings,
    options: GuideRenderOptions<'_>,
) -> String {
    let document = build_guide_document_from_doc(guide, options);
    render_help_document(document, resolved)
}

pub(crate) fn render_help_payload(
    payload: &GuidePayload,
    settings: &RenderSettings,
    config: &crate::config::ResolvedConfig,
) -> String {
    if matches!(
        crate::ui::format::resolve_output_format(&payload.to_output_result(), settings),
        OutputFormat::Markdown
    ) {
        return payload.to_markdown_with_width(settings.resolve_render_settings().width);
    }
    render_help_doc_with_layout(&payload.to_doc(), settings, help_layout(config))
}

pub(crate) fn render_help_doc_with_layout(
    help: &GuideDoc,
    settings: &RenderSettings,
    layout: HelpLayout,
) -> String {
    render_guide_doc(
        help,
        settings,
        GuideRenderOptions {
            title_prefix: None,
            layout,
            frame_style: settings.chrome_frame,
            panel_kind: None,
        },
    )
}

pub(crate) fn render_guide_doc(
    guide: &GuideDoc,
    settings: &RenderSettings,
    options: GuideRenderOptions<'_>,
) -> String {
    if settings.prefers_guide_rendering() {
        return render_guide_doc_with_chrome(guide, &settings.resolve_render_settings(), options);
    }

    render_output(&guide.to_output_result(), settings)
}

pub(crate) fn render_guide_output(
    output: &crate::core::output_model::OutputResult,
    settings: &RenderSettings,
    options: GuideRenderOptions<'_>,
) -> String {
    let resolved_format = crate::ui::format::resolve_output_format(output, settings);
    if matches!(resolved_format, OutputFormat::Markdown)
        && let Some(payload) = GuidePayload::try_from_output_result(output)
    {
        return payload.to_markdown_with_width(settings.resolve_render_settings().width);
    }

    if settings.prefers_guide_rendering()
        && matches!(
            output.meta.render_recommendation,
            Some(crate::core::output_model::RenderRecommendation::Guide)
        )
        && let Some(payload) = GuidePayload::try_from_output_result(output)
    {
        let guide = payload.to_doc();
        return render_guide_doc_with_chrome(&guide, &settings.resolve_render_settings(), options);
    }

    render_output(output, settings)
}

fn render_help_document(
    document: crate::ui::Document,
    resolved: &ResolvedRenderSettings,
) -> String {
    let mut rendered = render_document_resolved(&document, resolved.clone());
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::output::OutputFormat;
    use crate::dsl::apply_output_pipeline;
    use crate::ui::style::StyleOverrides;
    use crate::ui::{
        GuideDefaultFormat, RenderBackend, RenderSettings, ResolvedRenderSettings,
        TableBorderStyle, TableOverflow,
    };
    use insta::assert_snapshot;

    fn resolved_settings(frame: crate::ui::chrome::SectionFrameStyle) -> ResolvedRenderSettings {
        ResolvedRenderSettings {
            backend: RenderBackend::Plain,
            color: false,
            unicode: false,
            width: Some(24),
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: TableBorderStyle::Square,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: StyleOverrides::default(),
            chrome_frame: frame,
        }
    }

    fn help_test_overrides() -> StyleOverrides {
        StyleOverrides {
            panel_title: Some("green".to_string()),
            key: Some("red".to_string()),
            value: Some("blue".to_string()),
            ..StyleOverrides::default()
        }
    }

    #[test]
    fn minimal_help_layout_matches_plain_snapshot_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n\nUse `osp plugins commands` to list plugin-provided commands.\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Minimal,
        );

        assert_snapshot!("repl_help_minimal_layout", rendered);
    }

    #[test]
    fn compact_help_layout_preserves_single_section_gap_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );

        assert_snapshot!("repl_help_compact_layout", rendered);
    }

    #[test]
    fn help_chrome_preserves_preamble_before_known_sections_unit() {
        let rendered = render_help_with_chrome(
            "Custom plugin help\nwith two intro lines\n\nUsage: osp sample\n\nCommands:\n  run\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );

        assert!(rendered.contains("Custom plugin help"));
        assert!(rendered.contains("with two intro lines"));
        assert!(rendered.contains("Usage:\n  osp sample"));
        assert!(rendered.contains("Commands:\n  run"));
    }

    #[test]
    fn help_chrome_preserves_custom_titled_sections_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp sample\n\nExamples:\n  osp sample run\n\nNotes:\n  extra detail\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );

        assert!(rendered.contains("Examples:\n  osp sample run"));
        assert!(rendered.contains("Notes:\n  extra detail"));
    }

    #[test]
    fn minimal_help_layout_preserves_custom_titled_sections_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp sample\n\nExamples:\n  osp sample run\n\nNotes:\n  extra detail\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Minimal,
        );

        assert!(rendered.contains("Examples:\n  osp sample run"));
        assert!(rendered.contains("Notes:\n  extra detail"));
    }

    #[test]
    fn help_chrome_colors_help_body_keys_and_text_unit() {
        let mut resolved = resolved_settings(crate::ui::chrome::SectionFrameStyle::TopBottom);
        resolved.color = true;
        resolved.style_overrides = help_test_overrides();

        let rendered = render_help_with_chrome(
            "Usage: osp history <COMMAND>\n\nCommands:\n  list   List stored history entries\n",
            &resolved,
            HelpLayout::Compact,
        );

        assert!(rendered.contains("\u{1b}[32mUsage\u{1b}[0m"));
        assert!(rendered.contains("\u{1b}[31mlist\u{1b}[0m"));
        assert!(rendered.contains("\u{1b}[34m   List stored history entries\u{1b}[0m"));
        assert!(rendered.contains("\u{1b}[34m  osp history <COMMAND>\u{1b}[0m"));
    }

    #[test]
    fn help_chrome_splits_single_space_command_descriptions_unit() {
        let mut resolved = resolved_settings(crate::ui::chrome::SectionFrameStyle::None);
        resolved.color = true;
        resolved.style_overrides = help_test_overrides();

        let rendered = render_help_with_chrome(
            "Commands:\n  list List stored history entries\n",
            &resolved,
            HelpLayout::Compact,
        );

        assert!(rendered.contains("\u{1b}[31mlist\u{1b}[0m"));
        assert!(rendered.contains("\u{1b}[34m List stored history entries\u{1b}[0m"));
    }

    #[test]
    fn guide_default_prefers_chrome_over_inherited_json_unit() {
        let guide = GuideDoc::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.guide_default_format = GuideDefaultFormat::Guide;

        let rendered = render_guide_doc(
            &guide,
            &settings,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: crate::ui::chrome::SectionFrameStyle::None,
                panel_kind: None,
            },
        );

        assert!(rendered.contains("Usage:"));
        assert!(!rendered.trim_start().starts_with('['));
    }

    #[test]
    fn guide_recommendation_beats_inherited_non_explicit_format_unit() {
        let guide = GuideDoc::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.guide_default_format = GuideDefaultFormat::Inherit;

        let rendered = render_guide_doc(
            &guide,
            &settings,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: crate::ui::chrome::SectionFrameStyle::None,
                panel_kind: None,
            },
        );

        assert!(rendered.contains("usage:"));
        assert!(!rendered.trim_start().starts_with('['));
    }

    #[test]
    fn explicit_format_beats_guide_default_unit() {
        let guide = GuideDoc::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.guide_default_format = GuideDefaultFormat::Guide;
        settings.format_explicit = true;

        let rendered = render_guide_doc(
            &guide,
            &settings,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: crate::ui::chrome::SectionFrameStyle::None,
                panel_kind: None,
            },
        );

        assert!(rendered.trim_start().starts_with('['));
    }

    #[test]
    fn guide_output_rehydrates_when_recommendation_survives_pipeline_unit() {
        let output = GuideDoc::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n")
            .to_output_result();
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.guide_default_format = GuideDefaultFormat::Guide;

        let rendered = render_guide_output(
            &output,
            &settings,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: crate::ui::chrome::SectionFrameStyle::None,
                panel_kind: None,
            },
        );

        assert!(rendered.contains("Usage:"));
        assert!(!rendered.trim_start().starts_with('['));
    }

    #[test]
    fn guide_output_after_quick_filter_keeps_guide_rendering_unit() {
        let output = GuidePayload {
            usage: vec!["history <COMMAND>".to_string()],
            commands: vec![
                crate::guide::GuidePayloadEntry {
                    name: "list".to_string(),
                    short_help: "List history entries".to_string(),
                },
                crate::guide::GuidePayloadEntry {
                    name: "prune".to_string(),
                    short_help: "Remove old history entries".to_string(),
                },
            ],
            ..GuidePayload::default()
        }
        .to_output_result();
        let output =
            apply_output_pipeline(output, &["list".to_string()]).expect("guide quick should work");

        let mut settings = RenderSettings::test_plain(OutputFormat::Auto);
        settings.guide_default_format = GuideDefaultFormat::Guide;

        let rendered = render_guide_output(
            &output,
            &settings,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: crate::ui::chrome::SectionFrameStyle::None,
                panel_kind: None,
            },
        );

        assert!(rendered.contains("Commands:"));
        assert!(rendered.contains("list"));
        assert!(!rendered.trim_start().starts_with('['));
    }

    #[test]
    fn explicit_format_beats_guide_recommendation_after_pipeline_unit() {
        let output = GuidePayload {
            usage: vec!["history <COMMAND>".to_string()],
            commands: vec![crate::guide::GuidePayloadEntry {
                name: "list".to_string(),
                short_help: "List history entries".to_string(),
            }],
            ..GuidePayload::default()
        }
        .to_output_result();
        let output =
            apply_output_pipeline(output, &["list".to_string()]).expect("guide quick should work");

        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.guide_default_format = GuideDefaultFormat::Guide;
        settings.format_explicit = true;

        let rendered = render_guide_output(
            &output,
            &settings,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: crate::ui::chrome::SectionFrameStyle::None,
                panel_kind: None,
            },
        );

        assert!(rendered.trim_start().starts_with('['));
        assert!(rendered.contains("\"commands\""));
        assert!(rendered.contains("\"short_help\""));
    }

    #[test]
    fn markdown_guide_output_uses_semantic_sections_after_pipeline_unit() {
        let output = GuidePayload {
            usage: vec!["history <COMMAND>".to_string()],
            commands: vec![
                crate::guide::GuidePayloadEntry {
                    name: "list".to_string(),
                    short_help: "List history entries".to_string(),
                },
                crate::guide::GuidePayloadEntry {
                    name: "prune".to_string(),
                    short_help: "Remove old history entries".to_string(),
                },
            ],
            ..GuidePayload::default()
        }
        .to_output_result();
        let output =
            apply_output_pipeline(output, &["list".to_string()]).expect("guide quick should work");

        let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
        settings.format_explicit = true;
        settings.guide_default_format = GuideDefaultFormat::Guide;

        let rendered = render_guide_output(
            &output,
            &settings,
            GuideRenderOptions {
                title_prefix: None,
                layout: HelpLayout::Compact,
                frame_style: crate::ui::chrome::SectionFrameStyle::None,
                panel_kind: None,
            },
        );

        assert!(rendered.contains("## Commands"));
        assert!(rendered.contains("| name"));
        assert!(rendered.contains("short_help"));
        assert!(rendered.contains("| list | List history entries |"));
        assert!(!rendered.contains("\"commands\""));
        assert!(!rendered.contains("+---"));
    }
}
