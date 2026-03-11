use super::*;

fn env_lock() -> &'static std::sync::Mutex<()> {
    crate::tests::env_lock()
}

include!("repl_runtime/plugin_dispatch.rs");
include!("repl_runtime/rebuild_restart.rs");
include!("repl_runtime/session_shell.rs");

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
