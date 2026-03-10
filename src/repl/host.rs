use crate::config::ResolvedConfig;
use crate::repl::{DebugStep, ReplRunConfig, run_repl};
use anyhow::anyhow;
use miette::{Result, miette};
use std::sync::Arc;

use super::{completion, dispatch, input, lifecycle, presentation, surface};
use crate::app::sink::StdIoUiSink;
use crate::app::{AppRuntime, AppSession, AppState};
use crate::app::{AuthState, ReplScopeStack, UiState};
use crate::ui::theme_loader::ThemeCatalog;

use super::history;
use crate::app;
use crate::app::{CliCommandResult, document_from_json};
use crate::cli::{DebugCompleteArgs, DebugHighlightArgs, DebugMenuArg, ReplArgs, ReplCommands};
use crate::completion::CompletionTree;
use crate::ui::messages::MessageLevel;
use crate::ui::presentation::{
    ReplInputMode, intro_style, intro_style_with_verbosity, repl_input_mode,
};
pub(crate) use dispatch::repl_command_spec;
use presentation::{build_repl_appearance, build_repl_prompt_right_renderer};
use surface::ReplSurface;

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

// The REPL loop is intentionally boring at this level:
// prepare one cycle, render the shell chrome, run the line editor, apply the result.
pub(crate) fn run_plugin_repl(state: &mut AppState) -> Result<i32> {
    let mut loop_state = lifecycle::ReplLoopState::new(should_show_repl_intro(
        state.runtime.config.resolved(),
        state.runtime.ui.message_verbosity,
    ));
    let mut sink = StdIoUiSink;

    loop {
        let cycle =
            loop_state.prepare_cycle(&mut state.runtime, &mut state.session, &mut state.clients)?;
        loop_state.render_cycle_chrome(
            &mut sink,
            ReplViewContext::from_parts(&state.runtime, &state.session),
            &cycle.help_text,
        );

        state.session.seed_startup_prompt_timing(
            state.runtime.ui.debug_verbosity,
            state.runtime.launch.startup_started_at.elapsed(),
        );

        let result = run_repl(
            ReplRunConfig {
                prompt: cycle.prompt,
                completion_words: cycle.root_words,
                completion_tree: Some(cycle.completion_tree),
                appearance: cycle.appearance,
                history_config: cycle.history_config,
                input_mode: map_repl_input_mode(repl_input_mode(state.runtime.config.resolved())),
                prompt_right: Some(build_repl_prompt_right_renderer(
                    ReplViewContext::from_parts(&state.runtime, &state.session),
                    state.session.prompt_timing.clone(),
                )),
                line_projector: Some(build_repl_ui_line_projector(
                    state.runtime.config.resolved(),
                )),
            },
            |line, history| {
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
            },
        )
        .map_err(|err| miette!("{err:#}"))?;
        if let Some(code) = loop_state.apply_run_result(&mut sink, result) {
            return Ok(code);
        }
    }
}

fn should_show_repl_intro(config: &ResolvedConfig, verbosity: MessageLevel) -> bool {
    !matches!(
        intro_style_with_verbosity(intro_style(config), verbosity),
        crate::ui::presentation::ReplIntroStyle::None
    )
}

fn map_repl_input_mode(mode: ReplInputMode) -> crate::repl::ReplInputMode {
    match mode {
        ReplInputMode::Auto => crate::repl::ReplInputMode::Auto,
        ReplInputMode::Interactive => crate::repl::ReplInputMode::Interactive,
        ReplInputMode::Basic => crate::repl::ReplInputMode::Basic,
    }
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
    let catalog = app::authorized_command_catalog_for(&runtime.auth, clients)?;
    let view = ReplViewContext::from_parts(runtime, session);
    let surface = surface::build_repl_surface(view, &catalog);
    let completion_tree = build_repl_completion_tree(view, &surface);
    let appearance = build_repl_appearance(view);
    let cursor = args.cursor.unwrap_or(args.line.len());

    let steps = args
        .steps
        .iter()
        .map(|raw| DebugStep::parse(raw).ok_or_else(|| miette!("Unknown debug step '{raw}'")))
        .collect::<Result<Vec<_>>>()?;

    let payload = if steps.is_empty() {
        let projected_line = input::project_repl_ui_line(&args.line, runtime.config.resolved())?;
        let options = crate::repl::CompletionDebugOptions::new(args.width, args.height)
            .ansi(args.menu_ansi)
            .unicode(args.menu_unicode)
            .appearance(Some(&appearance));
        let debug = match args.menu {
            DebugMenuArg::Completion => crate::repl::debug_completion(
                &completion_tree,
                &projected_line.line,
                cursor,
                options,
            ),
            DebugMenuArg::History => {
                let history = crate::repl::SharedHistory::new(history::build_history_config(
                    runtime, session,
                ))
                .map_err(|err| miette!("{err:#}"))?;
                crate::repl::debug_history_menu(&history, &projected_line.line, cursor, options)
            }
        };
        serde_json::to_string_pretty(&debug).map_err(|err| miette!("{err:#}"))?
    } else {
        let projected_line = input::project_repl_ui_line(&args.line, runtime.config.resolved())?;
        let options = crate::repl::CompletionDebugOptions::new(args.width, args.height)
            .ansi(args.menu_ansi)
            .unicode(args.menu_unicode)
            .appearance(Some(&appearance));
        let frames = match args.menu {
            DebugMenuArg::Completion => crate::repl::debug_completion_steps(
                &completion_tree,
                &projected_line.line,
                cursor,
                options,
                &steps,
            ),
            DebugMenuArg::History => {
                let history = crate::repl::SharedHistory::new(history::build_history_config(
                    runtime, session,
                ))
                .map_err(|err| miette!("{err:#}"))?;
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
    Ok(CliCommandResult::document(document_from_json(payload)))
}

fn run_repl_debug_highlight(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &crate::app::AppClients,
    args: DebugHighlightArgs,
) -> Result<CliCommandResult> {
    let catalog = app::authorized_command_catalog_for(&runtime.auth, clients)?;
    let view = ReplViewContext::from_parts(runtime, session);
    let surface = surface::build_repl_surface(view, &catalog);
    let completion_tree = build_repl_completion_tree(view, &surface);
    let appearance = build_repl_appearance(view);
    let projected_line = input::project_repl_ui_line(&args.line, runtime.config.resolved())?;
    let command_color = appearance
        .command_highlight_style
        .as_deref()
        .and_then(crate::repl::color_from_style_spec)
        .unwrap_or_else(|| {
            crate::repl::color_from_style_spec("green")
                .expect("known fallback REPL command highlight color should parse")
        });
    let spans = crate::repl::debug_highlight(
        &completion_tree,
        &args.line,
        command_color,
        Some(build_repl_ui_line_projector(runtime.config.resolved())),
    );
    let payload = serde_json::json!({
        "line": args.line,
        "projected_line": projected_line.line,
        "hidden_suggestions": projected_line.hidden_suggestions,
        "spans": spans,
    });
    Ok(CliCommandResult::document(document_from_json(payload)))
}

fn build_repl_completion_tree(view: ReplViewContext<'_>, surface: &ReplSurface) -> CompletionTree {
    completion::build_repl_completion_tree(view, surface)
}

fn build_repl_ui_line_projector(
    config: &ResolvedConfig,
) -> Arc<dyn Fn(&str) -> crate::repl::LineProjection + Send + Sync> {
    let config = config.clone();
    Arc::new(move |line| {
        input::project_repl_ui_line(line, &config)
            .unwrap_or_else(|_| crate::repl::LineProjection::passthrough(line))
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_repl_ui_line_projector, map_repl_input_mode, run_repl_debug_command_for,
        should_show_repl_intro,
    };
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
    use crate::ui::presentation::ReplInputMode;

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
            themes: crate::ui::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        });
        (state.runtime, state.session, state.clients)
    }

    #[test]
    fn cycle_chrome_renders_intro_then_help_then_pending_output() {
        let (runtime, session, _) = make_state();
        let output = build_cycle_chrome_output(
            super::ReplViewContext::from_parts(&runtime, &session),
            "Welcome anonymous.\nHELP\n",
            true,
            "PENDING\n",
        );

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
        let (runtime, session, _) = make_state();
        let output = build_cycle_chrome_output(
            super::ReplViewContext::from_parts(&runtime, &session),
            "HELP\n",
            false,
            "PENDING\n",
        );

        assert_eq!(output, "PENDING\n");
    }

    #[test]
    fn repl_input_mode_mapping_covers_all_variants_unit() {
        assert_eq!(
            map_repl_input_mode(ReplInputMode::Auto),
            crate::repl::ReplInputMode::Auto
        );
        assert_eq!(
            map_repl_input_mode(ReplInputMode::Interactive),
            crate::repl::ReplInputMode::Interactive
        );
        assert_eq!(
            map_repl_input_mode(ReplInputMode::Basic),
            crate::repl::ReplInputMode::Basic
        );
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
    fn repl_ui_line_projector_falls_back_to_passthrough_on_parse_error_unit() {
        let (runtime, _, _) = make_state();
        let projector = build_repl_ui_line_projector(runtime.config.resolved());
        let projected = projector("config \"unterminated");

        assert_eq!(projected.line, "config \"unterminated");
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
            Some(crate::app::ReplCommandOutput::Document(_))
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
            Some(crate::app::ReplCommandOutput::Document(_))
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
            Some(crate::app::ReplCommandOutput::Document(_))
        ));
    }
}
