use crate::osp_ui::ResolvedRenderSettings;
use crate::osp_ui::chrome::{
    SectionRenderContext, SectionStyleTokens, render_section_block_with_overrides,
};
use crate::osp_ui::style::{StyleToken, apply_style_with_theme_overrides};

use super::ReplViewContext;
use crate::osp_cli::ui_presentation::{HelpLayout, help_layout};

pub(crate) fn render_repl_help_with_chrome(view: ReplViewContext<'_>, help_text: &str) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let layout = help_layout(view.config);
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

        let body = style_help_body(
            &section.title,
            &normalize_help_body(&section.body, layout),
            resolved,
        );
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

fn style_help_body(title: &str, body: &str, resolved: &ResolvedRenderSettings) -> String {
    if !resolved.color {
        return body.to_string();
    }

    body.lines()
        .map(|line| style_help_line(title, line, resolved))
        .collect::<Vec<_>>()
        .join("\n")
}

fn style_help_line(title: &str, line: &str, resolved: &ResolvedRenderSettings) -> String {
    if line.trim().is_empty() {
        return line.to_string();
    }

    match title {
        "Commands" | "Options" | "Arguments" | "Common Invocation Options" => {
            style_help_keyed_line(title, line, resolved)
        }
        _ => style_help_text(line, resolved),
    }
}

fn style_help_keyed_line(
    section_title: &str,
    line: &str,
    resolved: &ResolvedRenderSettings,
) -> String {
    let indent_len = line.len().saturating_sub(line.trim_start().len());
    let (indent, rest) = line.split_at(indent_len);
    let split = help_description_split(section_title, rest).unwrap_or(rest.len());
    let (head, tail) = rest.split_at(split);

    let mut out = String::new();
    out.push_str(indent);
    out.push_str(&style_help_segment(head, StyleToken::Key, resolved));
    if !tail.is_empty() {
        out.push_str(&style_help_segment(tail, StyleToken::Value, resolved));
    }
    out
}

fn help_description_split(section_title: &str, line: &str) -> Option<usize> {
    let mut saw_non_whitespace = false;
    let mut run_start = None;
    let mut run_len = 0usize;

    for (idx, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if saw_non_whitespace {
                run_start.get_or_insert(idx);
                run_len += 1;
            }
            continue;
        }

        if saw_non_whitespace && run_len >= 2 {
            return run_start;
        }

        saw_non_whitespace = true;
        run_start = None;
        run_len = 0;
    }

    if matches!(section_title, "Commands" | "Arguments") {
        return line.find(char::is_whitespace);
    }

    None
}

fn style_help_text(line: &str, resolved: &ResolvedRenderSettings) -> String {
    style_help_segment(line, StyleToken::Value, resolved)
}

fn style_help_segment(text: &str, token: StyleToken, resolved: &ResolvedRenderSettings) -> String {
    apply_style_with_theme_overrides(
        text,
        token,
        true,
        &resolved.theme,
        &resolved.style_overrides,
    )
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
    use crate::osp_ui::style::StyleOverrides;
    use crate::osp_ui::{RenderBackend, ResolvedRenderSettings, TableBorderStyle, TableOverflow};
    use insta::assert_snapshot;

    fn resolved_settings(
        frame: crate::osp_ui::chrome::SectionFrameStyle,
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
            theme_name: crate::osp_ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::osp_ui::theme::resolve_theme(crate::osp_ui::theme::DEFAULT_THEME_NAME),
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
            &resolved_settings(crate::osp_ui::chrome::SectionFrameStyle::None),
            HelpLayout::Minimal,
        );

        assert_snapshot!("repl_help_minimal_layout", rendered);
    }

    #[test]
    fn compact_help_layout_preserves_single_section_gap_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
            &resolved_settings(crate::osp_ui::chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );

        assert_snapshot!("repl_help_compact_layout", rendered);
    }

    #[test]
    fn help_chrome_preserves_preamble_before_known_sections_unit() {
        let rendered = render_help_with_chrome(
            "Custom plugin help\nwith two intro lines\n\nUsage: osp sample\n\nCommands:\n  run\n",
            &resolved_settings(crate::osp_ui::chrome::SectionFrameStyle::None),
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
            &resolved_settings(crate::osp_ui::chrome::SectionFrameStyle::None),
            HelpLayout::Compact,
        );

        assert!(rendered.contains("Examples:\n  osp sample run"));
        assert!(rendered.contains("Notes:\n  extra detail"));
    }

    #[test]
    fn minimal_help_layout_preserves_custom_titled_sections_unit() {
        let rendered = render_help_with_chrome(
            "Usage: osp sample\n\nExamples:\n  osp sample run\n\nNotes:\n  extra detail\n",
            &resolved_settings(crate::osp_ui::chrome::SectionFrameStyle::None),
            HelpLayout::Minimal,
        );

        assert!(rendered.contains("Examples:\n  osp sample run"));
        assert!(rendered.contains("Notes:\n  extra detail"));
    }

    #[test]
    fn help_chrome_colors_help_body_keys_and_text_unit() {
        let mut resolved = resolved_settings(crate::osp_ui::chrome::SectionFrameStyle::TopBottom);
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
        let mut resolved = resolved_settings(crate::osp_ui::chrome::SectionFrameStyle::None);
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
}
