use osp_ui::ResolvedRenderSettings;
use osp_ui::messages::{
    SectionRenderContext, SectionStyleTokens, render_section_block_with_overrides,
};
use osp_ui::style::StyleToken;

use super::ReplViewContext;
use crate::ui_presentation::{HelpLayout, effective_help_layout};

pub(crate) fn render_repl_help_with_chrome(view: ReplViewContext<'_>, help_text: &str) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let layout = effective_help_layout(view.config);
    render_help_with_chrome(help_text, &resolved, layout)
}

pub(crate) fn render_help_with_chrome(
    help_text: &str,
    resolved: &ResolvedRenderSettings,
    layout: HelpLayout,
) -> String {
    let (preamble, sections) = parse_help_sections(help_text);
    if sections.is_empty() {
        return help_text.to_string();
    }

    let theme = &resolved.theme;
    let mut out = String::new();

    if !preamble.trim().is_empty() {
        out.push_str(preamble.trim_end());
        out.push_str(section_separator(layout));
    }

    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            out.push_str(section_separator(layout));
        }

        let body = normalize_help_body(&section.body, layout);
        out.push_str(&render_section_block_with_overrides(
            &section.title,
            &body,
            resolved.chrome_frame,
            resolved.unicode,
            resolved.width,
            SectionRenderContext {
                color: resolved.color,
                theme,
                style_overrides: &resolved.style_overrides,
            },
            SectionStyleTokens {
                border: StyleToken::PanelBorder,
                title: StyleToken::PanelTitle,
            },
        ));
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }

    out
}

fn normalize_help_body(body: &str, layout: HelpLayout) -> String {
    let lines = trim_blank_lines(body.lines().map(str::trim_end).collect());
    match layout {
        HelpLayout::Full => lines.join("\n"),
        HelpLayout::Compact => collapse_blank_runs(&lines, true).join("\n"),
        HelpLayout::Minimal => collapse_blank_runs(&lines, false).join("\n"),
    }
}

fn section_separator(layout: HelpLayout) -> &'static str {
    match layout {
        HelpLayout::Minimal => "\n",
        HelpLayout::Full | HelpLayout::Compact => "\n\n",
    }
}

fn trim_blank_lines(mut lines: Vec<&str>) -> Vec<&str> {
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

fn collapse_blank_runs<'a>(lines: &'a [&'a str], keep_single_blank: bool) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut last_blank = false;

    for line in lines {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if keep_single_blank && !last_blank {
                out.push(*line);
            }
            last_blank = true;
            continue;
        }
        out.push(*line);
        last_blank = false;
    }

    trim_blank_lines(out)
}

#[derive(Debug, Clone)]
struct HelpSection {
    title: String,
    body: String,
}

fn parse_help_sections(help_text: &str) -> (String, Vec<HelpSection>) {
    let mut preamble = String::new();
    let mut sections = Vec::new();
    let mut current: Option<HelpSection> = None;

    for raw_line in help_text.lines() {
        let line = raw_line.trim_end();

        if let Some(section) = parse_section_header(line) {
            if let Some(existing) = current.take() {
                sections.push(existing);
            }
            current = Some(section);
            continue;
        }

        let Some(section) = current.as_mut() else {
            if !preamble.is_empty() {
                preamble.push('\n');
            }
            preamble.push_str(line);
            continue;
        };

        if !section.body.is_empty() {
            section.body.push('\n');
        }
        section.body.push_str(line);
    }

    if let Some(section) = current {
        sections.push(section);
    }

    (preamble, sections)
}

fn parse_section_header(line: &str) -> Option<HelpSection> {
    if line.starts_with("Usage:") {
        let usage = line.trim_start_matches("Usage:").trim();
        return Some(HelpSection {
            title: "Usage".to_string(),
            body: if usage.is_empty() {
                String::new()
            } else {
                format!("  {usage}")
            },
        });
    }

    let title = match line {
        "Commands:" => "Commands".to_string(),
        "Options:" => "Options".to_string(),
        "Arguments:" => "Arguments".to_string(),
        "Common Invocation Options:" => "Common Invocation Options".to_string(),
        _ if !line.starts_with(' ') && line.ends_with(':') => {
            line.trim_end_matches(':').trim().to_string()
        }
        _ => return None,
    };

    Some(HelpSection {
        title,
        body: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use osp_ui::{RenderBackend, ResolvedRenderSettings, TableBorderStyle, TableOverflow};

    fn resolved_settings(frame: osp_ui::messages::SectionFrameStyle) -> ResolvedRenderSettings {
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
            theme_name: osp_ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: osp_ui::theme::resolve_theme(osp_ui::theme::DEFAULT_THEME_NAME),
            style_overrides: osp_ui::style::StyleOverrides::default(),
            chrome_frame: frame,
        }
    }

    #[test]
    fn minimal_help_layout_matches_plain_snapshot_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n\nUse `osp plugins commands` to list plugin-provided commands.\n",
            &resolved_settings(osp_ui::messages::SectionFrameStyle::None),
            HelpLayout::Minimal,
        );

        assert_eq!(
            rendered,
            "Usage:\n  osp [OPTIONS]\nCommands:\n  help\nOptions:\n  -h, --help\nUse `osp plugins commands` to list plugin-provided commands.\n"
        );
    }

    #[test]
    fn compact_help_layout_preserves_single_section_gap_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
            &resolved_settings(osp_ui::messages::SectionFrameStyle::None),
            HelpLayout::Compact,
        );

        assert_eq!(
            rendered,
            "Usage:\n  osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n"
        );
    }

    #[test]
    fn help_chrome_preserves_preamble_before_known_sections_unit() {
        let rendered = render_help_with_chrome(
            "Custom plugin help\nwith two intro lines\n\nUsage: osp sample\n\nCommands:\n  run\n",
            &resolved_settings(osp_ui::messages::SectionFrameStyle::None),
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
            &resolved_settings(osp_ui::messages::SectionFrameStyle::None),
            HelpLayout::Compact,
        );

        assert!(rendered.contains("Examples:\n  osp sample run"));
        assert!(rendered.contains("Notes:\n  extra detail"));
    }

    #[test]
    fn minimal_help_layout_preserves_custom_titled_sections_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp sample\n\nExamples:\n  osp sample run\n\nNotes:\n  extra detail\n",
            &resolved_settings(osp_ui::messages::SectionFrameStyle::None),
            HelpLayout::Minimal,
        );

        assert!(rendered.contains("Examples:\n  osp sample run"));
        assert!(rendered.contains("Notes:\n  extra detail"));
    }
}
