pub(crate) mod completion;
pub(crate) mod dispatch;
pub(crate) mod help;
pub(crate) mod history;
pub(crate) mod input;
pub(crate) mod lifecycle;
pub(crate) mod presentation;
pub(crate) mod surface;

use anyhow::anyhow;
use miette::{Result, miette};
use osp_config::ResolvedConfig;
use osp_repl::{DebugStep, ReplRunConfig, run_repl};
use std::sync::Arc;

use crate::state::{AppRuntime, AppSession, AppState};
use crate::state::{AuthState, ReplScopeStack, UiState};
use crate::theme_loader::ThemeCatalog;

use crate::app;
use crate::app::{CliCommandResult, document_from_json};
use crate::cli::{DebugCompleteArgs, ReplArgs, ReplCommands};
use crate::ui_presentation::{ReplInputMode, effective_repl_input_mode, effective_repl_intro};
#[cfg(test)]
pub(crate) use dispatch::apply_repl_shell_prefix;
pub(crate) use dispatch::repl_command_spec;
#[cfg(test)]
pub(crate) use input::{ReplParsedLine, is_repl_shellable_command};
#[cfg(test)]
pub(crate) use lifecycle::build_cycle_chrome_output;
use osp_completion::CompletionTree;
#[cfg(test)]
pub(crate) use presentation::render_prompt_template;
#[cfg(test)]
pub(crate) use presentation::render_repl_prompt_right_for_test;
#[cfg(test)]
pub(crate) use presentation::theme_display_name;
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
    let mut loop_state =
        lifecycle::ReplLoopState::new(effective_repl_intro(state.runtime.config.resolved()));

    loop {
        let cycle =
            loop_state.prepare_cycle(&mut state.runtime, &mut state.session, &mut state.clients)?;
        loop_state.render_cycle_chrome(
            ReplViewContext::from_parts(&state.runtime, &state.session),
            &cycle.help_text,
        );

        let result = run_repl(
            ReplRunConfig {
                prompt: cycle.prompt,
                completion_words: cycle.root_words,
                completion_tree: Some(cycle.completion_tree),
                appearance: cycle.appearance,
                history_config: cycle.history_config,
                input_mode: map_repl_input_mode(effective_repl_input_mode(
                    state.runtime.config.resolved(),
                )),
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
        if let Some(code) = loop_state.apply_run_result(result) {
            return Ok(code);
        }
    }
}

fn map_repl_input_mode(mode: ReplInputMode) -> osp_repl::ReplInputMode {
    match mode {
        ReplInputMode::Auto => osp_repl::ReplInputMode::Auto,
        ReplInputMode::Interactive => osp_repl::ReplInputMode::Interactive,
        ReplInputMode::Basic => osp_repl::ReplInputMode::Basic,
    }
}

pub(crate) fn run_repl_debug_command_for(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &crate::state::AppClients,
    args: ReplArgs,
) -> Result<CliCommandResult> {
    match args.command {
        ReplCommands::DebugComplete(args) => {
            run_repl_debug_complete(runtime, session, clients, args)
        }
    }
}

fn run_repl_debug_complete(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &crate::state::AppClients,
    args: DebugCompleteArgs,
) -> Result<CliCommandResult> {
    let catalog = app::authorized_command_catalog_for(&runtime.auth, &clients.plugins)?;
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
        let debug = osp_repl::debug_completion(
            &completion_tree,
            &projected_line,
            cursor,
            osp_repl::CompletionDebugOptions::new(args.width, args.height)
                .ansi(args.menu_ansi)
                .unicode(args.menu_unicode)
                .appearance(Some(&appearance)),
        );
        serde_json::to_string_pretty(&debug).map_err(|err| miette!("{err:#}"))?
    } else {
        let projected_line = input::project_repl_ui_line(&args.line, runtime.config.resolved())?;
        let frames = osp_repl::debug_completion_steps(
            &completion_tree,
            &projected_line,
            cursor,
            osp_repl::CompletionDebugOptions::new(args.width, args.height)
                .ansi(args.menu_ansi)
                .unicode(args.menu_unicode)
                .appearance(Some(&appearance)),
            &steps,
        );
        serde_json::to_string_pretty(&frames).map_err(|err| miette!("{err:#}"))?
    };
    let payload = serde_json::from_str(&payload).map_err(|err| miette!("{err:#}"))?;
    Ok(CliCommandResult::document(document_from_json(payload)))
}

fn build_repl_completion_tree(view: ReplViewContext<'_>, surface: &ReplSurface) -> CompletionTree {
    completion::build_repl_completion_tree(view, surface)
}

fn build_repl_ui_line_projector(
    config: &ResolvedConfig,
) -> Arc<dyn Fn(&str) -> String + Send + Sync> {
    let config = config.clone();
    Arc::new(move |line| {
        input::project_repl_ui_line(line, &config).unwrap_or_else(|_| line.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::build_cycle_chrome_output;
    use crate::state::{
        AppRuntime, AppSession, AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind,
    };
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use osp_core::output::OutputFormat;
    use osp_ui::RenderSettings;
    use osp_ui::messages::MessageLevel;

    fn make_state() -> (AppRuntime, AppSession) {
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
            plugins: crate::plugin_manager::PluginManager::new(Vec::new()),
            themes: crate::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        });
        (state.runtime, state.session)
    }

    #[test]
    fn cycle_chrome_renders_intro_then_help_then_pending_output() {
        let (runtime, session) = make_state();
        let output = build_cycle_chrome_output(
            super::ReplViewContext::from_parts(&runtime, &session),
            "HELP\n",
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
        let (runtime, session) = make_state();
        let output = build_cycle_chrome_output(
            super::ReplViewContext::from_parts(&runtime, &session),
            "HELP\n",
            false,
            "PENDING\n",
        );

        assert_eq!(output, "PENDING\n");
    }
}
