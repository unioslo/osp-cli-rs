//! `intro` builtin command implementation.
//!
//! This command reuses the REPL intro/presentation machinery outside the REPL
//! loop so operators can render the current intro/overview surface on demand.

use miette::Result;

use crate::app::CliCommandResult;
use crate::app::{AppClients, AppRuntime, AppSession, authorized_command_catalog_for};
use crate::cli::IntroArgs;
use crate::repl::ReplViewContext;
use crate::repl::presentation::build_repl_intro_payload;
use crate::repl::surface::ReplSurface;

pub(crate) struct IntroCommandContext<'a> {
    pub(crate) view: ReplViewContext<'a>,
    pub(crate) surface: ReplSurface,
}

impl<'a> IntroCommandContext<'a> {
    pub(crate) fn from_parts(
        runtime: &'a AppRuntime,
        session: &'a AppSession,
        clients: &'a AppClients,
        ui: &'a crate::app::UiState,
    ) -> Self {
        let view = ReplViewContext {
            config: runtime.config.resolved(),
            ui,
            auth: &runtime.auth,
            themes: &runtime.themes,
            scope: &session.scope,
            native_context: &session.native_context,
            history_enabled: session.history_enabled,
        };
        let surface = authorized_command_catalog_for(&runtime.auth, clients)
            .ok()
            .map(|catalog| crate::repl::surface::build_repl_surface(view, &catalog))
            .unwrap_or_else(|| crate::repl::surface::ReplSurface {
                root_words: Vec::new(),
                intro_commands: Vec::new(),
                specs: Vec::new(),
                aliases: Vec::new(),
                overview_entries: Vec::new(),
            });

        Self { view, surface }
    }
}

pub(crate) fn run_intro_command(
    context: IntroCommandContext<'_>,
    _args: IntroArgs,
) -> Result<CliCommandResult> {
    Ok(CliCommandResult::guide(build_repl_intro_payload(
        context.view,
        &context.surface,
        None,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        AppState, AppStateInit, LaunchContext, ReplCommandOutput, RuntimeContext, TerminalKind,
    };
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::native::NativeCommandRegistry;
    use crate::plugin::PluginManager;
    use crate::ui::RenderSettings;
    use crate::ui::build_presentation_defaults_layer;
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme_catalog::ThemeCatalog;

    fn resolved_repl_config() -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("repl.history.path", "/tmp/osp-intro-history.jsonl");
        defaults.set("theme.path", Vec::<String>::new());

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);

        let options = ResolveOptions::default().with_terminal("repl");
        let base = resolver
            .resolve(options.clone())
            .expect("base REPL config should resolve");
        resolver.set_presentation(build_presentation_defaults_layer(&base));
        resolver
            .resolve(options)
            .expect("presentation-seeded REPL config should resolve")
    }

    fn intro_state() -> AppState {
        AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config: resolved_repl_config(),
            render_settings: RenderSettings::test_plain(OutputFormat::Json),
            message_verbosity: MessageLevel::Success,
            error_detail: crate::app::ErrorDetail::Terse,
            debug_verbosity: 0,
            plugins: PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
            native_commands: NativeCommandRegistry::default(),
            themes: ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    #[test]
    fn intro_context_projects_visible_surface_for_repl_intro_unit() {
        let state = intro_state();

        let context = IntroCommandContext::from_parts(
            &state.runtime,
            &state.session,
            &state.clients,
            state.runtime.ui(),
        );

        assert!(context.surface.root_words.iter().any(|word| word == "help"));
        assert!(
            context
                .surface
                .intro_commands
                .iter()
                .all(|word| word != "exit" && word != "quit")
        );
        assert!(
            context
                .surface
                .overview_entries
                .iter()
                .any(|entry| entry.name == "help")
        );
    }

    #[test]
    fn intro_command_returns_structured_guide_output_unit() {
        let state = intro_state();
        let context = IntroCommandContext::from_parts(
            &state.runtime,
            &state.session,
            &state.clients,
            state.runtime.ui(),
        );

        let result =
            run_intro_command(context, IntroArgs::default()).expect("intro command should succeed");

        assert_eq!(result.exit_code, 0);
        let ReplCommandOutput::Output(structured) = result
            .output
            .expect("intro command should emit structured output")
        else {
            panic!("intro command should return guide output");
        };
        assert!(structured.source_guide.is_some());
    }
}
