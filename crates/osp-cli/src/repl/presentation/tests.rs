use super::{
    build_repl_appearance, build_repl_prompt, render_prompt_template, render_repl_command_overview,
    render_repl_intro, render_repl_prompt_right_for_test, theme_display_name,
};
use crate::app::TimingSummary;
use crate::app::{CMD_CONFIG, CMD_HELP, CMD_PLUGINS, CMD_THEME};
use crate::plugin_manager::PluginManager;
use crate::repl::ReplViewContext;
use crate::repl::surface::{ReplOverviewEntry, ReplSurface};
use crate::state::{
    AppState, AppStateInit, DebugTimingBadge, DebugTimingState, LaunchContext, RuntimeContext,
    TerminalKind,
};
use insta::assert_snapshot;
use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
use osp_core::output::OutputFormat;
use osp_ui::RenderSettings;
use osp_ui::messages::MessageLevel;
use std::time::Duration;

fn make_state(entries: &[(&str, &str)]) -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    let mut session = ConfigLayer::default();
    for (key, value) in entries {
        session.set(*key, *value);
    }
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_session(session);
    let config = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve");

    AppState::new(AppStateInit {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        render_settings: RenderSettings::test_plain(OutputFormat::Json),
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
        plugins: PluginManager::new(Vec::new()),
        themes: crate::theme_loader::ThemeCatalog::default(),
        launch: LaunchContext::default(),
    })
}

fn repl_view<'a>(state: &'a AppState) -> ReplViewContext<'a> {
    ReplViewContext::from_parts(&state.runtime, &state.session)
}

#[test]
fn repl_intro_expressive_includes_sections_and_user_context() {
    let state = make_state(&[
        ("ui.presentation", "expressive"),
        ("user.name", "oistes"),
        ("user.display_name", "Oistes"),
        ("theme.name", "rose-pine-moon"),
    ]);

    let rendered = render_repl_intro(repl_view(&state));
    assert!(rendered.contains("OSP"));
    assert!(rendered.contains("Keybindings"));
    assert!(rendered.contains("Pipes"));
    assert!(rendered.contains("Oistes"));
    assert!(rendered.contains("oistes"));
    assert!(rendered.contains("Rose Pine Moon"));
    assert_snapshot!("repl_intro_expressive", rendered);
}

#[test]
fn repl_overview_lists_invocation_options_for_expressive_surface() {
    let state = make_state(&[("ui.presentation", "expressive")]);
    let surface = ReplSurface {
        root_words: vec![
            CMD_HELP.to_string(),
            CMD_CONFIG.to_string(),
            CMD_THEME.to_string(),
            CMD_PLUGINS.to_string(),
        ],
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: vec![
            ReplOverviewEntry {
                name: "options".to_string(),
                summary: "per invocation: --format json".to_string(),
            },
            ReplOverviewEntry {
                name: "config".to_string(),
                summary: "show config".to_string(),
            },
        ],
    };

    let rendered = render_repl_command_overview(repl_view(&state), &surface);
    assert!(rendered.contains("[INVOCATION_OPTIONS] COMMAND [ARGS]"));
    assert!(rendered.contains("options"));
    assert!(rendered.contains("--format json"));
    assert_snapshot!("repl_overview_expressive", rendered);
}

#[test]
fn repl_appearance_respects_color_toggle_and_overrides() {
    let plain_state = make_state(&[]);
    let plain = build_repl_appearance(repl_view(&plain_state));
    assert_eq!(plain.completion_text_style, None);
    assert_eq!(plain.completion_background_style, None);
    assert_eq!(plain.completion_highlight_style, None);
    assert_eq!(plain.command_highlight_style, None);

    let mut rich_state = make_state(&[
        ("color.prompt.completion.text", "red"),
        ("color.prompt.completion.background", "blue"),
        ("color.prompt.completion.highlight", "bold green"),
        ("color.prompt.command", "yellow"),
    ]);
    rich_state.runtime.ui.render_settings.mode = osp_core::output::RenderMode::Rich;
    rich_state.runtime.ui.render_settings.color = osp_core::output::ColorMode::Always;
    rich_state.runtime.ui.render_settings.unicode = osp_core::output::UnicodeMode::Always;
    rich_state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    rich_state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);

    let appearance = build_repl_appearance(repl_view(&rich_state));
    assert_eq!(appearance.completion_text_style.as_deref(), Some("red"));
    assert_eq!(
        appearance.completion_background_style.as_deref(),
        Some("blue")
    );
    assert_eq!(
        appearance.completion_highlight_style.as_deref(),
        Some("bold green")
    );
    assert_eq!(
        appearance.command_highlight_style.as_deref(),
        Some("yellow")
    );
}

#[test]
fn repl_prompt_simple_mode_omits_blank_indicator_separator() {
    let state = make_state(&[
        ("ui.presentation", "compact"),
        ("repl.shell_indicator", "   "),
    ]);

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert_eq!(prompt, "default> ");
}

#[test]
fn repl_prompt_custom_indicator_template_can_be_literal() {
    let mut state = make_state(&[("repl.shell_indicator", "scoped")]);
    state.session.scope.enter("ldap");

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert!(prompt.contains("scoped"));
    assert!(!prompt.contains("ldap /"));
}

#[test]
fn repl_prompt_right_shows_unicode_incognito_when_history_is_disabled() {
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.mode = osp_core::output::RenderMode::Rich;
    settings.color = osp_core::output::ColorMode::Always;
    settings.unicode = osp_core::output::UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.runtime.locale_utf8 = Some(true);
    let resolved = settings.resolve_render_settings();

    let rendered =
        render_repl_prompt_right_for_test(&resolved, false, &DebugTimingState::default());
    assert!(rendered.contains("(⌐■_■)"));
}

#[test]
fn repl_prompt_right_combines_incognito_and_timing_badge() {
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.mode = osp_core::output::RenderMode::Rich;
    settings.color = osp_core::output::ColorMode::Always;
    settings.unicode = osp_core::output::UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.runtime.locale_utf8 = Some(true);
    let resolved = settings.resolve_render_settings();
    let timing = DebugTimingState::default();
    timing.set(DebugTimingBadge {
        level: 1,
        summary: TimingSummary {
            total: Duration::from_millis(120),
            parse: None,
            execute: None,
            render: None,
        },
    });

    let rendered = render_repl_prompt_right_for_test(&resolved, false, &timing);
    assert!(rendered.contains("(⌐■_■)"));
    assert!(rendered.contains("120"));
}

#[test]
fn theme_display_name_falls_back_when_slug_has_no_words() {
    assert_eq!(theme_display_name("---"), "---");
    assert_eq!(theme_display_name("nord_light"), "Nord Light");
}

#[test]
fn prompt_template_preserves_unknown_placeholders_and_appends_indicator() {
    let rendered = render_prompt_template("{profile} {unknown}", "u", "d", "prod", "[ldap]");
    assert_eq!(rendered, "prod {unknown} [ldap]");
}

#[test]
fn prompt_template_replaces_context_alias_and_indicator_placeholder() {
    let rendered = render_prompt_template("{context}:{indicator}", "u", "d", "prod", "[ldap]");
    assert_eq!(rendered, "prod:[ldap]");
}

#[test]
fn repl_intro_minimal_without_help_visibility_uses_completion_hint() {
    let state = make_state(&[
        ("ui.presentation", "compact"),
        ("auth.visible.builtins", "config,theme,plugins"),
    ]);

    let rendered = render_repl_intro(repl_view(&state));
    assert!(rendered.contains("Use completion to explore commands."));
    assert!(!rendered.contains("See `help`"));
}

#[test]
fn repl_prompt_renders_custom_template_with_prompt_style() {
    let mut state = make_state(&[
        ("repl.prompt", "{user}@{domain} {indicator} {profile}> "),
        ("color.prompt.text", "green"),
    ]);
    state.session.scope.enter("ldap");
    state.runtime.ui.render_settings.mode = osp_core::output::RenderMode::Rich;
    state.runtime.ui.render_settings.color = osp_core::output::ColorMode::Always;
    state.runtime.ui.render_settings.unicode = osp_core::output::UnicodeMode::Always;
    state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert!(prompt.contains("anonymous"));
    assert!(prompt.contains("local"));
    assert!(prompt.contains("[ldap]"));
    assert!(prompt.contains("default"));
    assert!(prompt.contains(">"));
    assert!(prompt.contains("\x1b["));
}

#[test]
fn repl_simple_prompt_includes_live_shell_indicator() {
    let mut state = make_state(&[("ui.presentation", "compact")]);
    state.session.scope.enter("ldap");

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert_eq!(prompt, "default [ldap]> ");
}
