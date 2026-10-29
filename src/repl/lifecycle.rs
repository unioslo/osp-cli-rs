//! REPL restart and cycle lifecycle helpers.
//!
//! The interactive loop needs a small state machine around "prepare one run,
//! render intro/pending output, apply restart result". This module keeps that
//! orchestration separate from the editor engine and from command execution so
//! the host loop stays readable.

use crate::completion::CompletionTree;
use crate::repl::{ReplAppearance, ReplReloadKind, ReplResult, ReplRunConfig};
use miette::Result;

use crate::app;
use crate::app::sink::UiSink;
use crate::app::{AppClients, AppRuntime, AppSession, AppState};

use super::ReplViewContext;
use super::completion;
use super::history;
use super::input;
use super::presentation::{
    build_repl_appearance, build_repl_prompt, build_repl_prompt_right_renderer, render_repl_intro,
    repl_autosuggest, repl_complete_on, repl_input_mode,
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
            &state.runtime,
            &state.session,
            &state.clients,
            self.show_intro,
        )
    }

    pub(super) fn render_cycle_chrome(
        &mut self,
        sink: &mut dyn UiSink,
        help_text: &str,
        clear_screen: bool,
    ) {
        let output = build_cycle_chrome_output(
            help_text,
            self.show_intro,
            &self.pending_output,
            clear_screen,
        );
        if !output.is_empty() {
            sink.write_stdout(&output);
        }
        self.pending_output.clear();
    }

    pub(super) fn apply_run_result(
        &mut self,
        sink: &mut dyn UiSink,
        result: ReplResult,
        exit_message: &str,
    ) -> Option<i32> {
        match result {
            ReplResult::Continue | ReplResult::Print(_) | ReplResult::ReplaceInput(_) => {
                unreachable!("the editor consumes continuation actions")
            }
            ReplResult::Exit(code) => {
                let message = exit_message.trim_end();
                if !message.is_empty() {
                    sink.write_stdout(message);
                    sink.write_stdout("\n");
                }
                Some(code)
            }
            ReplResult::Restart { output, reload } => {
                self.pending_reload = !matches!(reload, ReplReloadKind::Completion);
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
    pub(super) clear_screen_on_intro: bool,
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
        let prepared = {
            let _phase =
                tracing::info_span!(target: "osp_startup", "REPL completion surface").entered();
            prepare_repl_surface_state(runtime, session, clients)?
        };
        let actions = resolved_product_actions(runtime, clients)?;
        let view = ReplViewContext {
            actions: &actions,
            ..ReplViewContext::from_parts(runtime, session)
        };
        let intro_text = if include_help_text {
            render_repl_intro(view, &prepared.surface)
        } else {
            String::new()
        };

        Ok(Self {
            run_config: build_repl_cycle_run_config(runtime, session, clients, prepared, actions)?,
            help_text: intro_text,
            clear_screen_on_intro: runtime.ui.render_settings.runtime.stdout_is_tty,
        })
    }
}

fn hint_insert_action(config: &crate::config::ResolvedConfig) -> Result<crate::repl::ReplAction> {
    let mut hint =
        crate::repl::ReplAction::resolve("hint.insert", "Insert suggested command", 2, config)
            .map_err(|e| miette::miette!("{e:#}"))?;
    if config.get_string("repl.bindings.hint.insert").is_none() {
        hint.key = Some(crate::repl::ReplActionKey::AltEnter);
    }
    Ok(hint)
}

pub(crate) fn hint_action(
    config: &crate::config::ResolvedConfig,
    format: crate::core::OutputFormat,
    choices: usize,
) -> Option<String> {
    if (1..=3).contains(&choices)
        && format != crate::core::OutputFormat::Json
        && let Ok(action) = hint_insert_action(config)
        && let Some(key) = action.key
    {
        let cycle = if choices > 1 { ", again to cycle" } else { "" };
        Some(format!(" ({key} inserts{cycle})"))
    } else {
        None
    }
}

pub(crate) fn resolved_product_actions(
    runtime: &AppRuntime,
    clients: &AppClients,
) -> Result<Vec<crate::repl::ReplAction>> {
    let mut actions = Vec::new();
    use std::io::IsTerminal;
    if runtime.ui.render_settings.requested_format() == crate::core::output::OutputFormat::Json
        || repl_input_mode(runtime.config.resolved()) == crate::repl::ReplInputMode::Basic
        || !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal())
    {
        return Ok(actions);
    }
    actions.push(hint_insert_action(runtime.config.resolved())?);
    for (id, label, key) in [
        ("output.page", "Page last output", 'l'),
        ("output.editor", "View last output in editor", 'e'),
        ("output.copy", "Copy last output", 'y'),
    ] {
        let mut action = crate::repl::ReplAction::resolve(id, label, 2, runtime.config.resolved())
            .map_err(|e| miette::miette!("{e:#}"))?;
        if runtime
            .config
            .resolved()
            .get_string(&format!("repl.bindings.{id}"))
            .is_none()
        {
            action.key = Some(crate::repl::ReplActionKey::Alt(key));
        }
        actions.push(action);
    }
    for (_, command) in crate::app::runtime::visible_native_commands(&runtime.auth, clients) {
        actions.extend(
            command
                .repl_actions(runtime.config.resolved())
                .map_err(|e| miette::miette!("{e:#}"))?,
        );
    }
    Ok(actions)
}

fn build_repl_cycle_run_config(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &AppClients,
    prepared: PreparedReplSurfaceState,
    actions: Vec<crate::repl::ReplAction>,
) -> Result<ReplRunConfig> {
    session.native_context.activate(runtime);
    for (name, command) in clients.native_commands().iter() {
        session.native_context.register_context(
            name,
            command.context_origin(runtime.config.resolved()),
            command.persisted_context_kinds(),
            &command.command(),
            command.context_kinds(),
        );
    }
    session
        .native_context
        .load_context()
        .map_err(|e| miette::miette!("{e:#}"))?;
    for (_, command) in clients.native_commands().iter() {
        command
            .migrate_context(&session.native_context)
            .map_err(|e| miette::miette!("{e:#}"))?;
    }
    seed_native_context(runtime, session, clients)?;
    let view = ReplViewContext::from_parts(runtime, session);

    let product_status =
        if runtime.ui.render_settings.requested_format() == crate::core::output::OutputFormat::Json
        {
            None
        } else {
            let commands = crate::app::runtime::visible_native_commands(&runtime.auth, clients)
                .map(|(_, command)| command.clone())
                .collect::<Vec<_>>();
            Some(std::sync::Arc::new(move || {
                commands.iter().find_map(|command| command.repl_status())
            }) as super::product::ReplStatusRenderer)
        };
    let mut config = ReplRunConfig::builder(
        build_repl_prompt(view),
        history::build_history_config(runtime, session),
    )
    .with_completion_tree(prepared.completion_tree)
    .with_appearance(prepared.appearance)
    .with_input_mode(repl_input_mode(runtime.config.resolved()))
    .with_complete_on(repl_complete_on(runtime.config.resolved()))
    .with_autosuggest(repl_autosuggest(runtime.config.resolved()))
    .with_prompt_right(Some(build_repl_prompt_right_renderer(
        view,
        session.prompt_timing.clone(),
        product_status,
    )))
    .with_line_projector(Some(input::build_repl_ui_line_projector(
        runtime.config.resolved(),
    )))
    .build();
    config.exit_on_interrupt = runtime.definition.repl_line_handler.is_some();
    config.actions = actions;
    if runtime.ui.render_settings.requested_format() != crate::core::output::OutputFormat::Json {
        let commands = crate::app::runtime::visible_native_commands(&runtime.auth, clients)
            .map(|(_, command)| command.clone())
            .collect::<Vec<_>>();
        let resolved = runtime.config.resolved().clone();
        config.on_start = Some(std::sync::Arc::new(move |session| {
            for command in &commands {
                command.start_repl(&resolved, session.clone());
            }
        }));
    }
    Ok(config)
}

pub(super) fn prepare_repl_surface_state(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &AppClients,
) -> Result<PreparedReplSurfaceState> {
    let catalog = app::authorized_command_catalog_for(
        &runtime.auth,
        clients,
        clients.native_commands().completion_catalog(),
    );
    let view = ReplViewContext::from_parts(runtime, session);
    let surface = surface::build_repl_surface(view, &catalog);
    let completion_tree = completion::build_repl_completion_tree(view, &surface)?;

    Ok(PreparedReplSurfaceState {
        appearance: build_repl_appearance(view),
        completion_tree,
        surface,
    })
}

pub(crate) fn build_cycle_chrome_output(
    help_text: &str,
    show_intro: bool,
    pending_output: &str,
    clear_screen: bool,
) -> String {
    let mut out = String::new();
    if show_intro {
        if clear_screen {
            out.push_str(&super::screen::clear_sequence());
        }
        out.push_str(help_text);
    }
    out.push_str(pending_output);
    out
}

/// Fill only absent selections, using the active identity and cached product data.
pub(super) fn seed_native_context(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &AppClients,
) -> Result<()> {
    session.native_context.activate(runtime);
    let enabled = runtime
        .config
        .resolved()
        .get_string_list("context.seed")
        .unwrap_or_default();
    if enabled.is_empty() {
        return Ok(());
    }
    let identity = runtime
        .access_policy()
        .map(|policy| policy.effective_identity(runtime.config.resolved()))
        .unwrap_or_default();
    let mut changed = false;
    for (name, command) in crate::app::runtime::visible_native_commands(&runtime.auth, clients) {
        let selected = session.native_context.for_service(name);
        for (kind, value) in command.context_seeds(runtime.config.resolved(), &identity) {
            if enabled.contains(&kind)
                && crate::core::context::KINDS.contains(&kind.as_str())
                && crate::context::valid_value(&value)
                && !session
                    .native_context
                    .context_entries()
                    .iter()
                    .any(|entry| entry.kind == kind)
            {
                selected.set_context_value(kind, value);
                changed = true;
            }
        }
    }
    if changed {
        session
            .native_context
            .save_context()
            .map_err(|e| miette::miette!("{e:#}"))?;
    }
    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::{ReplLoopState, build_cycle_chrome_output};
    use crate::app::sink::BufferedUiSink;
    use crate::app::{LaunchContext, RuntimeContext, TerminalKind};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::repl::{ReplReloadKind, ReplResult};
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme_catalog::ThemeCatalog;

    #[test]
    fn apply_run_result_handles_exit_and_restart_modes() {
        let mut loop_state = ReplLoopState::new(true);
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            loop_state.apply_run_result(&mut sink, ReplResult::Exit(7), "Goodbye.\n"),
            Some(7)
        );
        assert_eq!(sink.stdout, "Goodbye.\n");

        let mut silent = ReplLoopState::new(false);
        let mut silent_sink = BufferedUiSink::default();
        assert_eq!(
            silent.apply_run_result(&mut silent_sink, ReplResult::Exit(0), ""),
            Some(0)
        );
        assert!(silent_sink.stdout.is_empty());

        let mut loop_state = ReplLoopState::new(false);
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            loop_state.apply_run_result(
                &mut sink,
                ReplResult::Restart {
                    output: "hello".to_string(),
                    reload: ReplReloadKind::WithIntro,
                },
                "unused\n",
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
                ReplResult::Restart {
                    output: "ignored".to_string(),
                    reload: ReplReloadKind::Default,
                },
                "unused\n",
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
        let rendered = build_cycle_chrome_output("Commands\n", true, "Queued\n", true);
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("Queued"));
    }

    #[test]
    fn build_cycle_chrome_output_skips_intro_when_not_requested() {
        let rendered = build_cycle_chrome_output("Commands\n", false, "Queued\n", false);
        assert_eq!(rendered, "Queued\n");
    }

    #[test]
    fn build_cycle_chrome_output_omits_ansi_clear_when_disabled() {
        let rendered = build_cycle_chrome_output("Commands\n", true, "Queued\n", false);
        assert!(rendered.starts_with("Commands\n"));
    }

    #[test]
    fn prepare_cycle_handles_reload_and_builds_surface_unit() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("repl.history.path", "/tmp/osp-repl-lifecycle-history.jsonl");
        defaults.set("theme.path", Vec::<String>::new());
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("config should resolve");

        let mut state = crate::app::AppStateBuilder {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            launch: LaunchContext::default(),
            native_commands: crate::native::NativeCommandRegistry::default(),
            ui: crate::app::UiState::new(
                RenderSettings::test_plain(OutputFormat::Table),
                MessageLevel::Success,
                0,
            ),
            plugins: Some(
                crate::plugin::PluginManager::new(Vec::new())
                    .with_bundled_roots(false)
                    .with_default_roots(false),
            ),
            themes: Some(ThemeCatalog::default()),
            session: None,
            definition: Default::default(),
        }
        .build();

        let mut loop_state = ReplLoopState::new(true);
        let first = loop_state
            .prepare_cycle(&mut state)
            .expect("initial cycle should build");
        assert!(!first.run_config.completion_tree.root.children.is_empty());
        assert!(first.help_text.contains("help") || first.help_text.contains("config"));

        loop_state.apply_run_result(
            &mut BufferedUiSink::default(),
            ReplResult::Restart {
                output: String::new(),
                reload: ReplReloadKind::Default,
            },
            "",
        );

        let second = loop_state
            .prepare_cycle(&mut state)
            .expect("reloaded cycle should build");
        assert!(!second.run_config.completion_tree.root.children.is_empty());
    }
}
