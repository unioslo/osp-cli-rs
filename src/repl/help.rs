#[cfg(test)]
use crate::guide::GuideView;
#[cfg(test)]
use crate::ui::ResolvedRenderSettings;

#[cfg(test)]
use super::ReplViewContext;
#[cfg(test)]
use crate::ui::HelpLayout;
#[cfg(test)]
use crate::ui::{StructuredGuideRenderOptions, help_layout_from_config};

#[cfg(test)]
pub(crate) fn render_repl_help_with_chrome(view: ReplViewContext<'_>, help_text: &str) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let layout = help_layout_from_config(view.config);
    render_help_with_chrome(help_text, &resolved, layout)
}

#[cfg(test)]
pub(crate) fn render_help_with_chrome(
    help_text: &str,
    resolved: &ResolvedRenderSettings,
    layout: crate::ui::HelpLayout,
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
    let settings = canonical_help_settings(resolved);
    let parsed = escape_inline_markup(parsed);
    let mut rendered = crate::ui::render_guide_with_layout_with_chrome(
        &parsed,
        &settings,
        layout,
        matches!(layout, HelpLayout::Full),
        None,
    );
    rendered = rendered.replace("\\`", "`").replace("\\*", "*");
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

#[cfg(test)]
fn escape_inline_markup(mut guide: GuideView) -> GuideView {
    fn escape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            if matches!(ch, '`' | '*') {
                out.push('\\');
            }
            out.push(ch);
        }
        out
    }

    fn escape_entry(entry: &mut crate::guide::GuideEntry) {
        entry.name = escape(&entry.name);
        entry.short_help = escape(&entry.short_help);
        entry.display_indent = entry.display_indent.as_ref().map(|value| escape(value));
        entry.display_gap = entry.display_gap.as_ref().map(|value| escape(value));
    }

    guide.preamble = guide
        .preamble
        .into_iter()
        .map(|text| escape(&text))
        .collect();
    guide.usage = guide.usage.into_iter().map(|text| escape(&text)).collect();
    guide.notes = guide.notes.into_iter().map(|text| escape(&text)).collect();
    guide.epilogue = guide
        .epilogue
        .into_iter()
        .map(|text| escape(&text))
        .collect();
    for entry in &mut guide.commands {
        escape_entry(entry);
    }
    for entry in &mut guide.arguments {
        escape_entry(entry);
    }
    for entry in &mut guide.options {
        escape_entry(entry);
    }
    for entry in &mut guide.common_invocation_options {
        escape_entry(entry);
    }
    for section in &mut guide.sections {
        section.title = escape(&section.title);
        section.paragraphs = section.paragraphs.iter().map(|text| escape(text)).collect();
        for entry in &mut section.entries {
            escape_entry(entry);
        }
    }
    guide
}

#[cfg(test)]
fn canonical_help_settings(resolved: &ResolvedRenderSettings) -> crate::ui::RenderSettings {
    crate::ui::RenderSettings {
        format: crate::core::output::OutputFormat::Guide,
        format_explicit: true,
        mode: match resolved.backend {
            crate::ui::RenderBackend::Plain => crate::core::output::RenderMode::Plain,
            crate::ui::RenderBackend::Rich => crate::core::output::RenderMode::Rich,
        },
        color: if resolved.color {
            crate::core::output::ColorMode::Always
        } else {
            crate::core::output::ColorMode::Never
        },
        unicode: if resolved.unicode {
            crate::core::output::UnicodeMode::Always
        } else {
            crate::core::output::UnicodeMode::Never
        },
        theme_name: resolved.theme_name.clone(),
        theme: None,
        width: resolved.width,
        margin: resolved.margin,
        indent_size: resolved.indent_size,
        short_list_max: resolved.short_list_max,
        medium_list_max: resolved.medium_list_max,
        grid_padding: 4,
        grid_columns: None,
        column_weight: 3,
        table_overflow: crate::ui::TableOverflow::Clip,
        table_border: match resolved.help_table_border {
            crate::ui::TableBorderStyle::None => crate::ui::TableBorderStyle::None,
            crate::ui::TableBorderStyle::Square => crate::ui::TableBorderStyle::Square,
            crate::ui::TableBorderStyle::Round => crate::ui::TableBorderStyle::Round,
        },
        style_overrides: crate::ui::style::StyleOverrides::default(),
        help_chrome: crate::ui::HelpChromeSettings::default(),
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        chrome_frame: crate::ui::section_chrome::SectionFrameStyle::Top,
        ruled_section_policy: crate::ui::section_chrome::RuledSectionPolicy::Shared,
        guide_default_format: crate::ui::GuideDefaultFormat::Guide,
        runtime: crate::ui::RenderRuntime {
            stdout_is_tty: matches!(resolved.backend, crate::ui::RenderBackend::Rich),
            terminal: None,
            no_color: !resolved.color,
            width: resolved.width,
            locale_utf8: Some(resolved.unicode),
        },
    }
}

#[cfg(test)]
mod tests {
    mod output_support {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/output.rs"
        ));
    }

    use super::*;
    use crate::cli::Cli;
    use crate::core::output::OutputFormat;
    use crate::dsl::apply_output_pipeline;
    use crate::guide::{GuideEntry, GuideView};
    use crate::ui::style::StyleOverrides;
    use crate::ui::{
        GuideDefaultFormat, RenderBackend, RenderSettings, ResolvedRenderSettings,
        TableBorderStyle, TableOverflow,
    };
    use clap::Parser;

    fn resolved_settings(
        frame: crate::ui::section_chrome::SectionFrameStyle,
    ) -> ResolvedRenderSettings {
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
            help_chrome: crate::ui::ResolvedHelpChromeSettings {
                entry_indent: 2,
                entry_gap: None,
                section_spacing: 1,
            },
            chrome_frame: frame,
            guide_default_format: GuideDefaultFormat::Guide,
        }
    }

    fn normalize_help_text(text: &str) -> String {
        let mut normalized = text
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if text.ends_with('\n') {
            normalized.push('\n');
        }
        normalized
    }

    fn clap_help(args: &[&str]) -> String {
        Cli::try_parse_from(args)
            .expect_err("args should trigger clap help")
            .to_string()
    }

    fn guide_render_layout() -> HelpLayout {
        HelpLayout::Compact
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
    fn help_chrome_preserves_custom_preamble_and_extra_sections_unit() {
        let preamble = render_help_with_chrome(
            "Custom plugin help\nwith two intro lines\n\nUsage: osp sample\n\nCommands:\n  run\n",
            &resolved_settings(crate::ui::section_chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );
        assert!(preamble.contains("Custom plugin help"));
        assert!(preamble.contains("with two intro lines"));
        assert!(preamble.contains("Usage: osp sample"));
        assert!(preamble.contains("Commands:\n  run"));

        for layout in [HelpLayout::Compact, HelpLayout::Minimal] {
            let rendered = render_help_with_chrome(
                "Usage: osp sample\n\nExamples:\n  osp sample run\n\nNotes:\n  extra detail\n",
                &resolved_settings(crate::ui::section_chrome::SectionFrameStyle::None),
                layout,
            );
            assert!(rendered.contains("Examples:\n  osp sample run"));
            assert!(rendered.contains("Notes:\n  extra detail"));
        }
    }

    #[test]
    fn compact_and_austere_help_surfaces_match_clap_layout_unit() {
        for raw in [
            clap_help(&["osp", "--help"]),
            clap_help(&["osp", "theme", "--help"]),
        ] {
            for layout in [HelpLayout::Compact, HelpLayout::Minimal] {
                let rendered = render_help_with_chrome(
                    &raw,
                    &resolved_settings(crate::ui::section_chrome::SectionFrameStyle::None),
                    layout,
                );
                assert_eq!(
                    normalize_help_text(rendered.trim_end()),
                    normalize_help_text(raw.trim_end())
                );
            }
        }
    }

    #[test]
    fn compact_help_can_add_color_without_box_chrome_unit() {
        let raw = clap_help(&["osp", "--help"]);
        let mut resolved = resolved_settings(crate::ui::section_chrome::SectionFrameStyle::None);
        resolved.backend = RenderBackend::Rich;
        resolved.color = true;

        let rendered = render_help_with_chrome(&raw, &resolved, HelpLayout::Compact);
        assert!(rendered.contains("\u{1b}["));
        assert!(!rendered.contains('│'));
        assert!(!rendered.contains('┌'));
        assert_eq!(
            normalize_help_text(&output_support::strip_ansi(rendered.trim_end())),
            normalize_help_text(raw.trim_end())
        );
    }

    #[test]
    fn guide_rendering_prefers_semantic_help_until_explicit_format_wins_unit() {
        let guide = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");

        let mut guide_default = RenderSettings::test_plain(OutputFormat::Json);
        guide_default.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = crate::ui::render_structured_output_with_source_guide(
            &guide.to_output_result(),
            Some(&guide),
            &guide_default,
            guide_render_layout(),
        );
        assert!(rendered.contains("Usage:"));
        assert!(!rendered.trim_start().starts_with('['));

        let mut inherited = RenderSettings::test_plain(OutputFormat::Json);
        inherited.guide_default_format = GuideDefaultFormat::Inherit;
        let rendered = crate::ui::render_structured_output_with_source_guide(
            &guide.to_output_result(),
            Some(&guide),
            &inherited,
            guide_render_layout(),
        );
        assert!(rendered.contains("Usage"));
        assert!(rendered.contains("list"));
        assert!(!rendered.trim_start().starts_with('['));

        let mut explicit = RenderSettings::test_plain(OutputFormat::Json);
        explicit.guide_default_format = GuideDefaultFormat::Guide;
        explicit.format_explicit = true;
        let rendered = crate::ui::render_structured_output_with_source_guide(
            &guide.to_output_result(),
            Some(&guide),
            &explicit,
            guide_render_layout(),
        );
        assert!(rendered.trim_start().starts_with('['));
    }

    #[test]
    fn guide_output_after_pipeline_preserves_recommendation_until_explicit_format_or_markdown_overrides_unit()
     {
        let from_text = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n")
            .to_output_result();
        let mut rehydrated = RenderSettings::test_plain(OutputFormat::Json);
        rehydrated.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = crate::ui::render_structured_output_with_guide_options(
            &from_text,
            &rehydrated,
            StructuredGuideRenderOptions {
                source_guide: None,
                layout: guide_render_layout(),
                title_prefix: None,
                show_footer_rule: None,
            },
        );
        assert!(rendered.contains("Usage:"));
        assert!(!rendered.trim_start().starts_with('['));

        let filtered = filtered_guide_output();
        let mut semantic = RenderSettings::test_plain(OutputFormat::Auto);
        semantic.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = crate::ui::render_structured_output_with_guide_options(
            &filtered,
            &semantic,
            StructuredGuideRenderOptions {
                source_guide: None,
                layout: guide_render_layout(),
                title_prefix: None,
                show_footer_rule: None,
            },
        );
        assert!(rendered.contains("Commands:"));
        assert!(rendered.contains("list"));
        assert!(!rendered.trim_start().starts_with('['));

        let mut explicit_json = RenderSettings::test_plain(OutputFormat::Json);
        explicit_json.guide_default_format = GuideDefaultFormat::Guide;
        explicit_json.format_explicit = true;
        let rendered = crate::ui::render_structured_output_with_guide_options(
            &filtered,
            &explicit_json,
            StructuredGuideRenderOptions {
                source_guide: None,
                layout: guide_render_layout(),
                title_prefix: None,
                show_footer_rule: None,
            },
        );
        assert!(rendered.trim_start().starts_with('['));
        assert!(rendered.contains("\"commands\""));
        assert!(rendered.contains("\"short_help\""));

        let mut markdown = RenderSettings::test_plain(OutputFormat::Markdown);
        markdown.format_explicit = true;
        markdown.guide_default_format = GuideDefaultFormat::Guide;
        let rendered = crate::ui::render_structured_output_with_guide_options(
            &filtered,
            &markdown,
            StructuredGuideRenderOptions {
                source_guide: None,
                layout: guide_render_layout(),
                title_prefix: None,
                show_footer_rule: None,
            },
        );
        assert!(rendered.contains("## Commands"));
        assert!(rendered.contains("- `list` List history entries"));
        assert!(!rendered.contains("\"commands\""));
        assert!(!rendered.contains("+---"));
        assert!(!rendered.contains("| name"));
    }
}
