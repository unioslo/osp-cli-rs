use super::{
    ReplIntroStyle, build_repl_appearance, build_repl_intro_payload, build_repl_prompt,
    build_repl_prompt_right_renderer, render_prompt_template, render_repl_command_overview,
    render_repl_intro, render_repl_prompt_right_for_test,
    render_repl_prompt_right_for_test_with_context, theme_display_name,
};
mod output_support {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/support/output.rs"
    ));
}
use crate::app::TimingSummary;
use crate::app::{
    AppState, DebugTimingBadge, DebugTimingState, LaunchContext, RuntimeContext, TerminalKind,
};
use crate::app::{CMD_CONFIG, CMD_HELP, CMD_PLUGINS, CMD_THEME};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
use crate::core::document::{DocBlock, Document, DocumentEntry};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::plugin::PluginManager;
use crate::repl::ReplViewContext;
use crate::repl::surface::{ReplOverviewEntry, ReplSurface};
use crate::ui::RenderSettings;
use crate::ui::build_presentation_defaults_layer;
use crate::ui::messages::MessageLevel;
use serde_json::json;
use std::time::Duration;

const FULL_INTRO_TEMPLATE_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/repl_intro/full_intro_template.md"
));

fn make_state(entries: &[(&str, &str)]) -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set(
        "repl.history.path",
        "/tmp/osp-repl-presentation-history.jsonl",
    );
    defaults.set("theme.path", Vec::<String>::new());
    let mut session = ConfigLayer::default();
    for (key, value) in entries {
        session.set(*key, *value);
    }
    let mut layers = crate::config::LoadedLayers {
        defaults,
        session,
        ..crate::config::LoadedLayers::default()
    };
    let options = ResolveOptions::default().with_terminal("repl");
    let base = ConfigResolver::from_loaded_layers_with_schema(
        layers.clone(),
        crate::config::ConfigSchema::default(),
    )
    .resolve(options.clone())
    .expect("base test config should resolve");
    layers.presentation = build_presentation_defaults_layer(&base);
    let config = ConfigResolver::from_loaded_layers_with_schema(
        layers.clone(),
        crate::config::ConfigSchema::default(),
    )
    .resolve(options)
    .expect("test config should resolve");

    crate::app::AppStateBuilder {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        launch: LaunchContext::default(),
        native_commands: crate::native::NativeCommandRegistry::default(),
        ui: crate::app::UiState::new(
            RenderSettings::test_plain(OutputFormat::Json),
            MessageLevel::Success,
            0,
        ),
        plugins: Some(
            PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
        ),
        themes: Some(crate::ui::theme_catalog::ThemeCatalog::default()),
        session: None,
        definition: Default::default(),
    }
    .build()
}

fn repl_view<'a>(state: &'a AppState) -> ReplViewContext<'a> {
    ReplViewContext::from_parts(&state.runtime, &state.session)
}

fn configure_render_runtime(
    state: &mut AppState,
    mode: RenderMode,
    color: ColorMode,
    unicode: UnicodeMode,
) {
    state.runtime.ui.render_settings.format = OutputFormat::Auto;
    state.runtime.ui.render_settings.mode = mode;
    state.runtime.ui.render_settings.color = color;
    state.runtime.ui.render_settings.unicode = unicode;
    state.runtime.ui.render_settings.width = Some(80);
    state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    state.runtime.ui.render_settings.runtime.terminal = Some("xterm-256color".to_string());
    state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);
    state.runtime.ui.render_settings.runtime.no_color = false;
}

fn make_intro_state(
    entries: &[(&str, &str)],
    mode: RenderMode,
    color: ColorMode,
    unicode: UnicodeMode,
) -> AppState {
    let mut state = make_state(entries);
    configure_render_runtime(&mut state, mode, color, unicode);
    crate::ui::settings::apply_render_config_overrides(
        &mut state.runtime.ui.render_settings,
        state.runtime.config.resolved(),
    );
    let theme_name = state
        .runtime
        .config
        .resolved()
        .get_string("theme.name")
        .unwrap_or(crate::ui::theme::DEFAULT_THEME_NAME)
        .to_string();
    state.runtime.ui.render_settings.theme_name = theme_name.clone();
    state.runtime.ui.render_settings.theme = Some(crate::ui::theme::resolve_theme(&theme_name));
    state
}

fn intro_commands(commands: &[&str]) -> Vec<String> {
    commands.iter().map(|value| (*value).to_string()).collect()
}

fn intro_surface(commands: &[&str]) -> ReplSurface {
    ReplSurface {
        root_words: intro_commands(commands),
        intro_commands: intro_commands(commands),
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: Vec::new(),
    }
}

fn intro_surface_with_overview(commands: &[&str]) -> ReplSurface {
    ReplSurface {
        root_words: intro_commands(commands),
        intro_commands: intro_commands(commands),
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: vec![
            ReplOverviewEntry {
                name: "help".to_string(),
                summary: "Show this command overview.".to_string(),
            },
            ReplOverviewEntry {
                name: "config".to_string(),
                summary: "Show and change config.".to_string(),
            },
            ReplOverviewEntry {
                name: "theme".to_string(),
                summary: "List and use themes.".to_string(),
            },
            ReplOverviewEntry {
                name: "plugins".to_string(),
                summary: "List, enable, and inspect plugins.".to_string(),
            },
        ],
    }
}

fn overview_surface(entries: Vec<ReplOverviewEntry>) -> ReplSurface {
    ReplSurface {
        root_words: intro_commands(&["help", "config", "theme"]),
        intro_commands: intro_commands(&["help", "config", "theme"]),
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: entries,
    }
}

fn rich_prompt_right_settings() -> crate::ui::ResolvedRenderSettings {
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.mode = crate::core::output::RenderMode::Rich;
    settings.color = crate::core::output::ColorMode::Always;
    settings.unicode = crate::core::output::UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.runtime.locale_utf8 = Some(true);
    settings.resolve_render_settings()
}

mod shape_contracts {
    use super::{
        ColorMode, MessageLevel, RenderMode, UnicodeMode, intro_surface_with_overview,
        make_intro_state, output_support, render_repl_intro, repl_view,
    };

    fn render_style(
        intro: Option<&str>,
        presentation: Option<&str>,
        verbosity: MessageLevel,
    ) -> String {
        let mut entries = Vec::new();
        if let Some(intro) = intro {
            entries.push(("repl.intro", intro));
        }
        if let Some(presentation) = presentation {
            entries.push(("ui.presentation", presentation));
        }
        let mut state = make_intro_state(
            &entries,
            RenderMode::Rich,
            ColorMode::Always,
            UnicodeMode::Always,
        );
        state.runtime.ui.verbosity = verbosity;
        output_support::strip_ansi(&render_repl_intro(
            repl_view(&state),
            &intro_surface_with_overview(&["help", "config", "theme", "plugins"]),
        ))
    }

    #[test]
    fn presentation_and_verbosity_matrix_select_expected_intro_shapes() {
        let expressive = render_style(None, Some("expressive"), MessageLevel::Success);
        assert!(expressive.contains("Keybindings"));
        assert!(expressive.contains("Usage"));

        let compact = render_style(None, Some("compact"), MessageLevel::Success);
        assert!(compact.contains("Usage:"));
        assert!(compact.contains("[INVOCATION_OPTIONS] COMMAND [ARGS]..."));
        assert!(compact.contains("Commands:"));
        assert!(!compact.contains("Welcome anonymous."));
        assert!(!compact.contains("Keybindings"));

        let austere = render_style(None, Some("austere"), MessageLevel::Success);
        assert!(austere.contains("Welcome anonymous."));
        assert!(!austere.contains("Usage"));

        let austere_trace = render_style(None, Some("austere"), MessageLevel::Trace);
        assert!(austere_trace.contains("Keybindings"));
        assert!(austere_trace.contains("Usage"));

        let austere_warning = render_style(None, Some("austere"), MessageLevel::Warning);
        assert!(austere_warning.trim().is_empty());
    }
}

#[test]
fn intro_payload_override_style_is_not_reinterpreted_by_verbosity_unit() {
    let mut state = make_intro_state(
        &[("ui.presentation", "expressive")],
        RenderMode::Rich,
        ColorMode::Always,
        UnicodeMode::Always,
    );
    state.runtime.ui.verbosity = MessageLevel::Trace;

    let payload = build_repl_intro_payload(
        repl_view(&state),
        &intro_surface_with_overview(&["help", "config"]),
        Some(ReplIntroStyle::Minimal),
    );

    assert!(
        matches!(payload.blocks.as_slice(), [DocBlock::Paragraph { text }] if text.contains("Welcome"))
    );
}

#[test]
fn repl_intro_round_trip_preserves_template_section_order_unit() {
    let state = make_intro_state(
        &[
            ("ui.presentation", "expressive"),
            ("user.name", "oistes"),
            ("theme.name", "rose-pine-moon"),
        ],
        RenderMode::Auto,
        ColorMode::Always,
        UnicodeMode::Always,
    );

    let payload = build_repl_intro_payload(
        repl_view(&state),
        &intro_surface_with_overview(&["help", "config", "theme", "plugins"]),
        None,
    );
    let value = serde_json::to_value(&payload).unwrap();
    let rebuilt: Document = serde_json::from_value(value).unwrap();
    assert_eq!(
        serde_json::to_value(&rebuilt).unwrap(),
        serde_json::to_value(&payload).unwrap()
    );
    assert_eq!(
        rebuilt
            .blocks
            .iter()
            .filter_map(|block| match block {
                DocBlock::Section { title, .. } => Some(title.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec!["OSP", "Keybindings", "Pipes", "Usage", "Commands"]
    );
}

#[test]
fn full_intro_template_uses_semantic_osp_blocks_for_section_data_unit() {
    let state = make_intro_state(
        &[
            ("repl.intro", "full"),
            ("repl.intro_template.full", FULL_INTRO_TEMPLATE_FIXTURE),
        ],
        RenderMode::Rich,
        ColorMode::Always,
        UnicodeMode::Always,
    );

    let payload = build_repl_intro_payload(
        repl_view(&state),
        &intro_surface_with_overview(&["help", "config", "theme", "plugins"]),
        None,
    );

    let value = serde_json::to_value(&payload).unwrap();
    let blocks = value["blocks"].as_array().unwrap();
    let osp = blocks
        .iter()
        .find(|block| block["section"] == "OSP")
        .unwrap();
    assert!(
        osp["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block.get("value").is_some())
    );
    assert!(osp["blocks"].as_array().unwrap().iter().any(|block| {
        block["text"]
            .as_str()
            .is_some_and(|text| text.contains("Welcome"))
    }));
    let keys = blocks
        .iter()
        .find(|block| block["section"] == "Keybindings")
        .unwrap();
    assert_eq!(keys["blocks"][0]["value"][0]["name"], "Ctrl-D");
    assert_eq!(keys["blocks"][0]["value"][0]["short_help"], "exit");
    let pipes = blocks
        .iter()
        .find(|block| block["section"] == "Pipes")
        .unwrap();
    assert!(
        pipes["blocks"][0]["value"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "`| H <verb>` verb help, e.g. `| H F`")
    );
}

#[test]
fn repl_overview_lists_invocation_options_for_expressive_surface() {
    let state = make_state(&[("ui.presentation", "expressive")]);
    let surface = ReplSurface {
        root_words: vec![
            CMD_HELP.to_string(),
            CMD_CONFIG.to_string(),
            CMD_THEME.to_string(),
            CMD_PLUGINS.to_string(),
        ],
        intro_commands: intro_commands(&["help", "config", "theme", "plugins"]),
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: vec![
            ReplOverviewEntry {
                name: "options".to_string(),
                summary: "per invocation: --format json".to_string(),
            },
            ReplOverviewEntry {
                name: "config".to_string(),
                summary: "show config".to_string(),
            },
        ],
    };

    let rendered = render_repl_command_overview(repl_view(&state), &surface);
    assert!(rendered.contains("[INVOCATION_OPTIONS] COMMAND [ARGS]"));
    assert!(rendered.contains("options"));
    assert!(rendered.contains("--format json"));
    assert!(rendered.contains("config"));
}

#[test]
fn repl_appearance_variants_respect_theme_defaults() {
    let plain_state = make_state(&[]);
    let plain = build_repl_appearance(repl_view(&plain_state));
    assert_eq!(plain.completion_text_style, None);
    assert_eq!(plain.completion_background_style, None);
    assert_eq!(plain.completion_highlight_style, None);
    assert_eq!(plain.command_highlight_style, None);
    assert_eq!(plain.history_menu_rows, 5);

    let mut rich_state = make_state(&[("repl.history.menu_rows", "7")]);
    rich_state.runtime.ui.render_settings.mode = crate::core::output::RenderMode::Rich;
    rich_state.runtime.ui.render_settings.color = crate::core::output::ColorMode::Always;
    rich_state.runtime.ui.render_settings.unicode = crate::core::output::UnicodeMode::Always;
    rich_state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    rich_state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);

    let appearance = build_repl_appearance(repl_view(&rich_state));
    assert!(appearance.completion_text_style.is_some());
    assert!(appearance.completion_background_style.is_some());
    assert!(appearance.completion_highlight_style.is_some());
    assert!(appearance.command_highlight_style.is_some());
    assert_eq!(appearance.history_menu_rows, 7);

    let mut state = make_state(&[("theme.name", "dracula")]);
    state.runtime.ui.render_settings.mode = crate::core::output::RenderMode::Rich;
    state.runtime.ui.render_settings.color = crate::core::output::ColorMode::Always;
    state.runtime.ui.render_settings.unicode = crate::core::output::UnicodeMode::Always;
    state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);
    state.runtime.ui.render_settings.theme_name = "dracula".to_string();
    state.runtime.ui.render_settings.theme = Some(crate::ui::theme::resolve_theme("dracula"));

    let appearance = build_repl_appearance(repl_view(&state));
    assert_eq!(appearance.completion_text_style.as_deref(), Some("#000000"));
    assert_eq!(
        appearance.completion_background_style.as_deref(),
        Some("#bd93f9")
    );
    assert_eq!(
        appearance.completion_highlight_style.as_deref(),
        Some("#ff79c6")
    );
}

#[test]
fn repl_prompt_variants_render_scope_indicator_and_prompt_right_unit() {
    let blank_indicator = make_state(&[
        ("ui.presentation", "compact"),
        ("repl.shell_indicator", "   "),
    ]);
    assert_eq!(
        build_repl_prompt(repl_view(&blank_indicator)).left,
        "default> "
    );

    let mut literal_indicator = make_state(&[("repl.shell_indicator", "scoped")]);
    literal_indicator.session.scope.enter("ldap");
    let literal_prompt = build_repl_prompt(repl_view(&literal_indicator)).left;
    assert!(literal_prompt.contains("scoped"));
    assert!(!literal_prompt.contains("ldap /"));

    let mut live_scope = make_state(&[("ui.presentation", "compact")]);
    live_scope.session.scope.enter("ldap");
    assert_eq!(
        build_repl_prompt(repl_view(&live_scope)).left,
        "default [ldap]> "
    );

    let resolved = rich_prompt_right_settings();
    let incognito =
        render_repl_prompt_right_for_test(&resolved, false, &DebugTimingState::default());
    assert!(incognito.contains("(⌐■_■)"));

    let timing = DebugTimingState::default();
    timing.set(DebugTimingBadge {
        level: 1,
        summary: TimingSummary {
            total: Duration::from_millis(120),
            parse: None,
            execute: None,
            render: None,
        },
    });

    let rendered = render_repl_prompt_right_for_test(&resolved, false, &timing);
    assert!(rendered.contains("(⌐■_■)"));
    assert!(rendered.contains("120"));
}

#[test]
fn repl_prompt_right_renders_native_session_context_unit() {
    let resolved = rich_prompt_right_settings();
    let native_context = crate::native::NativeSessionContext::default();
    native_context
        .for_service("siteadmin")
        .set_context_value("netgroup", "uio-drift");
    native_context
        .for_service("siteadmin")
        .set_context_value("contact", "drift-team");

    let rendered = render_repl_prompt_right_for_test_with_context(
        &resolved,
        true,
        &DebugTimingState::default(),
        &native_context,
    );
    let plain = output_support::strip_ansi(&rendered);
    assert!(plain.contains("contact:drift-team"));
    assert!(plain.contains("netgroup:uio-drift"));
}

#[test]
fn theme_display_name_and_prompt_template_formatting_unit() {
    for (slug, expected) in [
        ("rose-pine-moon", "Rose Pine Moon"),
        ("dracula", "Dracula"),
        ("---", "---"),
        ("nord_light", "Nord Light"),
    ] {
        assert_eq!(theme_display_name(slug), expected, "slug={slug}");
    }

    for (template, expected) in [
        (
            "╭─{user}@{domain} {indicator}\n╰─{profile}> ",
            "╭─oistes@uio.no [orch]\n╰─uio> ",
        ),
        ("{profile}>", "tsd> [shell]"),
        ("{profile} {unknown}", "prod {unknown} [ldap]"),
        ("{context}:{indicator}", "prod:[ldap]"),
    ] {
        let rendered = match template {
            "╭─{user}@{domain} {indicator}\n╰─{profile}> " => {
                render_prompt_template(template, "oistes", "uio.no", "uio", "[orch]")
            }
            "{profile}>" => render_prompt_template(template, "oistes", "uio.no", "tsd", "[shell]"),
            _ => render_prompt_template(template, "u", "d", "prod", "[ldap]"),
        };
        assert_eq!(rendered, expected, "template={template}");
    }
}

#[test]
fn repl_intro_template_placeholder_rules_unit() {
    let expanded = make_state(&[
        ("ui.presentation", "compact"),
        (
            "repl.intro_template.compact",
            "Hello {{display_name}} {{profile}} {{version}} {{missing}}",
        ),
        ("user.name", "oistes"),
        ("user.display_names.oistes", "Oistes"),
    ]);
    let expanded_rendered = render_repl_intro(repl_view(&expanded), &intro_surface(&["help"]));
    assert!(expanded_rendered.contains("Hello Oistes default"));
    assert!(expanded_rendered.contains(env!("CARGO_PKG_VERSION")));
    assert!(expanded_rendered.contains("{{missing}}"));

    struct SignInFacts(crate::core::command_policy::SessionFacts);
    impl crate::app::AccessPolicy for SignInFacts {
        fn session_facts(&self) -> crate::core::command_policy::SessionFacts {
            self.0.clone()
        }
    }
    for (services, expected) in [
        (vec![], "Before\nAfter"),
        (
            vec![("directory", "Directory", true)],
            "Before\nLogged in as  oistes\nAfter",
        ),
        (
            vec![("directory", "Directory", false)],
            "Before\nLogged in as  Not signed in\nAfter",
        ),
        (
            vec![
                ("directory", "Directory", true),
                ("registry", "Registry", false),
            ],
            "Before\nLogged in as  oistes (Directory; not signed in: Registry)\nAfter",
        ),
    ] {
        let mut state = make_state(&[
            ("ui.presentation", "compact"),
            (
                "repl.intro_template.compact",
                "Before\n{{session.login}}After",
            ),
            ("user.name", "oistes"),
        ]);
        let mut facts = crate::core::command_policy::SessionFacts::default();
        for (name, label, valid) in services {
            facts
                .selected_credentials
                .push(crate::core::command_policy::SelectedCredential {
                    name: name.into(),
                    label: label.into(),
                });
            facts.credentials.insert(
                name.into(),
                crate::core::command_policy::CredentialState {
                    valid,
                    ttl_seconds: None,
                },
            );
        }
        state
            .runtime
            .set_access_policy(Some(std::sync::Arc::new(SignInFacts(facts))));
        assert_eq!(
            output_support::strip_ansi(&render_repl_intro(repl_view(&state), &intro_surface(&[])))
                .trim(),
            expected
        );
    }

    let sensitive = make_state(&[
        ("ui.presentation", "compact"),
        (
            "repl.intro_template.compact",
            "Token {{extensions.demo.token}}",
        ),
        ("extensions.demo.token", "secret"),
    ]);
    let sensitive_rendered = render_repl_intro(repl_view(&sensitive), &intro_surface(&["help"]));
    assert!(sensitive_rendered.contains("{{extensions.demo.token}}"));
    assert!(!sensitive_rendered.contains("Token secret"));
}

#[test]
fn repl_intro_payload_uses_custom_full_section_templates() {
    let mut state = make_state(&[
        ("ui.presentation", "expressive"),
        (
            "repl.intro_template.full",
            "## OSP\nUser {{user.name}}\n## Keybindings\nKeys {{profile.active}}\n## Pipes\nPipe {{theme_display}}",
        ),
    ]);
    let base = crate::ui::theme::find_builtin_theme("dracula").expect("builtin theme should exist");
    let custom = crate::ui::theme::ThemeDefinition::new(
        "demo-slug",
        "Authored Theme Name",
        None,
        base.palette.clone(),
        base.overrides.clone(),
    );
    state.runtime.ui.render_settings.theme_name = "demo-slug".to_string();
    state.runtime.themes.entries.insert(
        "demo-slug".to_string(),
        crate::ui::theme_catalog::ThemeEntry {
            theme: custom,
            source: crate::ui::theme_catalog::ThemeSource::Custom,
            origin: None,
        },
    );

    let payload =
        build_repl_intro_payload(repl_view(&state), &intro_surface(&["help", "config"]), None);

    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["blocks"].as_array().unwrap().len(), 3);
    assert_eq!(value["blocks"][0]["blocks"][0]["text"], "User anonymous");
    assert_eq!(value["blocks"][1]["blocks"][0]["text"], "Keys default");
    assert_eq!(
        value["blocks"][2]["blocks"][0]["text"],
        "Pipe Authored Theme Name"
    );
}

#[test]
fn repl_intro_payload_overview_placeholders_preserve_sections_and_authored_order_unit() {
    let state = make_state(&[
        ("ui.presentation", "expressive"),
        ("repl.intro_template.full", "## Summary\n{{ help }}"),
    ]);
    let surface = overview_surface(vec![
        ReplOverviewEntry {
            name: "config".to_string(),
            summary: "Show and change config".to_string(),
        },
        ReplOverviewEntry {
            name: "theme".to_string(),
            summary: "List and use themes".to_string(),
        },
    ]);

    let payload = build_repl_intro_payload(repl_view(&state), &surface, None);

    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["blocks"][0]["section"], "Usage");
    assert_eq!(
        value["blocks"][0]["blocks"][0]["text"],
        "[INVOCATION_OPTIONS] COMMAND [ARGS]..."
    );
    assert_eq!(value["blocks"][1]["section"], "Commands");
    assert_eq!(
        value["blocks"][1]["blocks"][0]["entries"],
        json!([
            {"name": "config", "description": "Show and change config"},
            {"name": "theme", "description": "List and use themes"},
        ])
    );

    let authored = make_state(&[
        ("ui.presentation", "expressive"),
        (
            "repl.intro_template.full",
            "Before overview\n## Summary\n{{ overview }}\n## Footer\nAfter overview",
        ),
    ]);
    let authored_payload = build_repl_intro_payload(repl_view(&authored), &surface, None);

    let value = serde_json::to_value(&authored_payload).unwrap();
    assert_eq!(value["blocks"][0]["text"], "Before overview");
    assert_eq!(value["blocks"][1]["section"], "Usage");
    assert_eq!(value["blocks"][2]["section"], "Commands");
    assert_eq!(
        value["blocks"][2]["blocks"][0]["entries"][0]["name"],
        "config"
    );
    assert_eq!(
        value["blocks"][2]["blocks"][0]["entries"][1]["name"],
        "theme"
    );
    assert_eq!(value["blocks"][4]["section"], "Footer");
    assert_eq!(value["blocks"][4]["blocks"][0]["text"], "After overview");
}

#[test]
fn repl_prompt_renders_custom_template_with_prompt_style() {
    let mut state = make_state(&[("repl.prompt", "{user}@{domain} {indicator} {profile}> ")]);
    state.session.scope.enter("ldap");
    state.runtime.ui.render_settings.mode = crate::core::output::RenderMode::Rich;
    state.runtime.ui.render_settings.color = crate::core::output::ColorMode::Always;
    state.runtime.ui.render_settings.unicode = crate::core::output::UnicodeMode::Always;
    state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert!(prompt.contains("anonymous"));
    assert!(prompt.contains("local"));
    assert!(prompt.contains("[ldap]"));
    assert!(prompt.contains("default"));
    assert!(prompt.contains(">"));
    assert!(prompt.contains("\x1b["));
}

#[test]
fn repl_prompt_templates_decode_escaped_newlines_unit() {
    let mut state = make_state(&[("repl.prompt", "{user}\\n{profile}> ")]);
    state.runtime.ui.render_settings.unicode = crate::core::output::UnicodeMode::Always;

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert!(prompt.contains("anonymous\ndefault> "));
}

#[test]
fn session_history_opt_out_enables_incognito_prompt_indicator_unit() {
    let mut state = make_state(&[]);
    state.runtime.ui.render_settings.unicode = crate::core::output::UnicodeMode::Always;
    state.session.history_enabled = false;

    let render =
        build_repl_prompt_right_renderer(repl_view(&state), DebugTimingState::default(), None);
    let prompt_right = render();

    let stripped = output_support::strip_ansi(&prompt_right);
    assert!(!stripped.is_empty());
    assert!(stripped.contains("(⌐■_■)") || stripped.contains("incognito"));
}

#[test]
fn intro_template_parser_expands_all_help_sections_and_merges_repeated_data_blocks() {
    let help = Document {
        blocks: vec![
            DocBlock::section(
                "Usage",
                vec![DocBlock::Paragraph {
                    text: "osp [OPTIONS] COMMAND".into(),
                }],
            ),
            DocBlock::section(
                "Commands",
                vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "help".into(),
                        description: "Show help".into(),
                    }],
                }],
            ),
            DocBlock::section(
                "Arguments",
                vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "<TARGET>".into(),
                        description: "Target name".into(),
                    }],
                }],
            ),
            DocBlock::section(
                "Options",
                vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "--json".into(),
                        description: "Render json".into(),
                    }],
                }],
            ),
            DocBlock::section(
                "Common Invocation Options",
                vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "--profile".into(),
                        description: "Select profile".into(),
                    }],
                }],
            ),
            DocBlock::section(
                "Notes",
                vec![DocBlock::Paragraph {
                    text: "Be careful.".into(),
                }],
            ),
            DocBlock::section(
                "Custom",
                vec![DocBlock::Paragraph {
                    text: "Extra section".into(),
                }],
            ),
            DocBlock::Paragraph {
                text: "tail".into(),
            },
        ],
    };

    let payload = super::parse_intro_template_payload(
        r#"
Before help
## Status
ready
```osp
{"phase":"boot"}
```
```osp
{"phase":"steady"}
```
{{ help }}
"#,
        &help,
    );

    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["blocks"][0]["text"], "Before help");
    assert_eq!(value["blocks"][1]["section"], "Status");
    assert_eq!(
        value["blocks"][1]["blocks"][1]["value"],
        json!({"phase": "boot"})
    );
    assert_eq!(
        value["blocks"][1]["blocks"][2]["value"],
        json!({"phase": "steady"})
    );
    assert_eq!(&payload.blocks[2..], help.blocks.as_slice());
}

#[test]
fn intro_template_parser_flattens_repeated_array_data_blocks_unit() {
    let payload = super::parse_intro_template_payload(
        r#"
## Status
```osp
["boot","steady"]
```
```osp
["ready"]
```
"#,
        &Document::default(),
    );

    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(
        value["blocks"][0]["blocks"][0]["value"],
        json!(["boot", "steady"])
    );
    assert_eq!(value["blocks"][0]["blocks"][1]["value"], json!(["ready"]));
}

#[test]
fn intro_template_expansion_handles_scalars_sensitive_keys_and_malformed_placeholders() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set(
        "repl.history.path",
        "/tmp/osp-repl-presentation-history.jsonl",
    );
    defaults.set("theme.path", Vec::<String>::new());
    let mut session = ConfigLayer::default();
    session.set("extensions.demo.enabled", true);
    session.set("extensions.demo.count", 42_i64);
    session.set("extensions.demo.token", "secret");
    session.set("user.name", "codex");
    session.set("user.display_names.codex", "Codex");
    session.set(
        "extensions.demo.items",
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
    );
    let mut layers = crate::config::LoadedLayers {
        defaults,
        session,
        ..crate::config::LoadedLayers::default()
    };
    let base = ConfigResolver::from_loaded_layers_with_schema(
        layers.clone(),
        crate::config::ConfigSchema::default(),
    )
    .resolve(ResolveOptions::default().with_terminal("repl"))
    .expect("list test config should resolve");
    layers.presentation = build_presentation_defaults_layer(&base);
    let config = ConfigResolver::from_loaded_layers_with_schema(
        layers.clone(),
        crate::config::ConfigSchema::default(),
    )
    .resolve(ResolveOptions::default().with_terminal("repl"))
    .expect("list-aware config should resolve");
    let state = crate::app::AppStateBuilder {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        launch: LaunchContext::default(),
        native_commands: crate::native::NativeCommandRegistry::default(),
        ui: crate::app::UiState::new(
            RenderSettings::test_plain(OutputFormat::Json),
            MessageLevel::Success,
            0,
        ),
        plugins: Some(
            PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
        ),
        themes: Some(crate::ui::theme_catalog::ThemeCatalog::default()),
        session: None,
        definition: Default::default(),
    }
    .build();
    let expanded = super::expand_intro_template(
        repl_view(&state),
        &intro_commands(&["help", "config"]),
        "Hello {{display_name}} {{extensions.demo.enabled}} {{extensions.demo.count}} {{extensions.demo.items}} {{help_hint}} {{help}} {{extensions.demo.token}} {{}} {{broken",
    );

    assert_eq!(
        expanded,
        "Hello Codex true 42 1, 2, 3 See help for more. {{ help }} {{extensions.demo.token}} {{}} {{broken"
    );
    assert_eq!(
        super::parse_intro_template_payload("", &Document::default()),
        Document::default()
    );
}
