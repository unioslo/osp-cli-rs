use osp_ui::ResolvedRenderSettings;
use osp_ui::style::{StyleToken, apply_style_with_theme_overrides};

use super::ReplViewContext;

pub(crate) fn render_repl_help_with_chrome(view: ReplViewContext<'_>, help_text: &str) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
    render_help_with_chrome(help_text, &resolved)
}

pub(crate) fn render_help_with_chrome(
    help_text: &str,
    resolved: &ResolvedRenderSettings,
) -> String {
    let theme = &resolved.theme;
    let mut out = String::new();
    let mut rendered_sections = false;

    for raw_line in help_text.lines() {
        let line = raw_line.trim_end();

        let section_name = if line.starts_with("Usage:") {
            Some("Usage")
        } else if line == "Commands:" {
            Some("Commands")
        } else if line == "Options:" {
            Some("Options")
        } else if line == "Arguments:" {
            Some("Arguments")
        } else {
            None
        };

        if let Some(section) = section_name {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            rendered_sections = true;

            if section == "Usage" {
                let usage = line.trim_start_matches("Usage:").trim();
                let label = if resolved.color {
                    apply_style_with_theme_overrides(
                        "Usage:",
                        StyleToken::MessageInfo,
                        true,
                        theme,
                        &resolved.style_overrides,
                    )
                } else {
                    "Usage:".to_string()
                };
                out.push_str("  ");
                out.push_str(&label);
                if !usage.is_empty() {
                    out.push(' ');
                    out.push_str(usage);
                }
                out.push('\n');
            } else {
                let label = if resolved.color {
                    apply_style_with_theme_overrides(
                        &format!("{section}:"),
                        StyleToken::PanelTitle,
                        true,
                        theme,
                        &resolved.style_overrides,
                    )
                } else {
                    format!("{section}:")
                };
                out.push_str(&label);
                out.push('\n');
            }
            continue;
        }

        if line.is_empty() {
            out.push('\n');
            continue;
        }

        out.push_str(line);
        out.push('\n');
    }

    if !rendered_sections {
        return help_text.to_string();
    }

    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}
