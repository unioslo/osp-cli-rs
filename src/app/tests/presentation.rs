use super::*;
use crate::repl::surface::ReplSurface;
use insta::assert_snapshot;

fn intro_commands(commands: &[&str]) -> Vec<String> {
    commands.iter().map(|value| (*value).to_string()).collect()
}

fn intro_surface(commands: &[&str]) -> ReplSurface {
    ReplSurface {
        root_words: intro_commands(commands),
        intro_commands: intro_commands(commands),
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: Vec::new(),
    }
}

#[test]
fn theme_slug_is_rendered_as_title_case_display_name_unit() {
    assert_eq!(repl::theme_display_name("rose-pine-moon"), "Rose Pine Moon");
    assert_eq!(repl::theme_display_name("dracula"), "Dracula");
}

#[test]
fn repl_prompt_right_shows_ascii_incognito_marker_unit() {
    let settings = RenderSettings::test_plain(OutputFormat::Table);
    let resolved = settings.resolve_render_settings();
    let timing = crate::app::DebugTimingState::default();

    let rendered = repl::render_repl_prompt_right_for_test(&resolved, false, &timing);

    assert!(rendered.contains("incognito"));
}

#[test]
fn repl_prompt_right_includes_timing_breakdown_at_debug_three_unit() {
    let settings = RenderSettings::test_plain(OutputFormat::Table);
    let resolved = settings.resolve_render_settings();
    let timing = crate::app::DebugTimingState::default();
    timing.set(crate::app::DebugTimingBadge {
        level: 3,
        summary: crate::app::TimingSummary {
            total: std::time::Duration::from_millis(321),
            parse: Some(std::time::Duration::from_millis(4)),
            execute: Some(std::time::Duration::from_millis(300)),
            render: Some(std::time::Duration::from_millis(17)),
        },
    });

    let rendered = repl::render_repl_prompt_right_for_test(&resolved, true, &timing);

    assert!(rendered.contains("321.0ms"));
    assert!(rendered.contains("p4.0ms"));
    assert!(rendered.contains("e300.0ms"));
    assert!(rendered.contains("r17.0ms"));
}

#[test]
fn startup_prompt_timing_is_seeded_once_unit() {
    let settings = RenderSettings::test_plain(OutputFormat::Table);
    let resolved = settings.resolve_render_settings();
    let mut session = crate::app::AppSession::with_cache_limit(8);

    session.seed_startup_prompt_timing(1, std::time::Duration::from_millis(42));
    session.seed_startup_prompt_timing(1, std::time::Duration::from_millis(99));

    let rendered = repl::render_repl_prompt_right_for_test(&resolved, true, &session.prompt_timing);

    assert!(rendered.contains("42ms"));
    assert!(!rendered.contains("99ms"));
}

#[test]
fn repl_prompt_template_substitutes_profile_and_indicator_unit() {
    let rendered = repl::render_prompt_template(
        "╭─{user}@{domain} {indicator}\n╰─{profile}> ",
        "oistes",
        "uio.no",
        "uio",
        "[orch]",
    );
    assert!(rendered.contains("oistes@uio.no [orch]"));
    assert!(rendered.contains("╰─uio> "));
}

#[test]
fn repl_prompt_template_appends_indicator_when_missing_placeholder_unit() {
    let rendered = repl::render_prompt_template("{profile}>", "oistes", "uio.no", "tsd", "[shell]");
    assert_eq!(rendered, "tsd> [shell]");
}

#[test]
fn repl_simple_prompt_shows_shell_scope_indicator_unit() {
    let mut state = make_completion_state_with_entries(None, &[("repl.simple_prompt", "true")]);
    state.session.scope.enter("orch");

    let prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;

    assert!(prompt.contains("[orch]"));
    assert!(prompt.ends_with("> "));
}

#[test]
fn repl_help_chrome_replaces_clap_headings_unit() {
    let state = make_completion_state(None);
    let raw =
        "Usage: config <COMMAND>\n\nCommands:\n  show\n\nOptions:\n  -h, --help  Print help\n";
    let rendered =
        repl_help::render_repl_help_with_chrome(repl_view(&state.runtime, &state.session), raw);
    assert!(rendered.contains("Usage"));
    assert!(rendered.contains("config <COMMAND>"));
    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("Options"));
}

#[test]
fn repl_help_chrome_passthrough_without_known_sections_unit() {
    let state = make_completion_state(None);
    let raw = "custom help text";
    assert_eq!(
        repl_help::render_repl_help_with_chrome(repl_view(&state.runtime, &state.session), raw,),
        raw
    );
}

#[test]
fn austere_repl_intro_is_minimal_single_line_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "austere")]);
    let rendered = crate::repl::presentation::render_repl_intro(
        repl_view(&state.runtime, &state.session),
        &intro_surface(&["help", "config", "theme", "plugins"]),
    );

    assert_eq!(
        rendered,
        format!(
            "\nWelcome anonymous. v{}. Commands: help, config, theme, plugins. See help for more.\n\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn repl_intro_respects_builtin_visibility_unit() {
    let state = make_completion_state_with_entries(Some("help"), &[("ui.presentation", "compact")]);
    let rendered = crate::repl::presentation::render_repl_intro(
        repl_view(&state.runtime, &state.session),
        &intro_surface(&["help"]),
    );

    assert!(rendered.contains("Commands: help."), "{rendered:?}");
    assert!(!rendered.contains("config"));
    assert!(!rendered.contains("theme"));
    assert!(!rendered.contains("plugins"));
}

#[test]
fn compact_repl_intro_is_minimal_single_line_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "compact")]);
    let rendered = crate::repl::presentation::render_repl_intro(
        repl_view(&state.runtime, &state.session),
        &intro_surface(&["help", "config", "theme", "plugins"]),
    );

    assert_eq!(
        rendered,
        format!(
            "\nWelcome anonymous. v{}. Commands: help, config, theme, plugins. See help for more.\n\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn presentation_profiles_shape_help_output_snapshot_unit() {
    assert_snapshot!(
        "presentation_help_expressive",
        render_help_snapshot(&[("ui.presentation", "expressive")])
    );
    assert_snapshot!(
        "presentation_help_compact",
        render_help_snapshot(&[("ui.presentation", "compact")])
    );
    assert_snapshot!(
        "presentation_help_austere",
        render_help_snapshot(&[("ui.presentation", "austere")])
    );
}

#[test]
fn presentation_profiles_shape_message_output_snapshot_unit() {
    assert_snapshot!(
        "presentation_messages_expressive",
        render_message_snapshot(&[("ui.presentation", "expressive")])
    );
    assert_snapshot!(
        "presentation_messages_compact",
        render_message_snapshot(&[("ui.presentation", "compact")])
    );
    assert_snapshot!(
        "presentation_messages_austere",
        render_message_snapshot(&[("ui.presentation", "austere")])
    );
}

#[test]
fn presentation_profiles_shape_prompt_output_snapshot_unit() {
    assert_snapshot!(
        "presentation_prompt_expressive",
        render_prompt_snapshot(&[("ui.presentation", "expressive")])
    );
    assert_snapshot!(
        "presentation_prompt_compact",
        render_prompt_snapshot(&[("ui.presentation", "compact")])
    );
    assert_snapshot!(
        "presentation_prompt_austere",
        render_prompt_snapshot(&[("ui.presentation", "austere")])
    );
}

#[test]
fn presentation_profiles_shape_table_output_snapshot_unit() {
    assert_snapshot!(
        "presentation_table_expressive",
        render_table_snapshot(&[("ui.presentation", "expressive")])
    );
    assert_snapshot!(
        "presentation_table_compact",
        render_table_snapshot(&[("ui.presentation", "compact")])
    );
    assert_snapshot!(
        "presentation_table_austere",
        render_table_snapshot(&[("ui.presentation", "austere")])
    );
}

#[test]
fn help_render_overrides_parse_long_flags_unit() {
    let args = vec![
        OsString::from("osp"),
        OsString::from("--profile"),
        OsString::from("tsd"),
        OsString::from("--theme=dracula"),
        OsString::from("--presentation"),
        OsString::from("compact"),
        OsString::from("--mode"),
        OsString::from("plain"),
        OsString::from("--color=always"),
        OsString::from("--unicode"),
        OsString::from("never"),
        OsString::from("--no-env"),
        OsString::from("--no-config-file"),
        OsString::from("--ascii"),
    ];

    let parsed = parse_help_render_overrides(&args);
    assert_eq!(parsed.profile.as_deref(), Some("tsd"));
    assert_eq!(parsed.theme.as_deref(), Some("dracula"));
    assert_eq!(
        parsed.presentation,
        Some(crate::ui::presentation::UiPresentation::Compact)
    );
    assert_eq!(parsed.mode, Some(crate::core::output::RenderMode::Plain));
    assert_eq!(parsed.color, Some(crate::core::output::ColorMode::Always));
    assert_eq!(
        parsed.unicode,
        Some(crate::core::output::UnicodeMode::Never)
    );
    assert!(parsed.no_env);
    assert!(parsed.no_config_file);
    assert!(parsed.ascii_legacy);
}

#[test]
fn help_render_overrides_parse_gammel_og_bitter_alias_unit() {
    let args = vec![OsString::from("osp"), OsString::from("--gammel-og-bitter")];

    let parsed = parse_help_render_overrides(&args);
    assert!(parsed.gammel_og_bitter);
}

#[test]
fn help_render_overrides_skips_next_flag_value_unit() {
    let args = vec![
        OsString::from("osp"),
        OsString::from("--mode"),
        OsString::from("--profile"),
        OsString::from("tsd"),
    ];
    let parsed = parse_help_render_overrides(&args);
    assert_eq!(parsed.mode, None);
    assert_eq!(parsed.profile.as_deref(), Some("tsd"));
}

#[test]
fn help_chrome_uses_unicode_dividers_when_enabled_unit() {
    let state = make_completion_state(None);
    let mut resolved = state.runtime.ui.render_settings.resolve_render_settings();
    resolved.unicode = true;
    let rendered = repl_help::render_help_with_chrome(
        "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
        &resolved,
        crate::ui::presentation::HelpLayout::Full,
    );
    assert!(rendered.contains("Usage"));
    assert!(rendered.contains("osp [OPTIONS]"));
    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("Options"));
}

#[test]
fn austere_help_layout_collapses_footer_spacing_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "austere")]);
    let raw = "Usage: osp [OPTIONS]\n\nOptions:\n  -h, --help\n\nUse `osp plugins commands` to list plugin-provided commands.\n";
    let rendered =
        repl_help::render_repl_help_with_chrome(repl_view(&state.runtime, &state.session), raw);

    assert!(rendered.contains("Options"));
    assert!(rendered.contains("Use `osp plugins commands`"));
    assert!(rendered.contains("\n\nUse `osp plugins commands`"));
    assert!(!rendered.contains("\n\n\nUse `osp plugins commands`"));
}

#[test]
fn sensitive_key_detection_handles_common_variants_unit() {
    assert!(is_sensitive_key("auth.api_key"));
    assert!(is_sensitive_key("ssh.private_key"));
    assert!(is_sensitive_key("oauth.access_token"));
    assert!(is_sensitive_key("client_secret"));
    assert!(is_sensitive_key("bearer_token"));
    assert!(!is_sensitive_key("ui.keybinding"));
    assert!(!is_sensitive_key("monkey.business"));
}
