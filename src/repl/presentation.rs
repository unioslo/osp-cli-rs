use crate::app::DebugTimingState;
use crate::app::format_timing_badge;
use crate::app::help::help_level;
use crate::app::is_sensitive_key;
use crate::app::{CMD_HELP, DEFAULT_REPL_PROMPT};
use crate::config::ConfigValue;
use crate::config::DEFAULT_REPL_HISTORY_MENU_ROWS;
use crate::config::ResolvedConfig;
use crate::guide::template::{GuideTemplateBlock, GuideTemplateInclude, parse_markdown_template};
use crate::guide::{GuideSection, GuideSectionKind, GuideView};
use crate::repl::{ReplAppearance, ReplPrompt};
use crate::ui::messages::MessageLevel;
use crate::ui::render_structured_output_with_source_guide;
use crate::ui::style::{
    StyleToken, apply_style_spec, apply_style_with_theme, apply_style_with_theme_overrides,
};
use serde_json::Value;
use std::borrow::Cow;
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplInputMode {
    Auto,
    Interactive,
    Basic,
}

impl ReplIntroStyle {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "minimal" | "austere" => Some(Self::Minimal),
            "compact" => Some(Self::Compact),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

impl ReplInputMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "interactive" | "full" => Some(Self::Interactive),
            "basic" | "plain" => Some(Self::Basic),
            _ => None,
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
        MessageLevel::Trace => 2,
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

const DEFAULT_MINIMAL_INTRO_TEMPLATE: &str =
    "Welcome {{display_name}}. v{{version}}. Commands: {{intro.commands}}. {{help_hint}}";
const DEFAULT_COMPACT_INTRO_TEMPLATE: &str = "{{ help }}";
// The full intro is authored as markdown plus semantic `osp` blocks. Prose
// stays as paragraphs, but structured sections like keybindings and pipes flow
// through the same data/document path as help and other guide output.
const DEFAULT_FULL_INTRO_TEMPLATE: &str = r#"## OSP
Welcome `{{display_name}}`!

```osp
[
  {"name": "Logged in as", "short_help": "{{user.name}}"},
  {"name": "Theme", "short_help": "{{theme_display}}"},
  {"name": "Version", "short_help": "{{version}}"}
]
```

## Keybindings
```osp
[
  {"name": "Ctrl-D", "short_help": "exit"},
  {"name": "Ctrl-L", "short_help": "clear screen"},
  {"name": "Ctrl-R", "short_help": "reverse search history"}
]
```

## Pipes
```osp
[
  "`F` key>3",
  "`P` col1 col2",
  "`S` sort_key",
  "`G` group_by_k1 k2",
  "`A` metric()",
  "`L` limit offset",
  "`C` count",
  "`K` key-only quick search",
  "`V` value-only quick search",
  "`contains` quick-search text",
  "`!not` negate a quick match",
  "`?exist` truthy / exists",
  "`!?not_exist` missing / falsy",
  "`= exact` exact match (ci)",
  "`== case-sens.` exact match (cs)",
  "`| H <verb>` verb help, e.g. `| H F`"
]
```

{{ help }}"#;

pub(crate) fn render_repl_intro(view: ReplViewContext<'_>, surface: &ReplSurface) -> String {
    let intro_style =
        intro_style_with_verbosity(intro_style(view.config), view.ui.message_verbosity);
    let guide = build_repl_intro_payload(view, surface, Some(intro_style));
    let mut rendered = render_structured_output_with_source_guide(
        &guide.to_output_result(),
        Some(&guide),
        &view.ui.render_settings,
        crate::ui::help_layout_from_config(view.config),
    );
    if !rendered.is_empty() {
        rendered.insert(0, '\n');
        rendered.push('\n');
    }
    rendered
}

pub(crate) fn build_repl_intro_payload(
    view: ReplViewContext<'_>,
    surface: &ReplSurface,
    override_style: Option<ReplIntroStyle>,
) -> GuideView {
    let config = view.config;
    let intro_style = intro_style_with_verbosity(
        override_style.unwrap_or_else(|| intro_style(config)),
        view.ui.message_verbosity,
    );

    if matches!(intro_style, ReplIntroStyle::None) {
        return GuideView::default();
    }
    let template = intro_template(view.config, intro_style);
    let expanded = expand_intro_template(view, &surface.intro_commands, template);
    parse_intro_template_payload(
        &expanded,
        &build_repl_command_overview_view(surface).filtered_for_help_level(help_level(
            view.config,
            0,
            0,
        )),
    )
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

// Intro templates are authored like lightweight markdown, but the payload they
// produce must stay semantic. Headings create explicit sections, `osp` fences
// attach semantic data to the current section, and `{{ help }}` expands into
// canonical help sections in template order instead of being re-derived later.
fn parse_intro_template_payload(template: &str, help: &GuideView) -> GuideView {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return GuideView::default();
    }

    let mut payload = GuideView::default();
    let mut current_section: Option<GuideSection> = None;

    for block in parse_markdown_template(trimmed) {
        match block {
            GuideTemplateBlock::Heading(title) => {
                flush_intro_section(&mut payload, &mut current_section);
                current_section = Some(GuideSection::new(title, GuideSectionKind::Custom));
            }
            GuideTemplateBlock::Include(GuideTemplateInclude::Help)
            | GuideTemplateBlock::Include(GuideTemplateInclude::Overview) => {
                flush_intro_section(&mut payload, &mut current_section);
                append_template_include_payload(&mut payload, help);
            }
            GuideTemplateBlock::Paragraph(line) => {
                let lines = line
                    .lines()
                    .map(str::trim_end)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                if let Some(section) = current_section.as_mut() {
                    section
                        .paragraphs
                        .extend(lines.into_iter().map(|line| format!("  {line}")));
                } else {
                    payload.preamble.extend(lines);
                }
            }
            GuideTemplateBlock::Data(data) => {
                // Fenced `osp` blocks attach semantic data to the current
                // section so intro authoring can opt into list/key-value/grid
                // rendering without hardcoding presentation cases here.
                let section = current_section
                    .get_or_insert_with(|| GuideSection::new("", GuideSectionKind::Custom));
                attach_intro_data_block(section, data);
            }
        }
    }

    flush_intro_section(&mut payload, &mut current_section);
    payload
}

fn flush_intro_section(payload: &mut GuideView, current: &mut Option<GuideSection>) {
    let Some(section) = current.take() else {
        return;
    };
    if intro_section_has_content(&section) {
        payload.sections.push(section);
    }
}

fn intro_section_has_content(section: &GuideSection) -> bool {
    !section.paragraphs.is_empty()
        || !section.entries.is_empty()
        || !matches!(section.data, None | Some(Value::Null))
}

fn attach_intro_data_block(section: &mut GuideSection, data: Value) {
    section.data = Some(match section.data.take() {
        None => data,
        // Keep repeated data fences inside the authored section instead of
        // inventing sibling sections during parsing.
        Some(existing) => Value::Array(vec![existing, data]),
    });
}

// Help/overview includes are expanded at parse time, in authored order. This
// is the boundary where `{{ help }}` / `{{ overview }}` stop being placeholders
// and become concrete builtin sections followed by any surrounding custom
// sections, so later quick/filter/render code does not reorder them.
fn append_template_include_payload(payload: &mut GuideView, included: &GuideView) {
    payload.preamble.extend(included.preamble.iter().cloned());

    if !included.usage.is_empty() {
        payload.sections.push(GuideSection {
            title: "Usage".to_string(),
            kind: GuideSectionKind::Usage,
            paragraphs: repl_intro_body_paragraphs(&included.usage),
            entries: Vec::new(),
            data: None,
        });
    }
    if !included.commands.is_empty() {
        payload.sections.push(GuideSection {
            title: "Commands".to_string(),
            kind: GuideSectionKind::Commands,
            paragraphs: Vec::new(),
            entries: included.commands.clone(),
            data: None,
        });
    }
    if !included.arguments.is_empty() {
        payload.sections.push(GuideSection {
            title: "Arguments".to_string(),
            kind: GuideSectionKind::Arguments,
            paragraphs: Vec::new(),
            entries: included.arguments.clone(),
            data: None,
        });
    }
    if !included.options.is_empty() {
        payload.sections.push(GuideSection {
            title: "Options".to_string(),
            kind: GuideSectionKind::Options,
            paragraphs: Vec::new(),
            entries: included.options.clone(),
            data: None,
        });
    }
    if !included.common_invocation_options.is_empty() {
        payload.sections.push(GuideSection {
            title: "Common Invocation Options".to_string(),
            kind: GuideSectionKind::CommonInvocationOptions,
            paragraphs: Vec::new(),
            entries: included.common_invocation_options.clone(),
            data: None,
        });
    }
    if !included.notes.is_empty() {
        payload.sections.push(GuideSection {
            title: "Notes".to_string(),
            kind: GuideSectionKind::Notes,
            paragraphs: repl_intro_body_paragraphs(&included.notes),
            entries: Vec::new(),
            data: None,
        });
    }

    payload.sections.extend(included.sections.iter().cloned());
    payload.epilogue.extend(included.epilogue.iter().cloned());
}

fn repl_intro_body_paragraphs(paragraphs: &[String]) -> Vec<String> {
    paragraphs
        .iter()
        .map(|line| format!("  {}", line.trim_start()))
        .collect()
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

fn resolve_intro_placeholder(
    view: ReplViewContext<'_>,
    intro_commands: &[String],
    key: &str,
) -> String {
    match key {
        "help" => return "{{ help }}".to_string(),
        "overview" => return "{{ overview }}".to_string(),
        "user" => {
            return view
                .config
                .get_string("user.name")
                .unwrap_or("anonymous")
                .to_string();
        }
        "user.name" => {
            return view
                .config
                .get_string("user.name")
                .unwrap_or("anonymous")
                .to_string();
        }
        "display_name" => {
            return view
                .config
                .get_string("user.display_name")
                .or_else(|| view.config.get_string("user.full_name"))
                .or_else(|| view.config.get_string("user.name"))
                .unwrap_or("anonymous")
                .to_string();
        }
        "user.display_name" | "user.full_name" => {
            return view
                .config
                .get_string("user.display_name")
                .or_else(|| view.config.get_string("user.full_name"))
                .or_else(|| view.config.get_string("user.name"))
                .unwrap_or("anonymous")
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
        "theme_display" => return theme_display_name(&view.ui.render_settings.theme_name),
        "version" => return env!("CARGO_PKG_VERSION").to_string(),
        "intro.commands" => {
            return intro_commands
                .iter()
                .map(|command| command.to_string())
                .collect::<Vec<_>>()
                .join(", ");
        }
        "help_hint" => {
            return if view.auth.is_builtin_visible(CMD_HELP) {
                "See help for more.".to_string()
            } else {
                "Use completion to explore commands.".to_string()
            };
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
pub(crate) fn render_repl_command_overview(
    view: ReplViewContext<'_>,
    surface: &ReplSurface,
) -> String {
    let guide = build_repl_command_overview_view(surface).filtered_for_help_level(help_level(
        view.config,
        0,
        0,
    ));
    render_structured_output_with_source_guide(
        &guide.to_output_result(),
        Some(&guide),
        &view.ui.render_settings,
        crate::ui::help_layout_from_config(view.config),
    )
}

pub(crate) fn build_repl_command_overview_view(surface: &ReplSurface) -> GuideView {
    let name_width = surface
        .overview_entries
        .iter()
        .map(|entry| UnicodeWidthStr::width(entry.name.as_str()))
        .max()
        .unwrap_or(0);
    GuideView {
        usage: vec!["[INVOCATION_OPTIONS] COMMAND [ARGS]...".to_string()],
        commands: surface
            .overview_entries
            .iter()
            .map(|entry| crate::guide::GuideEntry {
                name: entry.name.clone(),
                short_help: entry.summary.clone(),
                display_indent: Some("  ".to_string()),
                display_gap: Some(format!(
                    "{}     ",
                    " ".repeat(
                        name_width.saturating_sub(UnicodeWidthStr::width(entry.name.as_str()))
                    )
                )),
            })
            .collect(),
        ..GuideView::default()
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
    let history_menu_rows = match config
        .get("repl.history.menu_rows")
        .map(ConfigValue::reveal)
    {
        Some(ConfigValue::Integer(value)) => (*value).clamp(1, u16::MAX as i64) as u16,
        _ => DEFAULT_REPL_HISTORY_MENU_ROWS as u16,
    };

    ReplAppearance::builder()
        .with_completion_text_style(Some(completion_text_style))
        .with_completion_background_style(Some(completion_background_style))
        .with_completion_highlight_style(Some(completion_highlight_style))
        .with_command_highlight_style(Some(command_highlight_style))
        .with_history_menu_rows(history_menu_rows)
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
            user: view
                .config
                .get_string("user.name")
                .unwrap_or("anonymous")
                .to_string(),
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
    incognito: String,
    timing: String,
}

pub(crate) fn build_repl_prompt(view: ReplViewContext<'_>) -> ReplPrompt {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let config = view.config;
    let theme = &resolved.theme;
    let prompt = ReplPromptState::from_view(view);
    let prompt_style = config.get_string("color.prompt.text");

    let user_text = style_prompt_fragment(
        prompt_style,
        &prompt.user,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let domain_text = style_prompt_fragment(
        prompt_style,
        &prompt.domain,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let profile_text = style_prompt_fragment(
        prompt_style,
        &prompt.profile,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let indicator_text = style_prompt_fragment(
        prompt_style,
        &prompt.indicator,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );

    let prompt = if prompt.simple {
        let suffix = style_prompt_fragment(
            prompt_style,
            "> ",
            StyleToken::PromptText,
            resolved.color,
            theme,
        );
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
        render_prompt_template_styled(
            &template,
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
    let prompt_right_template = view
        .config
        .get_string("repl.prompt_right")
        .map(str::to_string);
    let history_enabled = history::repl_history_enabled(view.config);
    Arc::new(move || {
        render_repl_prompt_right(
            &resolved,
            prompt_right_template.as_deref(),
            history_enabled,
            &timing,
        )
    })
}

#[cfg(test)]
pub(crate) fn render_repl_prompt_right_for_test(
    resolved: &crate::ui::ResolvedRenderSettings,
    prompt_right_template: Option<&str>,
    history_enabled: bool,
    timing: &DebugTimingState,
) -> String {
    render_repl_prompt_right(resolved, prompt_right_template, history_enabled, timing)
}

fn render_repl_prompt_right(
    resolved: &crate::ui::ResolvedRenderSettings,
    prompt_right_template: Option<&str>,
    history_enabled: bool,
    timing: &DebugTimingState,
) -> String {
    let state = ReplPromptRightState {
        incognito: render_repl_prompt_incognito(resolved, history_enabled),
        timing: render_repl_prompt_timing(resolved, timing),
    };

    if let Some(template) = prompt_right_template {
        return render_repl_prompt_right_template(
            &decode_repl_prompt_template(template),
            &state.incognito,
            &state.timing,
        );
    }

    let mut parts = Vec::new();
    if !state.incognito.is_empty() {
        parts.push(state.incognito);
    }
    if !state.timing.is_empty() {
        parts.push(state.timing);
    }
    parts.join("  ")
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

fn render_repl_prompt_right_template(template: &str, incognito: &str, timing: &str) -> String {
    let mut out = template.replace("{incognito}", incognito);
    out = out.replace("{timing}", timing);
    out
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
    let mut out = decode_repl_prompt_template(template)
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

fn decode_repl_prompt_template(template: &str) -> String {
    template.replace("\\n", "\n")
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
