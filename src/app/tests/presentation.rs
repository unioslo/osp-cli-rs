use super::*;
use insta::assert_snapshot;

#[test]
fn repl_prompt_right_variants_render_incognito_and_timing_unit() {
    let settings = RenderSettings::test_plain(OutputFormat::Table);
    let resolved = settings.resolve_render_settings();
    let plain_timing = crate::app::DebugTimingState::default();
    let plain = repl::render_repl_prompt_right_for_test(&resolved, None, false, &plain_timing);
    assert!(plain.contains("incognito"));

    let breakdown_timing = crate::app::DebugTimingState::default();
    breakdown_timing.set(crate::app::DebugTimingBadge {
        level: 3,
        summary: crate::app::TimingSummary {
            total: std::time::Duration::from_millis(321),
            parse: Some(std::time::Duration::from_millis(4)),
            execute: Some(std::time::Duration::from_millis(300)),
            render: Some(std::time::Duration::from_millis(17)),
        },
    });
    let breakdown =
        repl::render_repl_prompt_right_for_test(&resolved, None, true, &breakdown_timing);
    assert!(breakdown.contains("321.0ms"));
    assert!(breakdown.contains("p4.0ms"));
    assert!(breakdown.contains("e300.0ms"));
    assert!(breakdown.contains("r17.0ms"));

    let mut session = crate::app::AppSession::with_cache_limit(8);
    session.seed_startup_prompt_timing(1, std::time::Duration::from_millis(42));
    session.seed_startup_prompt_timing(1, std::time::Duration::from_millis(99));
    let seeded =
        repl::render_repl_prompt_right_for_test(&resolved, None, true, &session.prompt_timing);
    assert!(seeded.contains("42ms"));
    assert!(!seeded.contains("99ms"));
}

#[test]
fn repl_help_chrome_variants_render_expected_structure_unit() {
    let state = make_completion_state(None);
    let known_sections =
        "Usage: config <COMMAND>\n\nCommands:\n  show\n\nOptions:\n  -h, --help  Print help\n";
    let headings = repl_help::render_repl_help_with_chrome(
        repl_view(&state.runtime, &state.session),
        known_sections,
    );
    assert!(headings.contains("Usage"));
    assert!(headings.contains("config <COMMAND>"));
    assert!(headings.contains("Commands"));
    assert!(headings.contains("Options"));

    let passthrough = "custom help text";
    assert_eq!(
        repl_help::render_repl_help_with_chrome(
            repl_view(&state.runtime, &state.session),
            passthrough,
        ),
        passthrough
    );

    let mut resolved = state.runtime.ui.render_settings.resolve_render_settings();
    resolved.unicode = true;
    let unicode = repl_help::render_help_with_chrome(
        "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
        &resolved,
        crate::ui::HelpLayout::Full,
    );
    assert!(unicode.contains("Usage"));
    assert!(unicode.contains("osp [OPTIONS]"));
    assert!(unicode.contains("Commands"));
    assert!(unicode.contains("Options"));

    let austere = make_completion_state_with_entries(None, &[("ui.presentation", "austere")]);
    let footer = "Usage: osp [OPTIONS]\n\nOptions:\n  -h, --help\n\nUse `osp plugins commands` to list plugin-provided commands.\n";
    let rendered = repl_help::render_repl_help_with_chrome(
        repl_view(&austere.runtime, &austere.session),
        footer,
    );
    assert!(rendered.contains("Options"));
    assert!(rendered.contains("Use `osp plugins commands`"));
    assert!(rendered.contains("\n\nUse `osp plugins commands`"));
    assert!(!rendered.contains("\n\n\nUse `osp plugins commands`"));
}

#[test]
fn presentation_prompt_profiles_preserve_local_snapshots_unit() {
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
fn help_render_overrides_parse_supported_and_edge_flags_unit() {
    let supported_args = vec![
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
    let parsed = parse_help_render_overrides(&supported_args);
    assert_eq!(parsed.profile.as_deref(), Some("tsd"), "supported flags");
    assert_eq!(parsed.theme.as_deref(), Some("dracula"), "supported flags");
    assert_eq!(
        parsed.presentation,
        Some(crate::ui::UiPresentation::Compact),
        "supported flags"
    );
    assert_eq!(
        parsed.mode,
        Some(crate::core::output::RenderMode::Plain),
        "supported flags"
    );
    assert_eq!(
        parsed.color,
        Some(crate::core::output::ColorMode::Always),
        "supported flags"
    );
    assert_eq!(
        parsed.unicode,
        Some(crate::core::output::UnicodeMode::Never),
        "supported flags"
    );
    assert!(parsed.no_env);
    assert!(parsed.no_config_file);
    assert!(parsed.ascii_legacy);

    let alias_args = vec![OsString::from("osp"), OsString::from("--gammel-og-bitter")];
    let alias = parse_help_render_overrides(&alias_args);
    assert!(alias.gammel_og_bitter);

    let edge_args = vec![
        OsString::from("osp"),
        OsString::from("--mode"),
        OsString::from("--profile"),
        OsString::from("tsd"),
    ];
    let edge = parse_help_render_overrides(&edge_args);
    assert_eq!(edge.mode, None);
    assert_eq!(edge.profile.as_deref(), Some("tsd"));
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
