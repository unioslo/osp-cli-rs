use crate::config::ResolvedConfig;
use crate::repl::{DebugStep, run_repl};
use anyhow::anyhow;
use miette::{Result, miette};

use super::{dispatch, input, lifecycle};
use crate::app::sink::StdIoUiSink;
use crate::app::{AppRuntime, AppSession, AppState};
use crate::app::{AuthState, ReplScopeStack, UiState};
use crate::ui::theme_catalog::ThemeCatalog;

use super::history;
use super::presentation::{ReplIntroStyle, intro_style, intro_style_with_verbosity};
use crate::app::CliCommandResult;
use crate::cli::{DebugCompleteArgs, DebugHighlightArgs, DebugMenuArg, ReplArgs, ReplCommands};
use crate::ui::messages::MessageLevel;
pub(crate) use dispatch::repl_command_spec;

#[derive(Clone, Copy)]
pub(crate) struct ReplViewContext<'a> {
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) auth: &'a AuthState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) scope: &'a ReplScopeStack,
}

impl<'a> ReplViewContext<'a> {
    pub(crate) fn from_parts(runtime: &'a AppRuntime, session: &'a AppSession) -> Self {
        Self {
            config: runtime.config.resolved(),
            ui: &runtime.ui,
            auth: &runtime.auth,
            themes: &runtime.themes,
            scope: &session.scope,
        }
    }
}

// Keep the host loop as orchestration only so editor/runtime details stay in
// `repl::engine` and command semantics stay in `repl::dispatch`.
pub(crate) fn run_plugin_repl(state: &mut AppState) -> Result<i32> {
    let mut loop_state = lifecycle::ReplLoopState::new(should_show_repl_intro(
        state.runtime.config.resolved(),
        state.runtime.ui.message_verbosity,
    ));
    let mut sink = StdIoUiSink;

    loop {
        // 1. Prepare the next REPL cycle from current runtime/session state.
        let cycle = loop_state.prepare_cycle(state)?;

        // 2. Print any intro/help chrome and pending restart output.
        loop_state.render_cycle_chrome(&mut sink, &cycle.help_text);

        // 3. Run the editor-owned REPL engine for this prepared cycle.
        let result = run_repl_cycle(state, cycle)?;

        // 4. Apply restart/exit effects back to lifecycle state.
        if let Some(code) = loop_state.apply_run_result(&mut sink, result) {
            return Ok(code);
        }
    }
}

fn run_repl_cycle(
    state: &mut AppState,
    cycle: lifecycle::ReplCycle,
) -> Result<crate::repl::ReplRunResult> {
    state.session.seed_startup_prompt_timing(
        state.runtime.ui.debug_verbosity,
        state.runtime.launch.startup_started_at.elapsed(),
    );

    run_repl(cycle.run_config, |line, history| {
        dispatch::execute_repl_plugin_line(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            history,
            line,
        )
        .map_err(|err| {
            anyhow!(
                "{}",
                crate::app::render_report_message(&err, state.runtime.ui.message_verbosity)
            )
        })
    })
    .map_err(|err| miette!("{err:#}"))
}

fn should_show_repl_intro(config: &ResolvedConfig, verbosity: MessageLevel) -> bool {
    !matches!(
        intro_style_with_verbosity(intro_style(config), verbosity),
        ReplIntroStyle::None
    )
}

pub(crate) fn run_repl_debug_command_for(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &crate::app::AppClients,
    args: ReplArgs,
) -> Result<CliCommandResult> {
    match args.command {
        ReplCommands::DebugComplete(args) => {
            run_repl_debug_complete(runtime, session, clients, args)
        }
        ReplCommands::DebugHighlight(args) => {
            run_repl_debug_highlight(runtime, session, clients, args)
        }
    }
}

fn run_repl_debug_complete(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &crate::app::AppClients,
    args: DebugCompleteArgs,
) -> Result<CliCommandResult> {
    let prepared = lifecycle::prepare_repl_surface_state(runtime, session, clients)?;
    let cursor = args.cursor.unwrap_or(args.line.len());

    let steps = args
        .steps
        .iter()
        .map(|raw| DebugStep::parse(raw).ok_or_else(|| miette!("Unknown debug step '{raw}'")))
        .collect::<Result<Vec<_>>>()?;

    let payload = if steps.is_empty() {
        let projected_line = input::project_repl_ui_line(&args.line, runtime.config.resolved())?;
        let options = crate::repl::CompletionDebugOptions::new(args.width, args.height)
            .with_ansi(args.menu_ansi)
            .with_unicode(args.menu_unicode)
            .with_appearance(Some(&prepared.appearance));
        let debug = match args.menu {
            DebugMenuArg::Completion => crate::repl::debug_completion(
                &prepared.completion_tree,
                &projected_line.line,
                cursor,
                options,
            ),
            DebugMenuArg::History => {
                let history = crate::repl::SharedHistory::new(history::build_history_config(
                    runtime, session,
                ));
                crate::repl::debug_history_menu(&history, &projected_line.line, cursor, options)
            }
        };
        serde_json::to_string_pretty(&debug).map_err(|err| miette!("{err:#}"))?
    } else {
        let projected_line = input::project_repl_ui_line(&args.line, runtime.config.resolved())?;
        let options = crate::repl::CompletionDebugOptions::new(args.width, args.height)
            .with_ansi(args.menu_ansi)
            .with_unicode(args.menu_unicode)
            .with_appearance(Some(&prepared.appearance));
        let frames = match args.menu {
            DebugMenuArg::Completion => crate::repl::debug_completion_steps(
                &prepared.completion_tree,
                &projected_line.line,
                cursor,
                options,
                &steps,
            ),
            DebugMenuArg::History => {
                let history = crate::repl::SharedHistory::new(history::build_history_config(
                    runtime, session,
                ));
                crate::repl::debug_history_menu_steps(
                    &history,
                    &projected_line.line,
                    cursor,
                    options,
                    &steps,
                )
            }
        };
        serde_json::to_string_pretty(&frames).map_err(|err| miette!("{err:#}"))?
    };
    let payload = serde_json::from_str(&payload).map_err(|err| miette!("{err:#}"))?;
    Ok(CliCommandResult::json(payload))
}

fn run_repl_debug_highlight(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &crate::app::AppClients,
    args: DebugHighlightArgs,
) -> Result<CliCommandResult> {
    let prepared = lifecycle::prepare_repl_surface_state(runtime, session, clients)?;
    let projected_line = input::project_repl_ui_line(&args.line, runtime.config.resolved())?;
    let command_color = prepared
        .appearance
        .command_highlight_style
        .as_deref()
        .and_then(crate::repl::color_from_style_spec)
        .unwrap_or(nu_ansi_term::Color::Green);
    let spans = crate::repl::debug_highlight(
        &prepared.completion_tree,
        &args.line,
        command_color,
        Some(input::build_repl_ui_line_projector(
            runtime.config.resolved(),
        )),
    );
    let payload = serde_json::json!({
        "line": args.line,
        "projected_line": projected_line.line,
        "hidden_suggestions": projected_line.hidden_suggestions,
        "spans": spans,
    });
    Ok(CliCommandResult::json(payload))
}

#[cfg(test)]
mod tests {
    use super::{run_repl_debug_command_for, should_show_repl_intro};
    use crate::app::{
        AppClients, AppRuntime, AppSession, AppState, AppStateInit, LaunchContext, RuntimeContext,
        TerminalKind,
    };
    use crate::cli::{DebugCompleteArgs, DebugHighlightArgs, DebugMenuArg, ReplArgs, ReplCommands};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::repl::lifecycle::build_cycle_chrome_output;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;

    fn make_state() -> (AppRuntime, AppSession, AppClients) {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        let settings = RenderSettings::test_plain(OutputFormat::Json);

        let state = AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            render_settings: settings,
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins: crate::plugin::PluginManager::new(Vec::new()),
            native_commands: crate::native::NativeCommandRegistry::default(),
            themes: crate::ui::theme_catalog::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        });
        (state.runtime, state.session, state.clients)
    }

    #[test]
    fn cycle_chrome_renders_intro_then_help_then_pending_output() {
        let output = build_cycle_chrome_output("Welcome anonymous.\nHELP\n", true, "PENDING\n");

        let intro_pos = output.find("Welcome").expect("intro should render");
        let help_pos = output.find("HELP").expect("help should render");
        let pending_pos = output
            .find("PENDING")
            .expect("pending output should render");

        assert!(output.starts_with("\x1b[2J\x1b[H"));
        assert!(intro_pos < help_pos);
        assert!(help_pos < pending_pos);
    }

    #[test]
    fn cycle_chrome_without_intro_keeps_pending_output_only() {
        let output = build_cycle_chrome_output("HELP\n", false, "PENDING\n");

        assert_eq!(output, "PENDING\n");
    }

    #[test]
    fn quiet_verbosity_suppresses_repl_intro_unit() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("config should resolve");

        assert!(should_show_repl_intro(&config, MessageLevel::Success));
        assert!(!should_show_repl_intro(&config, MessageLevel::Warning));
    }

    #[test]
    fn repl_debug_commands_return_documents_unit() {
        let (runtime, session, clients) = make_state();

        let complete = run_repl_debug_command_for(
            &runtime,
            &session,
            &clients,
            ReplArgs {
                command: ReplCommands::DebugComplete(DebugCompleteArgs {
                    line: "co".to_string(),
                    menu: DebugMenuArg::Completion,
                    cursor: None,
                    width: 80,
                    height: 8,
                    menu_ansi: false,
                    menu_unicode: false,
                    steps: Vec::new(),
                }),
            },
        )
        .expect("debug complete should succeed");
        assert!(matches!(
            complete.output,
            Some(crate::app::ReplCommandOutput::Json(_))
        ));

        let history_complete = run_repl_debug_command_for(
            &runtime,
            &session,
            &clients,
            ReplArgs {
                command: ReplCommands::DebugComplete(DebugCompleteArgs {
                    line: "co".to_string(),
                    menu: DebugMenuArg::History,
                    cursor: None,
                    width: 80,
                    height: 8,
                    menu_ansi: false,
                    menu_unicode: false,
                    steps: vec!["tab".to_string()],
                }),
            },
        )
        .expect("history debug complete should succeed");
        assert!(matches!(
            history_complete.output,
            Some(crate::app::ReplCommandOutput::Json(_))
        ));

        let highlight = run_repl_debug_command_for(
            &runtime,
            &session,
            &clients,
            ReplArgs {
                command: ReplCommands::DebugHighlight(DebugHighlightArgs {
                    line: "help history".to_string(),
                }),
            },
        )
        .expect("debug highlight should succeed");
        assert!(matches!(
            highlight.output,
            Some(crate::app::ReplCommandOutput::Json(_))
        ));
    }
}
