use miette::Result;
use osp_completion::CompletionTree;
use osp_repl::{HistoryConfig, ReplAppearance, ReplPrompt, ReplReloadKind, ReplRunResult};

use crate::app;
use crate::state::AppState;

use super::completion;
use super::history;
use super::presentation::{
    build_repl_appearance, build_repl_prompt, render_repl_command_overview, render_repl_intro,
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
            *state = app::rebuild_repl_state(state)?;
        }
        ReplCycle::prepare(state)
    }

    pub(super) fn render_cycle_chrome(&mut self, state: &AppState, help_text: &str) {
        let output =
            build_cycle_chrome_output(state, help_text, self.show_intro, &self.pending_output);
        if !output.is_empty() {
            print!("{output}");
        }
        self.pending_output.clear();
    }

    pub(super) fn apply_run_result(&mut self, result: ReplRunResult) -> Option<i32> {
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

pub(super) struct ReplCycle {
    pub(super) prompt: ReplPrompt,
    pub(super) root_words: Vec<String>,
    pub(super) completion_tree: CompletionTree,
    pub(super) appearance: ReplAppearance,
    pub(super) history_config: HistoryConfig,
    pub(super) help_text: String,
}

impl ReplCycle {
    fn prepare(state: &mut AppState) -> Result<Self> {
        let catalog = app::authorized_command_catalog(state)?;
        let surface = surface::build_repl_surface(state, &catalog);
        let completion_tree = completion::build_repl_completion_tree(state, &surface);
        let help_text = render_repl_command_overview(state, &surface);

        Ok(Self {
            prompt: build_repl_prompt(state),
            root_words: surface.root_words.clone(),
            completion_tree,
            appearance: build_repl_appearance(state),
            history_config: history::build_history_config(state),
            help_text,
        })
    }
}

pub(crate) fn build_cycle_chrome_output(
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
