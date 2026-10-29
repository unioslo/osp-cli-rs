//! REPL prompt, intro, and presentation shaping.
//!
//! This module owns the operator-facing text and guide surfaces around the
//! REPL itself: prompt composition, intro rendering, theme-aware styling, and
//! a few small presentation-mode policies. It should shape REPL presentation,
//! not command execution.

use crate::app::DEFAULT_REPL_PROMPT;
use crate::app::DebugTimingState;
use crate::app::format_timing_badge;
use crate::app::help::help_level;
use crate::app::is_sensitive_key;
use crate::config::ConfigValue;
use crate::config::DEFAULT_REPL_HISTORY_MENU_ROWS;
use crate::config::ResolvedConfig;
use crate::core::document::{DocBlock, Document, DocumentEntry};
use crate::native::NativeSessionContext;
use crate::repl::ReplInputMode;
use crate::repl::{ReplAppearance, ReplPrompt};
use crate::ui::messages::MessageLevel;
use crate::ui::render;
use crate::ui::style::{StyleToken, apply_style_with_theme, apply_style_with_theme_overrides};
use std::borrow::Cow;
use std::sync::Arc;

use super::ReplViewContext;
use super::history;
use super::surface::ReplSurface;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplIntroStyle {
    None,
    Minimal,
    Compact,
    Full,
}

impl ReplIntroStyle {
    /// All variants in canonical configuration order.
    pub const ALL: [Self; 4] = [Self::None, Self::Minimal, Self::Compact, Self::Full];

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "minimal" | "austere" => Some(Self::Minimal),
            "compact" => Some(Self::Compact),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// Returns the canonical configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }
}

pub(crate) fn repl_simple_prompt(config: &ResolvedConfig) -> bool {
    config.get_bool("repl.simple_prompt").unwrap_or(false)
}

pub(crate) fn intro_style(config: &ResolvedConfig) -> ReplIntroStyle {
    config
        .get_string("repl.intro")
        .and_then(ReplIntroStyle::parse)
        .or_else(|| {
            config.get_bool("repl.intro").map(|enabled| {
                if enabled {
                    ReplIntroStyle::Full
                } else {
                    ReplIntroStyle::None
                }
            })
        })
        .unwrap_or(ReplIntroStyle::Full)
}

pub(crate) fn intro_style_with_verbosity(
    style: ReplIntroStyle,
    verbosity: MessageLevel,
) -> ReplIntroStyle {
    if style == ReplIntroStyle::None {
        return style;
    }
    let mut rank = match style {
        ReplIntroStyle::None => 0_i8,
        ReplIntroStyle::Minimal => 1,
        ReplIntroStyle::Compact => 2,
        ReplIntroStyle::Full => 3,
    };
    let delta = match verbosity {
        MessageLevel::Error | MessageLevel::Warning => -3,
        MessageLevel::Success => 0,
        MessageLevel::Info => 1,
        MessageLevel::Trace | MessageLevel::Evidence => 2,
    };
    rank = (rank + delta).clamp(0, 3);
    match rank {
        0 => ReplIntroStyle::None,
        1 => ReplIntroStyle::Minimal,
        2 => ReplIntroStyle::Compact,
        _ => ReplIntroStyle::Full,
    }
}

pub(crate) fn repl_input_mode(config: &ResolvedConfig) -> ReplInputMode {
    config
        .get_string("repl.input_mode")
        .and_then(ReplInputMode::parse)
        .unwrap_or(ReplInputMode::Auto)
}

pub(crate) fn repl_autosuggest(config: &ResolvedConfig) -> bool {
    config.get_bool("repl.autosuggest").unwrap_or(true)
}

pub(crate) fn repl_complete_on(config: &ResolvedConfig) -> crate::repl::ReplCompleteOn {
    config
        .get_string_list("repl.complete_on")
        .map(|values| crate::repl::ReplCompleteOn::parse(&values))
        .unwrap_or_default()
}

const DEFAULT_MINIMAL_INTRO_TEMPLATE: &str =
    "Welcome {{display_name}}. v{{version}}. Commands: {{intro.commands}}. {{help_hint}}";
const DEFAULT_COMPACT_INTRO_TEMPLATE: &str = "{{ help }}";
// The full intro is authored as markdown plus semantic `osp` blocks. Prose
// stays as paragraphs, but structured sections like keybindings and pipes flow
// through the same data/document path as help and other guide output.
const DEFAULT_FULL_INTRO_TEMPLATE: &str = r#"## OSP
Welcome `{{display_name}}`!
{{session.user_label}}: `{{session.user_value}}`
Theme: `{{theme_display}}`

## Keybindings
`Ctrl-D`    exit
`Ctrl-L`    clear screen
`Ctrl-R`    reverse search history

## Pipes
`text` search | `!text` exclude | `K key` | `V value` | `=exact` | `==case-sensitive`
`F key>3` | `P col1 col2` | `S sort_key` | `G key` | `A count()` | `L 10` | `C`
Help: `| H` or `| H <verb>` e.g. `| H F`

{{ help }}"#;

pub(crate) fn render_repl_intro(view: ReplViewContext<'_>, surface: &ReplSurface) -> String {
    let _phase = tracing::info_span!(target: "osp_startup", "REPL intro rendering").entered();
    let intro_style = intro_style_with_verbosity(intro_style(view.config), view.ui.verbosity);
    let guide = build_repl_intro_payload(view, surface, Some(intro_style));
    let decoration = intro_decoration(view);
    let mut rendered = render(
        &repl_document_output(&guide),
        &crate::ui::RenderRequest {
            leading_decoration: Some(&decoration),
            layout: Some(crate::ui::help_layout_from_config(view.config)),
            ..crate::ui::RenderRequest::new(&view.ui.render_settings)
        },
    );
    if !rendered.is_empty() {
        // One blank row above the intro, so neither text nor artwork touches the top edge.
        rendered.insert(0, '\n');
        rendered.push('\n');
    }
    rendered
}

pub(crate) fn intro_decoration(view: ReplViewContext<'_>) -> Vec<String> {
    if matches!(
        intro_style_with_verbosity(intro_style(view.config), view.ui.verbosity),
        ReplIntroStyle::Compact | ReplIntroStyle::Full
    ) && let Some(render) = view.intro_decoration
    {
        render(&view.ui.render_settings.resolve_render_settings())
    } else {
        Vec::new()
    }
}

pub(crate) fn repl_document_output(document: &Document) -> crate::core::output_model::OutputResult {
    let mut output = document.to_output_result();
    output.meta.document_entry_gap = Some(5);
    output
}

pub(crate) fn build_repl_intro_payload(
    view: ReplViewContext<'_>,
    surface: &ReplSurface,
    override_style: Option<ReplIntroStyle>,
) -> Document {
    let config = view.config;
    let intro_style = override_style
        .unwrap_or_else(|| intro_style_with_verbosity(intro_style(config), view.ui.verbosity));

    if matches!(intro_style, ReplIntroStyle::None) {
        return Document::default();
    }
    let template = intro_template_text(
        intro_template(view.config, intro_style),
        view.ui.render_settings.resolve_render_settings().unicode,
    );
    let expanded = expand_intro_template(view, &surface.intro_commands, &template);
    parse_intro_template_payload(
        &expanded,
        &build_repl_command_overview_view(surface).filtered_for_help_level(help_level(
            view.config,
            0,
            0,
        )),
    )
}

/// Normalize authored punctuation before expanding identity placeholders.
fn intro_template_text(template: &str, unicode: bool) -> Cow<'_, str> {
    if unicode {
        Cow::Borrowed(template)
    } else {
        Cow::Owned(
            template
                .replace('→', "->")
                .replace('·', "/")
                .replace(['‘', '’'], "'")
                .replace(['“', '”'], "\""),
        )
    }
}

fn intro_template(config: &crate::config::ResolvedConfig, style: ReplIntroStyle) -> &str {
    match style {
        ReplIntroStyle::None => "",
        ReplIntroStyle::Minimal => config
            .get_string("repl.intro_template.minimal")
            .unwrap_or(DEFAULT_MINIMAL_INTRO_TEMPLATE),
        ReplIntroStyle::Compact => config
            .get_string("repl.intro_template.compact")
            .unwrap_or(DEFAULT_COMPACT_INTRO_TEMPLATE),
        ReplIntroStyle::Full => config
            .get_string("repl.intro_template.full")
            .unwrap_or(DEFAULT_FULL_INTRO_TEMPLATE),
    }
}

fn parse_intro_template_payload(template: &str, help: &Document) -> Document {
    Document::from_markdown_template(template, help)
}

/// Picks one entry from `repl.intro_tips`, the same one all day (UTC).
fn tip_of_the_day(view: ReplViewContext<'_>, intro_commands: &[String]) -> String {
    let unicode = view.ui.render_settings.resolve_render_settings().unicode;
    let tips = view
        .config
        .get_string_list("repl.intro_tips")
        .unwrap_or_default()
        .into_iter()
        .map(|tip| {
            let tip = intro_template_text(&tip, unicode);
            expand_intro_template(view, intro_commands, &tip).into_owned()
        })
        .filter(|tip| !tip.trim().is_empty())
        .collect::<Vec<_>>();
    if tips.is_empty() {
        return String::new();
    }
    let day = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() / 86_400)
        .unwrap_or(0);
    tips[(day % tips.len() as u64) as usize].clone()
}

fn expand_intro_template<'a>(
    view: ReplViewContext<'_>,
    intro_commands: &[String],
    template: &'a str,
) -> Cow<'a, str> {
    let mut out = String::new();
    let mut cursor = 0;

    while let Some(open_rel) = template[cursor..].find("{{") {
        let open = cursor + open_rel;
        out.push_str(&template[cursor..open]);
        let tail = &template[open + 2..];
        let Some(close_rel) = tail.find("}}") else {
            out.push_str(&template[open..]);
            return Cow::Owned(out);
        };
        let close = open + 2 + close_rel;
        let key = template[open + 2..close].trim();
        if let Some(command) = key.strip_prefix("#visible ")
            && let Some(end_rel) = template[close + 2..].find("{{/visible}}")
        {
            let end = close + 2 + end_rel;
            let path = crate::core::command_policy::CommandPath::new(command.split_whitespace());
            if view.auth.external_command_path_access(&path).is_visible() {
                out.push_str(&expand_intro_template(
                    view,
                    intro_commands,
                    &template[close + 2..end],
                ));
            }
            cursor = end + "{{/visible}}".len();
            continue;
        }
        if key.is_empty() {
            out.push_str("{{}}");
            cursor = close + 2;
            continue;
        }
        out.push_str(&resolve_intro_placeholder(view, intro_commands, key));
        cursor = close + 2;
    }

    if cursor == 0 {
        Cow::Borrowed(template)
    } else {
        out.push_str(&template[cursor..]);
        Cow::Owned(out)
    }
}

fn effective_user(view: ReplViewContext<'_>) -> &str {
    view.context
        .authenticated_subject()
        .or_else(|| view.config.get_string("user.name"))
        .unwrap_or("anonymous")
}

fn resolve_intro_placeholder(
    view: ReplViewContext<'_>,
    intro_commands: &[String],
    key: &str,
) -> String {
    match key {
        "actions" => {
            return view
                .actions
                .iter()
                .filter_map(|action| {
                    action
                        .key
                        .map(|key| format!("`{key}` {}", action.label.to_ascii_lowercase()))
                })
                .collect::<Vec<_>>()
                .join(" / ");
        }
        "help" => return "{{ help }}".to_string(),
        "overview" => return "{{ overview }}".to_string(),
        "user" | "user.name" => {
            return effective_user(view).to_string();
        }
        "display_name" => {
            let user = effective_user(view);
            return view
                .context
                .display_name()
                .or_else(|| {
                    view.config
                        .get_string(&format!("user.display_names.{user}"))
                })
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(user)
                .to_string();
        }
        "session.login" => {
            let facts = view.auth.session_facts();
            if facts.selected_credentials.is_empty() {
                return String::new();
            }
            let (ready, missing): (Vec<_>, Vec<_>) =
                facts.selected_credentials.iter().partition(|selected| {
                    facts
                        .credentials
                        .get(&selected.name)
                        .is_some_and(|state| state.valid)
                });
            let value = if missing.is_empty() {
                effective_user(view).to_string()
            } else if ready.is_empty() {
                "Not signed in".to_string()
            } else {
                format!(
                    "{} ({}; not signed in: {})",
                    effective_user(view),
                    ready
                        .into_iter()
                        .map(|selected| selected.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    missing
                        .into_iter()
                        .map(|selected| selected.label.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return format!("Logged in as  `{value}`\n");
        }
        "session.user_label" => {
            return if view.auth.session_facts().authenticated {
                "Logged in as"
            } else {
                "User"
            }
            .to_string();
        }
        "session.user_value" => {
            return if view.auth.session_facts().authenticated {
                effective_user(view)
            } else {
                "Not signed in"
            }
            .to_string();
        }
        "profile" => return view.config.active_profile().to_string(),
        "profile.active" => return view.config.active_profile().to_string(),
        "domain" => {
            return view
                .config
                .get_string("domain")
                .unwrap_or("local")
                .to_string();
        }
        "theme" | "theme.name" => return view.ui.render_settings.theme_name.clone(),
        "theme_display" => {
            return view
                .themes
                .resolve(&view.ui.render_settings.theme_name)
                .map(|entry| entry.theme.display_name().to_string())
                .unwrap_or_else(|| theme_display_name(&view.ui.render_settings.theme_name));
        }
        "version" => return env!("CARGO_PKG_VERSION").to_string(),
        "tip" => return tip_of_the_day(view, intro_commands),
        "intro.commands" => {
            return intro_commands
                .iter()
                .map(|command| command.to_string())
                .collect::<Vec<_>>()
                .join(", ");
        }
        "help_hint" => {
            return "See help for more.".to_string();
        }
        _ => {}
    }

    if is_sensitive_key(key) {
        return format!("{{{{{key}}}}}");
    }

    match view.config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::String(value)) => value.clone(),
        Some(ConfigValue::Bool(value)) => value.to_string(),
        Some(ConfigValue::Integer(value)) => value.to_string(),
        Some(ConfigValue::Float(value)) => value.to_string(),
        Some(ConfigValue::List(values)) => values
            .iter()
            .filter_map(|value| match value {
                ConfigValue::String(value) => Some(value.clone()),
                ConfigValue::Bool(value) => Some(value.to_string()),
                ConfigValue::Integer(value) => Some(value.to_string()),
                ConfigValue::Float(value) => Some(value.to_string()),
                ConfigValue::List(_) | ConfigValue::Secret(_) => None,
            })
            .collect::<Vec<_>>()
            .join(", "),
        Some(ConfigValue::Secret(_)) | None => format!("{{{{{key}}}}}"),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn render_repl_command_overview(
    view: ReplViewContext<'_>,
    surface: &ReplSurface,
) -> String {
    let guide = build_repl_help_view(surface, help_level(view.config, 0, 0));
    render(
        &repl_document_output(&guide),
        &crate::ui::RenderRequest {
            layout: Some(crate::ui::help_layout_from_config(view.config)),
            ..crate::ui::RenderRequest::new(&view.ui.render_settings)
        },
    )
}

pub(crate) fn build_repl_command_overview_view(surface: &ReplSurface) -> Document {
    Document {
        blocks: vec![
            DocBlock::section(
                "Usage",
                vec![DocBlock::Paragraph {
                    text: "[INVOCATION_OPTIONS] COMMAND [ARGS]...".into(),
                }],
            )
            .with_role(crate::core::document::SectionRole::Usage),
            DocBlock::section(
                "Commands",
                vec![DocBlock::Entries {
                    entries: surface
                        .overview_entries
                        .iter()
                        .map(|entry| DocumentEntry {
                            role: crate::core::document::EntryRole::Command,
                            name: entry.name.clone(),
                            description: entry.summary.clone(),
                        })
                        .collect(),
                }],
            ),
            DocBlock::Paragraph {
                text: "Pipe command output with `|`; use `| H` for in-band DSL help.".into(),
            },
        ],
    }
}

pub(crate) fn build_repl_help_view(
    surface: &ReplSurface,
    level: crate::core::document::HelpLevel,
) -> Document {
    let mut document = build_repl_command_overview_view(surface);
    crate::cli::invocation::extend_with_invocation_help(&mut document, level);
    document.filtered_for_help_level(level)
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
    let history_menu_rows = match view
        .config
        .get("repl.history.menu_rows")
        .map(ConfigValue::reveal)
    {
        Some(ConfigValue::Integer(value)) => (*value).clamp(1, u16::MAX as i64) as u16,
        _ => DEFAULT_REPL_HISTORY_MENU_ROWS as u16,
    };

    let appearance = ReplAppearance::builder()
        .with_ansi_colors(resolved.color)
        .with_command_prefixes(
            crate::command_words::CommandPrefixes::from_config(view.config).enabled(true),
        )
        .with_command_hint_limit(
            view.config
                .get("repl.hint.command_limit")
                .and_then(|value| match value.reveal() {
                    ConfigValue::Integer(value) => usize::try_from((*value).max(0)).ok(),
                    _ => None,
                })
                .unwrap_or(3),
        )
        .with_history_menu_rows(history_menu_rows);
    if !resolved.color {
        return appearance.build();
    }

    let theme = &resolved.theme;
    let completion_text_style = theme.repl_completion_text_spec().to_string();
    let completion_background_style = theme.repl_completion_background_spec().to_string();
    let completion_highlight_style = theme.repl_completion_highlight_spec().to_string();
    let command_highlight_style = theme.palette.success.to_string();

    appearance
        .with_completion_text_style(Some(completion_text_style))
        .with_completion_background_style(Some(completion_background_style))
        .with_completion_highlight_style(Some(completion_highlight_style))
        .with_command_highlight_style(Some(command_highlight_style))
        .with_error_highlight_style(Some(theme.palette.error.to_string()))
        .with_hint_style(Some(theme.palette.muted.to_string()))
        .build()
}

struct ReplPromptState {
    simple: bool,
    profile: String,
    user: String,
    domain: String,
    indicator: String,
}

impl ReplPromptState {
    fn from_view(view: ReplViewContext<'_>) -> Self {
        Self {
            simple: repl_simple_prompt(view.config),
            profile: view.config.active_profile().to_string(),
            user: effective_user(view).to_string(),
            domain: view
                .config
                .get_string("domain")
                .unwrap_or("local")
                .to_string(),
            indicator: build_shell_indicator(view),
        }
    }
}

struct ReplPromptRightState {
    context: String,
    incognito: String,
    timing: String,
}

pub(crate) fn build_repl_prompt(view: ReplViewContext<'_>) -> ReplPrompt {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let config = view.config;
    let theme = &resolved.theme;
    let prompt = ReplPromptState::from_view(view);
    let user_text =
        apply_style_with_theme(&prompt.user, StyleToken::PromptText, resolved.color, theme);
    let domain_text = apply_style_with_theme(
        &prompt.domain,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let profile_text = apply_style_with_theme(
        &prompt.profile,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let indicator_text = apply_style_with_theme(
        &prompt.indicator,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );

    let prompt = if prompt.simple {
        let suffix = apply_style_with_theme("> ", StyleToken::PromptText, resolved.color, theme);
        if prompt.indicator.trim().is_empty() {
            format!("{profile_text}{suffix}")
        } else {
            format!("{profile_text} {indicator_text}{suffix}")
        }
    } else {
        let template = decode_repl_prompt_template(
            config
                .get_string("repl.prompt")
                .unwrap_or(DEFAULT_REPL_PROMPT),
        );
        let template = if resolved.unicode {
            template
        } else {
            template.replace(['╭', '╰'], "+").replace('─', "-")
        };
        render_prompt_template_styled(
            &template,
            &user_text,
            &domain_text,
            &profile_text,
            &indicator_text,
            PromptTemplateStyleContext {
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
    product: Option<super::product::ReplStatusRenderer>,
) -> crate::repl::PromptRightRenderer {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let history_enabled = history::repl_history_enabled(view.config)
        && view.history_enabled
        && view.config.get_string("repl.history.path") != Some("");
    let native_context = view.native_context.clone();
    let left_width = crate::ui::display_width(&build_repl_prompt(view).left);
    Arc::new(move || {
        let base = render_repl_prompt_right(&resolved, history_enabled, &timing, &native_context);
        let status = product.as_ref().and_then(|render| render());
        let emphasis = status.as_ref().map(|status| status.level);
        let text = status
            .as_ref()
            .map(|status| status.text.as_str())
            .unwrap_or_default();
        let status = crate::ui::sanitize_human_text(text).replace('\n', " ");
        let token = match emphasis {
            Some(MessageLevel::Error) => Some(StyleToken::Error),
            Some(MessageLevel::Warning) => Some(StyleToken::Warning),
            _ => None,
        };
        let status = token.map_or_else(
            || status.clone(),
            |token| {
                apply_style_with_theme_overrides(
                    &status,
                    token,
                    resolved.color,
                    &resolved.theme,
                    &resolved.style_overrides,
                )
            },
        );
        if status.is_empty() {
            if left_width + crate::ui::display_width(&base) + 3 > resolved.width.unwrap_or(80)
                && !native_context.context_entries().is_empty()
            {
                format!("+{} ctx", native_context.context_entries().len())
            } else {
                base
            }
        } else {
            let mut context = render_repl_prompt_context(&resolved, &native_context);
            let incognito = render_repl_prompt_incognito(&resolved, history_enabled);
            if left_width
                + crate::ui::display_width(&context)
                + crate::ui::display_width(&incognito)
                + crate::ui::display_width(&status)
                + 5
                > resolved.width.unwrap_or(80)
            {
                context = format!("+{} ctx", native_context.context_entries().len());
            }
            let mut parts = [context, incognito, status]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let timing = render_repl_prompt_timing(&resolved, &timing);
            // Attention wins the fixed prompt budget; Reedline handles draft collisions.
            if !timing.is_empty()
                && left_width
                    + crate::ui::display_width(&parts.join("  "))
                    + crate::ui::display_width(&timing)
                    + 3
                    <= resolved.width.unwrap_or(80)
            {
                parts.push(timing);
            }
            parts.join("  ")
        }
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn render_repl_prompt_right_for_test(
    resolved: &crate::ui::ResolvedRenderSettings,
    history_enabled: bool,
    timing: &DebugTimingState,
) -> String {
    render_repl_prompt_right_for_test_with_context(
        resolved,
        history_enabled,
        timing,
        &NativeSessionContext::default(),
    )
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn render_repl_prompt_right_for_test_with_context(
    resolved: &crate::ui::ResolvedRenderSettings,
    history_enabled: bool,
    timing: &DebugTimingState,
    native_context: &NativeSessionContext,
) -> String {
    render_repl_prompt_right(resolved, history_enabled, timing, native_context)
}

fn render_repl_prompt_right(
    resolved: &crate::ui::ResolvedRenderSettings,
    history_enabled: bool,
    timing: &DebugTimingState,
    native_context: &NativeSessionContext,
) -> String {
    let state = ReplPromptRightState {
        context: render_repl_prompt_context(resolved, native_context),
        incognito: render_repl_prompt_incognito(resolved, history_enabled),
        timing: render_repl_prompt_timing(resolved, timing),
    };

    let mut parts = Vec::new();
    if !state.context.is_empty() {
        parts.push(state.context);
    }
    if !state.incognito.is_empty() {
        parts.push(state.incognito);
    }
    if !state.timing.is_empty() {
        parts.push(state.timing);
    }
    parts.join("  ")
}

fn render_repl_prompt_context(
    resolved: &crate::ui::ResolvedRenderSettings,
    native_context: &NativeSessionContext,
) -> String {
    let entries = native_context.prompt_entries();
    if entries.is_empty() {
        return String::new();
    }
    let rendered = entries
        .iter()
        .map(|entry| NativeSessionContext::render_context_token(&entry.label, &entry.value))
        .collect::<Vec<_>>()
        .join(" ");
    apply_style_with_theme_overrides(
        &rendered,
        StyleToken::Muted,
        resolved.color,
        &resolved.theme,
        &resolved.style_overrides,
    )
}

fn render_repl_prompt_incognito(
    resolved: &crate::ui::ResolvedRenderSettings,
    history_enabled: bool,
) -> String {
    if history_enabled {
        return String::new();
    }

    let incognito = if resolved.unicode {
        "(⌐■_■)"
    } else {
        "incognito"
    };
    apply_style_with_theme_overrides(
        incognito,
        StyleToken::Muted,
        resolved.color,
        &resolved.theme,
        &resolved.style_overrides,
    )
}

fn render_repl_prompt_timing(
    resolved: &crate::ui::ResolvedRenderSettings,
    timing: &DebugTimingState,
) -> String {
    timing
        .badge()
        .map(|badge| format_timing_badge(badge.summary, badge.level, resolved))
        .filter(|rendered| !rendered.is_empty())
        .unwrap_or_default()
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
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn render_prompt_template(
    template: &str,
    user: &str,
    domain: &str,
    profile: &str,
    indicator: &str,
) -> String {
    render_prompt_template_styled(
        &decode_repl_prompt_template(template),
        user,
        domain,
        profile,
        indicator,
        PromptTemplateStyleContext {
            color: false,
            theme: &crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
        },
    )
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

    let style_literal =
        |text: &str| apply_style_with_theme(text, StyleToken::PromptText, style.color, style.theme);

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

fn decode_repl_prompt_template(template: &str) -> String {
    template.replace("\\n", "\n")
}

struct PromptTemplateStyleContext<'a> {
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;
