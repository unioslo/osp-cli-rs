use crate::app::DEFAULT_REPL_PROMPT;
use crate::state::AppState;
use osp_repl::{ReplAppearance, ReplPrompt};
use osp_ui::messages::render_section_divider_with_overrides;
use osp_ui::render_inline;
use osp_ui::style::{
    StyleToken, apply_style_spec, apply_style_with_theme, apply_style_with_theme_overrides,
};

use super::surface::ReplSurface;

pub(crate) fn render_repl_intro(state: &AppState) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let config = state.config.resolved();
    let theme = &resolved.theme;

    let user = config.get_string("user.name").unwrap_or("anonymous");
    let display_name = config
        .get_string("user.display_name")
        .or_else(|| config.get_string("user.full_name"))
        .unwrap_or(user);
    let theme_id = state.ui.render_settings.theme_name.clone();
    let version = env!("CARGO_PKG_VERSION");
    let theme_display = theme_display_name(&theme_id);

    let mut out = String::new();
    out.push('\n');
    out.push_str(&render_section_divider_with_overrides(
        "OSP",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Welcome `{display_name}`!"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Logged in as: `{user}`"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Theme: `{theme_display}`"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Version: `{version}`"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push_str("\n\n");
    out.push_str(&render_section_divider_with_overrides(
        "Keybindings",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `Ctrl-D`    **exit**",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `Ctrl-L`    **clear screen**",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `Ctrl-R`    **reverse search history**",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push_str("\n\n");
    out.push_str(&render_section_divider_with_overrides(
        "Pipes",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `F` key>3 *|* `P` col1 col2 *|* `S` sort_key *|* `G` group_by_k1 k2",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  *|* `A` metric() *|* `L` limit offset *|* `C` count",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `K` key *|* `V` value *|* contains *|* !not *|* ?exist *|* !?not_exist *(= exact, == case-sens.)*",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  *Help:* `| H` *or* `| H <verb>` *e.g.* `| H F`",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push_str("\n\n");
    out
}

pub(crate) fn render_repl_command_overview(state: &AppState, surface: &ReplSurface) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let theme = &resolved.theme;
    let mut out = String::new();

    let usage_label = if resolved.color {
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
    out.push(' ');
    out.push_str(&usage_label);
    out.push_str("  [OPTIONS] COMMAND [ARGS]...\n\n");

    out.push_str(&render_section_divider_with_overrides(
        "Commands",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');

    for entry in &surface.overview_entries {
        let name = format!("{:<12}", entry.name);
        out.push_str("  ");
        out.push_str(&style_command_name(&resolved, theme, &name));
        out.push_str(&entry.summary);
        out.push('\n');
    }

    out.push_str(&render_section_divider_with_overrides(
        "",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out
}

fn style_command_name(
    resolved: &osp_ui::ResolvedRenderSettings,
    theme: &osp_ui::theme::ThemeDefinition,
    name: &str,
) -> String {
    if resolved.color {
        apply_style_with_theme_overrides(
            name,
            StyleToken::Key,
            true,
            theme,
            &resolved.style_overrides,
        )
    } else {
        name.to_string()
    }
}

pub(crate) fn theme_display_name(slug: &str) -> String {
    let normalized = slug
        .split(['-', '_'])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut out = first.to_uppercase().to_string();
            out.push_str(&chars.as_str().to_ascii_lowercase());
            out
        })
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        slug.to_string()
    } else {
        normalized
    }
}

pub(crate) fn build_repl_appearance(state: &AppState) -> ReplAppearance {
    let resolved = state.ui.render_settings.resolve_render_settings();
    if !resolved.color {
        return ReplAppearance::default();
    }
    let theme = &resolved.theme;
    let config = state.config.resolved();

    let config_style = |key: &str| {
        config
            .get_string(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };

    let completion_text_style = config_style("color.prompt.completion.text")
        .unwrap_or_else(|| theme.repl_completion_text_spec().to_string());
    let completion_background_style = config_style("color.prompt.completion.background")
        .unwrap_or_else(|| theme.repl_completion_background_spec().to_string());
    let completion_highlight_style = config_style("color.prompt.completion.highlight")
        .unwrap_or_else(|| theme.repl_completion_highlight_spec().to_string());
    let command_highlight_style =
        config_style("color.prompt.command").unwrap_or_else(|| theme.palette.success.to_string());

    ReplAppearance {
        completion_text_style: Some(completion_text_style),
        completion_background_style: Some(completion_background_style),
        completion_highlight_style: Some(completion_highlight_style),
        command_highlight_style: Some(command_highlight_style),
    }
}

pub(crate) fn build_repl_prompt(state: &AppState) -> ReplPrompt {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let config = state.config.resolved();
    let theme = &resolved.theme;
    let simple = config.get_bool("repl.simple_prompt").unwrap_or(false);
    let profile = config.active_profile();
    let user = config.get_string("user.name").unwrap_or("anonymous");
    let domain = config.get_string("domain").unwrap_or("local");
    let indicator = build_shell_indicator(state);
    let prompt_style = config.get_string("color.prompt.text");

    let user_text = style_prompt_fragment(
        prompt_style,
        user,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let domain_text = style_prompt_fragment(
        prompt_style,
        domain,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let profile_text = style_prompt_fragment(
        prompt_style,
        profile,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let indicator_text = style_prompt_fragment(
        prompt_style,
        &indicator,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );

    let prompt = if simple {
        let suffix = style_prompt_fragment(
            prompt_style,
            "> ",
            StyleToken::PromptText,
            resolved.color,
            theme,
        );
        format!("{profile_text}{suffix}")
    } else {
        let template = config
            .get_string("repl.prompt")
            .unwrap_or(DEFAULT_REPL_PROMPT);
        render_prompt_template_styled(
            template,
            &user_text,
            &domain_text,
            &profile_text,
            &indicator_text,
            prompt_style,
            resolved.color,
            theme,
        )
    };

    ReplPrompt::simple(prompt)
}

fn build_shell_indicator(state: &AppState) -> String {
    let Some(stack) = state.session.scope.display_label() else {
        return String::new();
    };
    let template = state
        .config
        .resolved()
        .get_string("repl.shell_indicator")
        .unwrap_or("[{shell}]");
    if template.contains("{shell}") {
        template.replace("{shell}", &stack)
    } else {
        template.to_string()
    }
}

#[cfg(test)]
pub(crate) fn render_prompt_template(
    template: &str,
    user: &str,
    domain: &str,
    profile: &str,
    indicator: &str,
) -> String {
    let mut out = template
        .replace("{user}", user)
        .replace("{domain}", domain)
        .replace("{profile}", profile)
        .replace("{context}", profile);

    if out.contains("{indicator}") {
        out = out.replace("{indicator}", indicator);
    } else if !indicator.trim().is_empty() {
        if !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(indicator);
    }

    out
}

fn render_prompt_template_styled(
    template: &str,
    user: &str,
    domain: &str,
    profile: &str,
    indicator: &str,
    literal_style: Option<&str>,
    color: bool,
    theme: &osp_ui::theme::ThemeDefinition,
) -> String {
    let mut out = String::new();
    let mut cursor = 0;

    let style_literal = |text: &str| {
        style_prompt_fragment(literal_style, text, StyleToken::PromptText, color, theme)
    };

    while cursor < template.len() {
        let remainder = &template[cursor..];
        let Some(open) = remainder.find('{') else {
            out.push_str(&style_literal(remainder));
            break;
        };
        let open = cursor + open;
        if open > cursor {
            out.push_str(&style_literal(&template[cursor..open]));
        }
        let tail = &template[open..];
        if let Some((replacement, consumed)) =
            prompt_placeholder_replacement(tail, user, domain, profile, indicator)
        {
            out.push_str(replacement);
            cursor = open + consumed;
            continue;
        }
        out.push_str(&style_literal("{"));
        cursor = open + 1;
    }

    if !template.contains("{indicator}") && !indicator.trim().is_empty() {
        if !out.ends_with(' ') {
            out.push_str(&style_literal(" "));
        }
        out.push_str(indicator);
    }

    out
}

fn prompt_placeholder_replacement<'a>(
    tail: &'a str,
    user: &'a str,
    domain: &'a str,
    profile: &'a str,
    indicator: &'a str,
) -> Option<(&'a str, usize)> {
    if tail.starts_with("{user}") {
        return Some((user, "{user}".len()));
    }
    if tail.starts_with("{domain}") {
        return Some((domain, "{domain}".len()));
    }
    if tail.starts_with("{profile}") {
        return Some((profile, "{profile}".len()));
    }
    if tail.starts_with("{context}") {
        return Some((profile, "{context}".len()));
    }
    if tail.starts_with("{indicator}") {
        return Some((indicator, "{indicator}".len()));
    }
    None
}

fn style_prompt_fragment(
    config_style: Option<&str>,
    value: &str,
    fallback: StyleToken,
    color: bool,
    theme: &osp_ui::theme::ThemeDefinition,
) -> String {
    match config_style.map(str::trim) {
        Some(spec) if !spec.is_empty() => apply_style_spec(value, spec, color),
        _ => apply_style_with_theme(value, fallback, color, theme),
    }
}
