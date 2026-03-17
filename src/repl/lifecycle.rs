use crate::completion::CompletionTree;
use crate::repl::{ReplAppearance, ReplReloadKind, ReplRunConfig, ReplRunResult};
use miette::Result;

use crate::app;
use crate::app::sink::UiSink;
use crate::app::{AppClients, AppRuntime, AppSession, AppState};

use super::ReplViewContext;
use super::completion;
use super::history;
use super::input;
use super::presentation::{
    ReplInputMode, build_repl_appearance, build_repl_prompt, build_repl_prompt_right_renderer,
    render_repl_intro, repl_input_mode,
};
use super::surface;

pub(super) struct ReplLoopState {
    show_intro: bool,
    pending_reload: bool,
    pending_output: String,
}

impl ReplLoopState {
    pub(super) fn new(show_intro: bool) -> Self {
        Self {
            show_intro,
            pending_reload: false,
            pending_output: String::new(),
        }
    }

    pub(super) fn prepare_cycle(&mut self, state: &mut AppState) -> Result<ReplCycle> {
        if std::mem::take(&mut self.pending_reload) {
            app::rebuild_repl_in_place(state)?;
        }
        ReplCycle::prepare(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            self.show_intro,
        )
    }

    pub(super) fn render_cycle_chrome(&mut self, sink: &mut dyn UiSink, help_text: &str) {
        let output = build_cycle_chrome_output(help_text, self.show_intro, &self.pending_output);
        if !output.is_empty() {
            sink.write_stdout(&output);
        }
        self.pending_output.clear();
    }

    pub(super) fn apply_run_result(
        &mut self,
        sink: &mut dyn UiSink,
        result: ReplRunResult,
    ) -> Option<i32> {
        match result {
            ReplRunResult::Exit(code) => Some(code),
            ReplRunResult::Restart { output, reload } => {
                self.pending_reload = true;
                self.show_intro = matches!(reload, ReplReloadKind::WithIntro);
                if self.show_intro {
                    self.pending_output = output;
                } else if !output.is_empty() {
                    sink.write_stdout(&output);
                }
                None
            }
        }
    }
}

pub(super) struct ReplCycle {
    pub(super) run_config: ReplRunConfig,
    pub(super) help_text: String,
}

pub(super) struct PreparedReplSurfaceState {
    pub(super) surface: surface::ReplSurface,
    pub(super) completion_tree: CompletionTree,
    pub(super) appearance: ReplAppearance,
}

impl ReplCycle {
    fn prepare(
        runtime: &AppRuntime,
        session: &AppSession,
        clients: &AppClients,
        include_help_text: bool,
    ) -> Result<Self> {
        let prepared = prepare_repl_surface_state(runtime, session, clients)?;
        let view = ReplViewContext::from_parts(runtime, session);
        let intro_text = if include_help_text {
            render_repl_intro(view, &prepared.surface)
        } else {
            String::new()
        };

        Ok(Self {
            run_config: build_repl_cycle_run_config(runtime, session, prepared),
            help_text: intro_text,
        })
    }
}

fn build_repl_cycle_run_config(
    runtime: &AppRuntime,
    session: &AppSession,
    prepared: PreparedReplSurfaceState,
) -> ReplRunConfig {
    let view = ReplViewContext::from_parts(runtime, session);

    ReplRunConfig::builder(
        build_repl_prompt(view),
        history::build_history_config(runtime, session),
    )
    .with_completion_words(prepared.surface.root_words.clone())
    .with_completion_tree(Some(prepared.completion_tree))
    .with_appearance(prepared.appearance)
    .with_input_mode(map_repl_input_mode(repl_input_mode(
        runtime.config.resolved(),
    )))
    .with_prompt_right(Some(build_repl_prompt_right_renderer(
        view,
        session.prompt_timing.clone(),
    )))
    .with_line_projector(Some(input::build_repl_ui_line_projector(
        runtime.config.resolved(),
    )))
    .build()
}

pub(super) fn prepare_repl_surface_state(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &AppClients,
) -> Result<PreparedReplSurfaceState> {
    let catalog = app::authorized_command_catalog_for(&runtime.auth, clients)?;
    let view = ReplViewContext::from_parts(runtime, session);
    let surface = surface::build_repl_surface(view, &catalog);
    let completion_tree = completion::build_repl_completion_tree(view, &surface)?;

    Ok(PreparedReplSurfaceState {
        appearance: build_repl_appearance(view),
        completion_tree,
        surface,
    })
}

fn map_repl_input_mode(mode: ReplInputMode) -> crate::repl::ReplInputMode {
    match mode {
        ReplInputMode::Auto => crate::repl::ReplInputMode::Auto,
        ReplInputMode::Interactive => crate::repl::ReplInputMode::Interactive,
        ReplInputMode::Basic => crate::repl::ReplInputMode::Basic,
    }
}

pub(crate) fn build_cycle_chrome_output(
    help_text: &str,
    show_intro: bool,
    pending_output: &str,
) -> String {
    let mut out = String::new();
    if show_intro {
        out.push_str("\x1b[2J\x1b[H");
        out.push_str(help_text);
    }
    out.push_str(pending_output);
    out
}

#[cfg(test)]
mod tests {
    use super::{ReplLoopState, build_cycle_chrome_output};
    use crate::app::sink::BufferedUiSink;
    use crate::app::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::repl::{ReplReloadKind, ReplRunResult};
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme_catalog::ThemeCatalog;

    #[test]
    fn apply_run_result_handles_exit_and_restart_modes() {
        let mut loop_state = ReplLoopState::new(true);
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            loop_state.apply_run_result(&mut sink, ReplRunResult::Exit(7)),
            Some(7)
        );

        let mut loop_state = ReplLoopState::new(false);
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            loop_state.apply_run_result(
                &mut sink,
                ReplRunResult::Restart {
                    output: "hello".to_string(),
                    reload: ReplReloadKind::WithIntro,
                }
            ),
            None
        );
        assert!(loop_state.pending_reload);
        assert!(loop_state.show_intro);
        assert_eq!(loop_state.pending_output, "hello");
        assert!(sink.stdout.is_empty());

        let mut loop_state = ReplLoopState::new(true);
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            loop_state.apply_run_result(
                &mut sink,
                ReplRunResult::Restart {
                    output: "ignored".to_string(),
                    reload: ReplReloadKind::Default,
                }
            ),
            None
        );
        assert!(loop_state.pending_reload);
        assert!(!loop_state.show_intro);
        assert!(loop_state.pending_output.is_empty());
        assert_eq!(sink.stdout, "ignored");
    }

    #[test]
    fn build_cycle_chrome_output_includes_intro_help_and_pending_output() {
        let rendered = build_cycle_chrome_output("Commands\n", true, "Queued\n");
        assert!(rendered.starts_with("\x1b[2J\x1b[H"));
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("Queued"));
    }

    #[test]
    fn build_cycle_chrome_output_skips_intro_when_not_requested() {
        let rendered = build_cycle_chrome_output("Commands\n", false, "Queued\n");
        assert_eq!(rendered, "Queued\n");
    }

    #[test]
    fn prepare_cycle_handles_reload_and_builds_surface_unit() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("config should resolve");

        let mut state = AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            render_settings: RenderSettings::test_plain(OutputFormat::Table),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins: crate::plugin::PluginManager::new(Vec::new()),
            native_commands: crate::native::NativeCommandRegistry::default(),
            themes: ThemeCatalog::default(),
            launch: LaunchContext::default(),
        });

        let mut loop_state = ReplLoopState::new(true);
        let first = loop_state
            .prepare_cycle(&mut state)
            .expect("initial cycle should build");
        assert!(!first.run_config.completion_words.is_empty());
        assert!(first.run_config.completion_tree.is_some());
        assert!(first.help_text.contains("help") || first.help_text.contains("config"));

        loop_state.apply_run_result(
            &mut BufferedUiSink::default(),
            ReplRunResult::Restart {
                output: String::new(),
                reload: ReplReloadKind::Default,
            },
        );

        let second = loop_state
            .prepare_cycle(&mut state)
            .expect("reloaded cycle should build");
        assert!(!second.run_config.completion_words.is_empty());
        assert!(second.run_config.completion_tree.is_some());
    }
}
