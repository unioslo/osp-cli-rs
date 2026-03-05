use osp_ui::ResolvedRenderSettings;
use osp_ui::messages::render_section_divider_with_overrides;
use osp_ui::style::StyleToken;

use crate::state::AppState;

pub(crate) fn render_repl_help_with_chrome(state: &AppState, help_text: &str) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    render_help_with_chrome(help_text, &resolved)
}

pub(crate) fn render_help_with_chrome(
    help_text: &str,
    resolved: &ResolvedRenderSettings,
) -> String {
    let theme_name = resolved.theme_name.as_str();
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
            out.push_str(&render_section_divider_with_overrides(
                section,
                resolved.unicode,
                resolved.width,
                resolved.color,
                theme_name,
                StyleToken::MessageInfo,
                &resolved.style_overrides,
            ));
            out.push('\n');
            rendered_sections = true;

            if section == "Usage" {
                let usage = line.trim_start_matches("Usage:").trim();
                if !usage.is_empty() {
                    out.push_str("  ");
                    out.push_str(usage);
                    out.push('\n');
                }
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
    out.push_str(&render_section_divider_with_overrides(
        "",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme_name,
        StyleToken::MessageInfo,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out
}
