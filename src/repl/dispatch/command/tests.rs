use super::{
    ParsedReplDispatch, parse_repl_invocation, render_repl_command_output,
    repl_cache_key_for_command,
};
use crate::app::sink::BufferedUiSink;
use crate::app::{
    AppState, AppStateInit, CliCommandResult, LaunchContext, ReplCommandOutput, RuntimeContext,
    TerminalKind,
};
use crate::cli::{Commands, IntroArgs};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
use crate::core::output::OutputFormat;
use crate::repl::input::ReplParsedLine;
use crate::ui::document::{Block, LineBlock, LinePart};
use crate::ui::messages::MessageLevel;
use crate::ui::{Document, RenderSettings};

fn base_invocation(state: &AppState) -> crate::app::ResolvedInvocation {
    crate::app::resolve_invocation_ui(
        state.runtime.config.resolved(),
        &state.runtime.ui,
        &crate::cli::invocation::InvocationOptions::default(),
    )
}

fn make_state() -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let config = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve");

    AppState::new(AppStateInit {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        render_settings: RenderSettings::test_plain(OutputFormat::Json),
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
        plugins: crate::plugin::PluginManager::new(Vec::new()),
        native_commands: crate::native::NativeCommandRegistry::default(),
        themes: crate::ui::theme_loader::ThemeCatalog::default(),
        launch: LaunchContext::default(),
    })
}

#[test]
fn parse_repl_invocation_invalid_help_alias_paths_cover_help_and_error_unit() {
    let mut state = make_state();
    let parsed = ReplParsedLine {
        command_tokens: vec!["help".to_string()],
        dispatch_tokens: vec!["help".to_string()],
        stages: Vec::new(),
    };

    let ParsedReplDispatch::Help {
        result,
        effective,
        stages,
    } = parse_repl_invocation(&state.runtime, &state.session, &parsed)
        .expect("invalid help alias should render guide help")
    else {
        panic!("expected help dispatch");
    };

    let mut sink = BufferedUiSink::default();
    let rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "help",
        &stages,
        *result,
        &effective,
        &mut sink,
    )
    .expect("invalid help guide should render");
    assert!(rendered.contains("help expects a command target"));

    let staged = ReplParsedLine {
        command_tokens: vec!["help".to_string()],
        dispatch_tokens: vec!["help".to_string()],
        stages: vec!["config".to_string()],
    };
    let err = match parse_repl_invocation(&state.runtime, &state.session, &staged) {
        Ok(_) => panic!("staged invalid help alias should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("help expects a command target"));
}

#[test]
fn parse_repl_invocation_covers_missing_command_and_inline_help_errors_unit() {
    let mut state = make_state();
    let empty = ReplParsedLine {
        command_tokens: Vec::new(),
        dispatch_tokens: Vec::new(),
        stages: Vec::new(),
    };
    let err = match parse_repl_invocation(&state.runtime, &state.session, &empty) {
        Ok(_) => panic!("empty parsed line should fail here"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("missing command"));

    let parsed = ReplParsedLine::parse("config show --wat", state.runtime.config.resolved())
        .expect("line should parse");
    let ParsedReplDispatch::Help {
        result, effective, ..
    } = parse_repl_invocation(&state.runtime, &state.session, &parsed)
        .expect("unknown argument should turn into inline help")
    else {
        panic!("expected help dispatch");
    };
    let mut sink = BufferedUiSink::default();
    let rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "config show --wat",
        &[],
        *result,
        &effective,
        &mut sink,
    )
    .expect("inline help should render");
    assert!(rendered.contains("Usage"));

    let staged = ReplParsedLine::parse(
        "config show --wat | config",
        state.runtime.config.resolved(),
    )
    .expect("line should parse");
    let err = match parse_repl_invocation(&state.runtime, &state.session, &staged) {
        Ok(_) => panic!("staged inline help parse errors should fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("--wat"));
}

#[test]
fn repl_cache_key_covers_disabled_builtin_and_external_paths_unit() {
    let state = make_state();
    let mut invocation = crate::cli::invocation::InvocationOptions::default();
    assert!(
        repl_cache_key_for_command(
            &state.runtime,
            &Commands::Intro(IntroArgs::default()),
            &invocation
        )
        .is_none()
    );

    invocation.cache = true;
    assert!(
        repl_cache_key_for_command(
            &state.runtime,
            &Commands::Intro(IntroArgs::default()),
            &invocation
        )
        .is_none()
    );

    invocation.plugin_provider = Some("plugin-a".to_string());
    let key = repl_cache_key_for_command(
        &state.runtime,
        &Commands::External(vec!["theme".to_string(), "show".to_string()]),
        &invocation,
    )
    .expect("external cache key should exist");
    assert!(key.contains("provider:plugin-a"));
    assert!(key.contains("\"theme\""));
}

#[test]
fn render_repl_command_output_covers_document_and_text_pipeline_paths_unit() {
    let mut state = make_state();
    let invocation = base_invocation(&state);
    let mut sink = BufferedUiSink::default();

    let document_result = CliCommandResult {
        exit_code: 0,
        messages: Default::default(),
        output: Some(ReplCommandOutput::Document(Document {
            blocks: vec![
                Block::Line(LineBlock {
                    parts: vec![LinePart {
                        text: "alpha".to_string(),
                        token: None,
                    }],
                }),
                Block::Line(LineBlock {
                    parts: vec![LinePart {
                        text: "beta".to_string(),
                        token: None,
                    }],
                }),
            ],
        })),
        stderr_text: None,
        failure_report: None,
    };
    let document_rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "intro | beta",
        &["beta".to_string()],
        document_result,
        &invocation,
        &mut sink,
    )
    .expect("document pipeline should render");
    assert_eq!(document_rendered.trim(), "beta");

    let text_result = CliCommandResult {
        exit_code: 0,
        messages: Default::default(),
        output: Some(ReplCommandOutput::Text("alpha\nbeta\n".to_string())),
        stderr_text: None,
        failure_report: None,
    };
    let text_rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "intro | beta",
        &["beta".to_string()],
        text_result,
        &invocation,
        &mut sink,
    )
    .expect("text pipeline should render");
    assert_eq!(text_rendered.trim(), "beta");
}
