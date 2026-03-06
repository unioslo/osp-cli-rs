pub(crate) mod completion;
pub(crate) mod dispatch;
pub(crate) mod help;
pub(crate) mod history;
pub(crate) mod input;
pub(crate) mod presentation;
pub(crate) mod surface;

use anyhow::anyhow;
use miette::{Result, miette};
use osp_repl::{DebugStep, ReplReloadKind, ReplRunResult, run_repl};

use crate::state::AppState;

use crate::app;
use crate::app::CliCommandResult;
use crate::cli::{DebugCompleteArgs, ReplArgs, ReplCommands};
#[cfg(test)]
pub(crate) use dispatch::{
    apply_repl_shell_prefix, execute_repl_plugin_line, leave_repl_shell, repl_command_spec,
};
#[cfg(test)]
pub(crate) use input::{ReplParsedLine, is_repl_shellable_command};
use osp_completion::CompletionTree;
#[cfg(test)]
pub(crate) use presentation::render_prompt_template;
#[cfg(test)]
pub(crate) use presentation::theme_display_name;
use presentation::{
    build_repl_appearance, build_repl_prompt, render_repl_command_overview, render_repl_intro,
};
use surface::ReplSurface;

struct ReplLoopState {
    show_intro: bool,
    pending_reload: bool,
    pending_output: String,
}

impl ReplLoopState {
    fn new(show_intro: bool) -> Self {
        Self {
            show_intro,
            pending_reload: false,
            pending_output: String::new(),
        }
    }

    fn prepare_cycle(&mut self, state: &mut AppState) -> Result<()> {
        if std::mem::take(&mut self.pending_reload) {
            let next = app::rebuild_repl_state(state)?;
            *state = next;
        }
        Ok(())
    }

    fn render_cycle_chrome(&mut self, state: &AppState, help_text: &str) {
        let output =
            build_cycle_chrome_output(state, help_text, self.show_intro, &self.pending_output);
        if !output.is_empty() {
            print!("{output}");
        }
        self.pending_output.clear();
    }

    fn apply_run_result(&mut self, result: ReplRunResult) -> Option<i32> {
        match result {
            ReplRunResult::Exit(code) => Some(code),
            ReplRunResult::Restart { output, reload } => {
                self.pending_reload = true;
                self.show_intro = matches!(reload, ReplReloadKind::WithIntro);
                if self.show_intro {
                    self.pending_output = output;
                } else if !output.is_empty() {
                    print!("{output}");
                }
                None
            }
        }
    }
}

fn build_cycle_chrome_output(
    state: &AppState,
    help_text: &str,
    show_intro: bool,
    pending_output: &str,
) -> String {
    let mut out = String::new();
    if show_intro {
        out.push_str("\x1b[2J\x1b[H");
        out.push_str(&render_repl_intro(state));
        out.push_str(help_text);
    }
    out.push_str(pending_output);
    out
}

pub(crate) fn run_plugin_repl(state: &mut AppState) -> Result<i32> {
    let mut loop_state = ReplLoopState::new(
        state
            .config
            .resolved()
            .get_bool("repl.intro")
            .unwrap_or(true),
    );

    loop {
        loop_state.prepare_cycle(state)?;
        let catalog = app::authorized_command_catalog(state)?;
        let surface = surface::build_repl_surface(state, &catalog);
        let completion_tree = build_repl_completion_tree(state, &surface);
        let help_text = render_repl_command_overview(state, &surface);

        loop_state.render_cycle_chrome(state, &help_text);

        let prompt = build_repl_prompt(state);
        let appearance = build_repl_appearance(state);
        let history_config = history::build_history_config(state);
        print!("Preparing prompt...\r");

        let result = run_repl(
            prompt,
            surface.root_words.clone(),
            Some(completion_tree),
            appearance,
            history_config,
            |line, history| {
                dispatch::execute_repl_plugin_line(state, history, line)
                    .map_err(|err| anyhow!("{err:#}"))
            },
        )
        .map_err(|err| miette!("{err:#}"))?;
        if let Some(code) = loop_state.apply_run_result(result) {
            return Ok(code);
        }
    }
}

pub(crate) fn run_repl_debug_command(state: &AppState, args: ReplArgs) -> Result<CliCommandResult> {
    match args.command {
        ReplCommands::DebugComplete(args) => run_repl_debug_complete(state, args),
    }
}

fn run_repl_debug_complete(state: &AppState, args: DebugCompleteArgs) -> Result<CliCommandResult> {
    let catalog = app::authorized_command_catalog(state)?;
    let surface = surface::build_repl_surface(state, &catalog);
    let completion_tree = build_repl_completion_tree(state, &surface);
    let appearance = build_repl_appearance(state);
    let cursor = args.cursor.unwrap_or(args.line.len());

    let steps = args
        .steps
        .iter()
        .map(|raw| DebugStep::parse(raw).ok_or_else(|| miette!("Unknown debug step '{raw}'")))
        .collect::<Result<Vec<_>>>()?;

    let payload = if steps.is_empty() {
        let debug = osp_repl::debug_completion(
            &completion_tree,
            &args.line,
            cursor,
            osp_repl::CompletionDebugOptions::new(args.width, args.height)
                .ansi(args.menu_ansi)
                .unicode(args.menu_unicode)
                .appearance(Some(&appearance)),
        );
        serde_json::to_string_pretty(&debug).map_err(|err| miette!("{err:#}"))?
    } else {
        let frames = osp_repl::debug_completion_steps(
            &completion_tree,
            &args.line,
            cursor,
            osp_repl::CompletionDebugOptions::new(args.width, args.height)
                .ansi(args.menu_ansi)
                .unicode(args.menu_unicode)
                .appearance(Some(&appearance)),
            &steps,
        );
        serde_json::to_string_pretty(&frames).map_err(|err| miette!("{err:#}"))?
    };
    Ok(CliCommandResult::text(format!("{payload}\n")))
}

fn build_repl_completion_tree(state: &AppState, surface: &ReplSurface) -> CompletionTree {
    completion::build_repl_completion_tree(state, surface)
}

#[cfg(test)]
mod tests {
    use super::build_cycle_chrome_output;
    use crate::state::{AppState, LaunchContext, RuntimeContext, TerminalKind};
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use osp_ui::messages::MessageLevel;
    use osp_ui::theme::DEFAULT_THEME_NAME;
    use osp_ui::{RenderRuntime, RenderSettings};

    fn make_state() -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        let settings = RenderSettings {
            format: OutputFormat::Json,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: osp_ui::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: osp_ui::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        };

        AppState::new(
            RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            settings,
            MessageLevel::Success,
            0,
            crate::plugin_manager::PluginManager::new(Vec::new()),
            crate::theme_loader::ThemeCatalog::default(),
            LaunchContext::default(),
        )
    }

    #[test]
    fn cycle_chrome_renders_intro_then_help_then_pending_output() {
        let state = make_state();
        let output = build_cycle_chrome_output(&state, "HELP\n", true, "PENDING\n");

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
        let state = make_state();
        let output = build_cycle_chrome_output(&state, "HELP\n", false, "PENDING\n");

        assert_eq!(output, "PENDING\n");
    }
}
