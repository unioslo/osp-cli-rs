use crate::app::DebugTimingState;
use crate::app::format_timing_badge;
use crate::app::{CMD_HELP, DEFAULT_REPL_PROMPT};
use crate::repl::{ReplAppearance, ReplPrompt};
use crate::ui::chrome::{
    SectionRenderContext, SectionStyleTokens, render_section_block_with_overrides,
};
use crate::ui::render_inline;
use crate::ui::style::{
    StyleToken, apply_style_spec, apply_style_with_theme, apply_style_with_theme_overrides,
};
use std::sync::Arc;

use super::ReplViewContext;
use super::history;
use super::surface::ReplSurface;
use crate::ui::presentation::{
    ReplIntroStyle, intro_style, intro_style_with_verbosity, repl_simple_prompt,
};

pub(crate) fn render_repl_intro(view: ReplViewContext<'_>, intro_commands: &[String]) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let config = view.config;

    let user = config.get_string("user.name").unwrap_or("anonymous");
    let display_name = config
        .get_string("user.display_name")
        .or_else(|| config.get_string("user.full_name"))
        .unwrap_or(user);
    let theme_id = view.ui.render_settings.theme_name.clone();
    let version = env!("CARGO_PKG_VERSION");
    let theme_display = theme_display_name(&theme_id);
    let intro_style = intro_style_with_verbosity(intro_style(config), view.ui.message_verbosity);

    if matches!(intro_style, ReplIntroStyle::None) {
        return String::new();
    }

    if matches!(
        intro_style,
        ReplIntroStyle::Minimal | ReplIntroStyle::Compact
    ) {
        let visible_commands = intro_commands
            .iter()
            .map(|command| format!("`{command}`"))
            .collect::<Vec<_>>();
        let help_hint = if view.auth.is_builtin_visible(CMD_HELP) {
            "See `help` for more.".to_string()
        } else {
            "Use completion to explore commands.".to_string()
        };
        let command_summary = if visible_commands.is_empty() {
            help_hint
        } else {
            format!("Commands: {}. {help_hint}", visible_commands.join(", "))
        };
        let summary = format!("Welcome `{display_name}`. v{version}. {command_summary}");
        let mut out = String::new();
        out.push('\n');
        out.push_str(&render_inline(
            &summary,
            resolved.color,
            &resolved.theme,
            &resolved.style_overrides,
        ));
        out.push_str("\n\n");
        return out;
    }

    let mut out = String::new();
    out.push('\n');
    out.push_str(&render_chrome_section(
        &resolved,
        "OSP",
        &[
            format!("  Welcome `{display_name}`!"),
            format!("  Logged in as: `{user}`"),
            format!("  Theme: `{theme_display}`"),
            format!("  Version: `{version}`"),
        ],
    ));
    out.push_str("\n\n");
    out.push_str(&render_chrome_section(
        &resolved,
        "Keybindings",
        &[
            "  `Ctrl-D`    **exit**".to_string(),
            "  `Ctrl-L`    **clear screen**".to_string(),
            "  `Ctrl-R`    **reverse search history**".to_string(),
        ],
    ));
    out.push_str("\n\n");
    out.push_str(&render_chrome_section(
        &resolved,
        "Pipes",
        &[
            "  `F` key>3 *|* `P` col1 col2 *|* `S` sort_key *|* `G` group_by_k1 k2".to_string(),
            "  *|* `A` metric() *|* `L` limit offset *|* `C` count".to_string(),
            "  `K` key *|* `V` value *|* contains *|* !not *|* ?exist *|* !?not_exist *(= exact, == case-sens.)*".to_string(),
            "  *Help:* `| H` *or* `| H <verb>` *e.g.* `| H F`".to_string(),
        ],
    ));
    out.push_str("\n\n");
    out
}

pub(crate) fn render_repl_command_overview(
    view: ReplViewContext<'_>,
    surface: &ReplSurface,
) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
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
    out.push_str("  [INVOCATION_OPTIONS] COMMAND [ARGS]...\n\n");

    let body = surface
        .overview_entries
        .iter()
        .map(|entry| {
            let name = format!("{:<12}", entry.name);
            format!(
                "  {}{}",
                style_command_name(&resolved, theme, &name),
                entry.summary
            )
        })
        .collect::<Vec<_>>();
    out.push_str(&render_chrome_section(&resolved, "Commands", &body));
    out.push('\n');
    out
}

fn render_chrome_section(
    resolved: &crate::ui::ResolvedRenderSettings,
    title: &str,
    lines: &[String],
) -> String {
    let theme = &resolved.theme;
    let body = lines
        .iter()
        .map(|line| render_inline(line, resolved.color, theme, &resolved.style_overrides))
        .collect::<Vec<_>>()
        .join("\n");

    render_section_block_with_overrides(
        title,
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
    )
}

fn style_command_name(
    resolved: &crate::ui::ResolvedRenderSettings,
    theme: &crate::ui::theme::ThemeDefinition,
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

pub(crate) fn build_repl_appearance(view: ReplViewContext<'_>) -> ReplAppearance {
    let resolved = view.ui.render_settings.resolve_render_settings();
    if !resolved.color {
        return ReplAppearance::default();
    }
    let theme = &resolved.theme;
    let config = view.config;

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

pub(crate) fn build_repl_prompt(view: ReplViewContext<'_>) -> ReplPrompt {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let config = view.config;
    let theme = &resolved.theme;
    let simple = repl_simple_prompt(config);
    let profile = config.active_profile();
    let user = config.get_string("user.name").unwrap_or("anonymous");
    let domain = config.get_string("domain").unwrap_or("local");
    let indicator = build_shell_indicator(view);
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
        if indicator.trim().is_empty() {
            format!("{profile_text}{suffix}")
        } else {
            format!("{profile_text} {indicator_text}{suffix}")
        }
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
            PromptTemplateStyleContext {
                literal_style: prompt_style,
                color: resolved.color,
                theme,
            },
        )
    };

    ReplPrompt::simple(prompt)
}

pub(crate) fn build_repl_prompt_right_renderer(
    view: ReplViewContext<'_>,
    timing: DebugTimingState,
) -> crate::repl::PromptRightRenderer {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let history_enabled = history::repl_history_enabled(view.config);
    Arc::new(move || render_repl_prompt_right(&resolved, history_enabled, &timing))
}

#[cfg(test)]
pub(crate) fn render_repl_prompt_right_for_test(
    resolved: &crate::ui::ResolvedRenderSettings,
    history_enabled: bool,
    timing: &DebugTimingState,
) -> String {
    render_repl_prompt_right(resolved, history_enabled, timing)
}

fn render_repl_prompt_right(
    resolved: &crate::ui::ResolvedRenderSettings,
    history_enabled: bool,
    timing: &DebugTimingState,
) -> String {
    let mut parts = Vec::new();

    if !history_enabled {
        let incognito = if resolved.unicode {
            "(⌐■_■)"
        } else {
            "incognito"
        };
        parts.push(apply_style_with_theme_overrides(
            incognito,
            StyleToken::Muted,
            resolved.color,
            &resolved.theme,
            &resolved.style_overrides,
        ));
    }

    if let Some(badge) = timing.badge() {
        let rendered = format_timing_badge(badge.summary, badge.level, resolved);
        if !rendered.is_empty() {
            parts.push(rendered);
        }
    }

    parts.join("  ")
}

fn build_shell_indicator(view: ReplViewContext<'_>) -> String {
    let Some(stack) = view.scope.display_label() else {
        return String::new();
    };
    let template = view
        .config
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
    style: PromptTemplateStyleContext<'_>,
) -> String {
    let mut out = String::new();
    let mut cursor = 0;

    let style_literal = |text: &str| {
        style_prompt_fragment(
            style.literal_style,
            text,
            StyleToken::PromptText,
            style.color,
            style.theme,
        )
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

struct PromptTemplateStyleContext<'a> {
    literal_style: Option<&'a str>,
    color: bool,
    theme: &'a crate::ui::theme::ThemeDefinition,
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
    theme: &crate::ui::theme::ThemeDefinition,
) -> String {
    match config_style.map(str::trim) {
        Some(spec) if !spec.is_empty() => apply_style_spec(value, spec, color),
        _ => apply_style_with_theme(value, fallback, color, theme),
    }
}

#[cfg(test)]
mod tests;
