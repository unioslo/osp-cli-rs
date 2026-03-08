use crate::osp_completion::CompletionTree;
use crate::osp_repl::{HistoryConfig, ReplAppearance, ReplPrompt, ReplReloadKind, ReplRunResult};
use miette::Result;

use crate::osp_cli::app;
use crate::osp_cli::state::{AppClients, AppRuntime, AppSession};
use crate::osp_cli::ui_sink::UiSink;

use super::ReplViewContext;
use super::completion;
use super::history;
use super::presentation::{
    build_repl_appearance, build_repl_prompt, render_repl_command_overview, render_repl_intro,
};
use super::surface;
use crate::osp_cli::ui_presentation::repl_intro_includes_overview;

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

    pub(super) fn prepare_cycle(
        &mut self,
        runtime: &mut AppRuntime,
        session: &mut AppSession,
        clients: &mut AppClients,
    ) -> Result<ReplCycle> {
        if std::mem::take(&mut self.pending_reload) {
            let (next_runtime, next_session, next_clients) =
                app::rebuild_repl_parts(runtime, session)?;
            *runtime = next_runtime;
            *session = next_session;
            *clients = next_clients;
        }
        ReplCycle::prepare(runtime, session, clients, self.show_intro)
    }

    pub(super) fn render_cycle_chrome(
        &mut self,
        sink: &mut dyn UiSink,
        view: ReplViewContext<'_>,
        help_text: &str,
    ) {
        let output =
            build_cycle_chrome_output(view, help_text, self.show_intro, &self.pending_output);
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
    pub(super) prompt: ReplPrompt,
    pub(super) root_words: Vec<String>,
    pub(super) completion_tree: CompletionTree,
    pub(super) appearance: ReplAppearance,
    pub(super) history_config: HistoryConfig,
    pub(super) help_text: String,
}

impl ReplCycle {
    fn prepare(
        runtime: &mut AppRuntime,
        session: &mut AppSession,
        clients: &AppClients,
        include_help_text: bool,
    ) -> Result<Self> {
        let catalog = app::authorized_command_catalog_for(&runtime.auth, &clients.plugins)?;
        let view = ReplViewContext::from_parts(runtime, session);
        let surface = surface::build_repl_surface(view, &catalog);
        let completion_tree = completion::build_repl_completion_tree(view, &surface);
        let help_text = if include_help_text
            && repl_intro_includes_overview(
                crate::osp_cli::ui_presentation::intro_style_with_verbosity(
                    crate::osp_cli::ui_presentation::intro_style(runtime.config.resolved()),
                    runtime.ui.message_verbosity,
                ),
            ) {
            render_repl_command_overview(view, &surface)
        } else {
            String::new()
        };
        let intro_text = if include_help_text {
            render_repl_intro(view, &surface.intro_commands)
        } else {
            String::new()
        };

        Ok(Self {
            prompt: build_repl_prompt(view),
            root_words: surface.root_words.clone(),
            completion_tree,
            appearance: build_repl_appearance(view),
            history_config: history::build_history_config(runtime, session),
            help_text: format!("{intro_text}{help_text}"),
        })
    }
}

pub(crate) fn build_cycle_chrome_output(
    _view: ReplViewContext<'_>,
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
    use crate::osp_cli::repl::ReplViewContext;
    use crate::osp_cli::state::{AuthState, ReplScopeStack, UiState};
    use crate::osp_cli::theme_loader::ThemeCatalog;
    use crate::osp_cli::ui_sink::BufferedUiSink;
    use crate::osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::osp_core::output::OutputFormat;
    use crate::osp_repl::{ReplReloadKind, ReplRunResult};
    use crate::osp_ui::RenderSettings;
    use crate::osp_ui::messages::MessageLevel;

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
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let resolved = resolver
            .resolve(ResolveOptions::default())
            .expect("config should resolve");
        let themes = ThemeCatalog::default();
        let ui = UiState {
            render_settings: RenderSettings::test_plain(OutputFormat::Table),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
        };
        let auth = AuthState::from_resolved(&resolved);
        let scope = ReplScopeStack::default();
        let view = ReplViewContext {
            config: &resolved,
            ui: &ui,
            auth: &auth,
            themes: &themes,
            scope: &scope,
        };

        let rendered = build_cycle_chrome_output(view, "Commands\n", true, "Queued\n");
        assert!(rendered.starts_with("\x1b[2J\x1b[H"));
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("Queued"));
    }

    #[test]
    fn build_cycle_chrome_output_skips_intro_when_not_requested() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let resolved = resolver
            .resolve(ResolveOptions::default())
            .expect("config should resolve");
        let themes = ThemeCatalog::default();
        let ui = UiState {
            render_settings: RenderSettings::test_plain(OutputFormat::Table),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
        };
        let auth = AuthState::from_resolved(&resolved);
        let scope = ReplScopeStack::default();
        let view = ReplViewContext {
            config: &resolved,
            ui: &ui,
            auth: &auth,
            themes: &themes,
            scope: &scope,
        };

        let rendered = build_cycle_chrome_output(view, "Commands\n", false, "Queued\n");
        assert_eq!(rendered, "Queued\n");
    }
}
