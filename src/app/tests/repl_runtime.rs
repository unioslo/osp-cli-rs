use super::*;
use insta::assert_snapshot;

fn env_lock() -> &'static std::sync::Mutex<()> {
    crate::tests::env_lock()
}

include!("repl_runtime/plugin_dispatch.rs");
include!("repl_runtime/rebuild_restart.rs");
include!("repl_runtime/session_shell.rs");

#[test]
fn repl_invalid_subcommand_renders_inline_help_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config sho",
    )
    .expect("invalid subcommand should stay inside repl help flow");

    match rendered {
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("unrecognized subcommand"));
            assert!(text.contains("config <COMMAND>"));
            assert!(!text.contains("For more information, try '--help'."));
            assert_snapshot!("repl_invalid_subcommand_inline_help", text);
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[test]
fn repl_help_alias_hides_common_invocation_options_without_verbose_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "help history",
    )
    .expect("flag-prefixed help alias should stay in help flow");

    match rendered {
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("history"));
            assert!(!text.contains("Common Invocation Options"));
            assert_snapshot!("repl_help_alias_default", text);
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[test]
fn repl_help_alias_keeps_colored_help_chrome_and_subcommands_unit() {
    let mut state = make_test_state(Vec::new());
    state.runtime.ui.render_settings.mode = RenderMode::Auto;
    state.runtime.ui.render_settings.color = ColorMode::Always;
    state.runtime.ui.render_settings.unicode = UnicodeMode::Always;
    state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    state.runtime.ui.render_settings.style_overrides = crate::ui::style::StyleOverrides {
        panel_title: Some("green".to_string()),
        key: Some("red".to_string()),
        value: Some("blue".to_string()),
        ..crate::ui::style::StyleOverrides::default()
    };
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "help history",
    )
    .expect("help alias should render with chrome");

    match rendered {
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("\u{1b}[32mUsage\u{1b}[0m"));
            assert!(text.contains("\u{1b}[31mlist\u{1b}[0m"));
            assert!(text.contains("\u{1b}[31mprune\u{1b}[0m"));
            assert!(text.contains("\u{1b}[31mclear\u{1b}[0m"));
            assert_snapshot!("repl_help_alias_colored", text);
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[test]
fn repl_help_alias_rejects_help_and_flag_targets_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    for line in ["help help", "help --help"] {
        let rendered = repl_dispatch::execute_repl_plugin_line(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &history,
            line,
        )
        .expect("invalid help target should stay in repl help flow");

        match rendered {
            crate::repl::ReplLineResult::Continue(text) => {
                assert!(text.contains("invalid help target"));
                assert!(text.contains("Usage"));
                assert!(text.contains("help <command>"));
                assert_snapshot!(
                    format!("repl_help_alias_invalid_target_{}", line.replace(' ', "_")),
                    text
                );
            }
            other => panic!("unexpected repl result: {other:?}"),
        }
    }
}

#[test]
fn repl_verbose_help_alias_dispatches_to_command_help_with_invocation_section_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "help history -v",
    )
    .expect("verbose help alias should stay in help flow");

    match rendered {
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("history"));
            assert!(text.contains("Common Invocation Options"));
            assert_snapshot!("repl_help_alias_verbose", text);
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[test]
fn repl_verbose_direct_help_shows_common_invocation_options_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "history --help -v",
    )
    .expect("verbose direct help should stay in help flow");

    match rendered {
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("history"));
            assert!(text.contains("Common Invocation Options"));
            assert_snapshot!("repl_direct_help_verbose", text);
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[test]
fn repl_flag_prefixed_help_records_prompt_timing_badge_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "-ddd help config",
    )
    .expect("flag-prefixed help should render successfully");

    match rendered {
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("config <COMMAND>"));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }

    let badge = state
        .session
        .prompt_timing
        .badge()
        .expect("help flow should update prompt timing");
    assert_eq!(badge.level, 3);

    let prompt_right = crate::repl::render_repl_prompt_right_for_test(
        &state.runtime.ui.render_settings.resolve_render_settings(),
        true,
        &state.session.prompt_timing,
    );
    assert!(
        prompt_right.contains("ms"),
        "unexpected prompt right: {prompt_right:?}"
    );
    assert!(
        prompt_right.contains('p'),
        "unexpected prompt right: {prompt_right:?}"
    );
}
