use crate::guide::GuideView;
#[cfg(test)]
use crate::ui::ResolvedGuideRenderSettings;
use crate::ui::{
    RenderSettings, ResolvedRenderSettings, render_guide_output_with_options,
    render_guide_view_with_options,
};
use crate::ui::{format::help::GuideRenderOptions, render_document_resolved};

#[cfg(test)]
use super::ReplViewContext;
#[cfg(test)]
use crate::ui::presentation::HelpLayout;
#[cfg(test)]
use crate::ui::presentation::help_layout;

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
    let parsed = GuideView::from_text(help_text);
    if parsed.sections.is_empty()
        && parsed.usage.is_empty()
        && parsed.commands.is_empty()
        && parsed.arguments.is_empty()
        && parsed.options.is_empty()
        && parsed.common_invocation_options.is_empty()
        && parsed.notes.is_empty()
    {
        return help_text.to_string();
    }
    let document = crate::ui::format::help::build_help_document_from_view(
        &parsed,
        None,
        layout,
        ResolvedGuideRenderSettings::plain_help(resolved.chrome_frame, resolved.help_table_border),
    );
    render_help_document(document, resolved)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn render_guide_doc(
    guide: &GuideView,
    settings: &RenderSettings,
    options: GuideRenderOptions<'_>,
) -> String {
    render_guide_view_with_options(guide, settings, options)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn render_guide_output(
    output: &crate::core::output_model::OutputResult,
    settings: &RenderSettings,
    options: GuideRenderOptions<'_>,
) -> String {
    render_guide_output_with_options(output, settings, options)
}

#[cfg_attr(not(test), allow(dead_code))]
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
    use crate::guide::{GuideEntry, GuideView};
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
            help_table_border: TableBorderStyle::None,
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

    fn guide_render_options() -> GuideRenderOptions<'static> {
        GuideRenderOptions {
            title_prefix: None,
            layout: HelpLayout::Compact,
            guide: ResolvedGuideRenderSettings::plain_help(
                crate::ui::chrome::SectionFrameStyle::None,
                TableBorderStyle::None,
            ),
            panel_kind: None,
        }
    }

    fn filtered_guide_output() -> crate::core::output_model::OutputResult {
        let output = GuideView {
            usage: vec!["history <COMMAND>".to_string()],
            commands: vec![
                GuideEntry {
                    name: "list".to_string(),
                    short_help: "List history entries".to_string(),
                    display_indent: None,
                    display_gap: None,
                },
                GuideEntry {
                    name: "prune".to_string(),
                    short_help: "Remove old history entries".to_string(),
                    display_indent: None,
                    display_gap: None,
                },
            ],
            ..GuideView::default()
        }
        .to_output_result();
        apply_output_pipeline(output, &["list".to_string()]).expect("guide quick should work")
    }

    #[test]
    fn help_chrome_layouts_preserve_snapshots_and_custom_sections_unit() {
        let minimal = render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n\nUse `osp plugins commands` to list plugin-provided commands.\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Minimal,
        );
        assert_snapshot!("repl_help_minimal_layout", minimal);

        let compact = render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );
        assert_snapshot!("repl_help_compact_layout", compact);

        let preamble = render_help_with_chrome(
            "Custom plugin help\nwith two intro lines\n\nUsage: osp sample\n\nCommands:\n  run\n",
            &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );
        assert!(preamble.contains("Custom plugin help"));
        assert!(preamble.contains("with two intro lines"));
        assert!(preamble.contains("Usage:\n  osp sample"));
        assert!(preamble.contains("Commands:\n  run"));

        for layout in [HelpLayout::Compact, HelpLayout::Minimal] {
            let rendered = render_help_with_chrome(
                "Usage: osp sample\n\nExamples:\n  osp sample run\n\nNotes:\n  extra detail\n",
                &resolved_settings(crate::ui::chrome::SectionFrameStyle::None),
                layout,
            );
            assert!(rendered.contains("Examples:\n  osp sample run"));
            assert!(rendered.contains("Notes:\n  extra detail"));
        }
    }

    #[test]
    fn help_chrome_styles_color_text_and_split_command_descriptions_unit() {
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

        let mut split = resolved_settings(crate::ui::chrome::SectionFrameStyle::None);
        split.color = true;
        split.style_overrides = help_test_overrides();
        let rendered = render_help_with_chrome(
            "Commands:\n  list List stored history entries\n",
            &split,
            HelpLayout::Compact,
        );
        assert!(rendered.contains("\u{1b}[31mlist\u{1b}[0m"));
        assert!(rendered.contains("\u{1b}[34m List stored history entries\u{1b}[0m"));
    }

    #[test]
    fn guide_rendering_prefers_semantic_help_until_explicit_format_wins_unit() {
        let guide = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");

        let mut guide_default = RenderSettings::test_plain(OutputFormat::Json);
        guide_default.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = render_guide_doc(&guide, &guide_default, guide_render_options());
        assert!(rendered.contains("Usage:"));
        assert!(!rendered.trim_start().starts_with('['));

        let mut inherited = RenderSettings::test_plain(OutputFormat::Json);
        inherited.guide_default_format = GuideDefaultFormat::Inherit;
        let rendered = render_guide_doc(&guide, &inherited, guide_render_options());
        assert!(rendered.contains("Usage"));
        assert!(rendered.contains("list"));
        assert!(!rendered.trim_start().starts_with('['));

        let mut explicit = RenderSettings::test_plain(OutputFormat::Json);
        explicit.guide_default_format = GuideDefaultFormat::Guide;
        explicit.format_explicit = true;
        let rendered = render_guide_doc(&guide, &explicit, guide_render_options());
        assert!(rendered.trim_start().starts_with('['));
    }

    #[test]
    fn guide_output_after_pipeline_preserves_recommendation_until_explicit_format_or_markdown_overrides_unit()
     {
        let from_text = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n")
            .to_output_result();
        let mut rehydrated = RenderSettings::test_plain(OutputFormat::Json);
        rehydrated.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = render_guide_output(&from_text, &rehydrated, guide_render_options());
        assert!(rendered.contains("Usage:"));
        assert!(!rendered.trim_start().starts_with('['));

        let filtered = filtered_guide_output();
        let mut semantic = RenderSettings::test_plain(OutputFormat::Auto);
        semantic.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = render_guide_output(&filtered, &semantic, guide_render_options());
        assert!(rendered.contains("Commands:"));
        assert!(rendered.contains("list"));
        assert!(!rendered.trim_start().starts_with('['));

        let mut explicit_json = RenderSettings::test_plain(OutputFormat::Json);
        explicit_json.guide_default_format = GuideDefaultFormat::Guide;
        explicit_json.format_explicit = true;
        let rendered = render_guide_output(&filtered, &explicit_json, guide_render_options());
        assert!(rendered.trim_start().starts_with('['));
        assert!(rendered.contains("\"commands\""));
        assert!(rendered.contains("\"short_help\""));

        let mut markdown = RenderSettings::test_plain(OutputFormat::Markdown);
        markdown.format_explicit = true;
        markdown.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = render_guide_output(&filtered, &markdown, guide_render_options());
        assert!(rendered.contains("## Commands"));
        assert!(rendered.contains("- `list` List history entries"));
        assert!(!rendered.contains("\"commands\""));
        assert!(!rendered.contains("+---"));
        assert!(!rendered.contains("| name"));
    }
}
