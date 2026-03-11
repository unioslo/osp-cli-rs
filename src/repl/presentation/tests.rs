use super::{
    build_repl_appearance, build_repl_intro_payload, build_repl_prompt, render_prompt_template,
    render_repl_command_overview, render_repl_intro, render_repl_prompt_right_for_test,
    theme_display_name,
};
use crate::app::TimingSummary;
use crate::app::{
    AppState, AppStateInit, DebugTimingBadge, DebugTimingState, LaunchContext, RuntimeContext,
    TerminalKind,
};
use crate::app::{CMD_CONFIG, CMD_HELP, CMD_PLUGINS, CMD_THEME};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::plugin::PluginManager;
use crate::repl::ReplViewContext;
use crate::repl::surface::{ReplOverviewEntry, ReplSurface};
use crate::ui::RenderSettings;
use crate::ui::messages::MessageLevel;
use crate::ui::presentation::build_presentation_defaults_layer;
use std::time::Duration;

const FULL_INTRO_TEMPLATE_FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/repl_intro/full_intro_template.md"
));

fn make_state(entries: &[(&str, &str)]) -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    let mut session = ConfigLayer::default();
    for (key, value) in entries {
        session.set(*key, *value);
    }
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_session(session);
    let options = ResolveOptions::default().with_terminal("repl");
    let base = resolver
        .resolve(options.clone())
        .expect("base test config should resolve");
    resolver.set_presentation(build_presentation_defaults_layer(&base));
    let config = resolver
        .resolve(options)
        .expect("test config should resolve");

    AppState::new(AppStateInit {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        render_settings: RenderSettings::test_plain(OutputFormat::Json),
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
        plugins: PluginManager::new(Vec::new()),
        native_commands: crate::native::NativeCommandRegistry::default(),
        themes: crate::ui::theme_loader::ThemeCatalog::default(),
        launch: LaunchContext::default(),
    })
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
    state.runtime.ui.render_settings.format = OutputFormat::Guide;
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
    crate::cli::apply_render_settings_from_config(
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

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }

    out
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
        make_intro_state, render_repl_intro, repl_view, strip_ansi,
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
        state.runtime.ui.message_verbosity = verbosity;
        strip_ansi(&render_repl_intro(
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
        assert!(compact.contains("Welcome anonymous."));
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
    let rebuilt = crate::guide::GuideView::try_from_output_result(&payload.to_output_result())
        .expect("guide");

    assert_eq!(
        rebuilt
            .sections
            .iter()
            .map(|section| section.title.as_str())
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

    let osp = payload
        .sections
        .iter()
        .find(|section| section.title == "OSP")
        .expect("expected OSP section");
    assert!(osp.data.is_some(), "expected semantic data for OSP section");
    assert!(osp.paragraphs.iter().any(|line| line.contains("Welcome")));

    let keybindings = payload
        .sections
        .iter()
        .find(|section| section.title == "Keybindings")
        .expect("expected keybindings section");
    let Some(serde_json::Value::Array(items)) = keybindings.data.as_ref() else {
        panic!("expected keybindings semantic array data");
    };
    assert_eq!(items[0]["name"], "Ctrl-D");
    assert_eq!(items[0]["short_help"], "exit");

    let pipes = payload
        .sections
        .iter()
        .find(|section| section.title == "Pipes")
        .expect("expected pipes section");
    let Some(serde_json::Value::Array(items)) = pipes.data.as_ref() else {
        panic!("expected pipes semantic array data");
    };
    assert_eq!(items[0], "`F` key>3");
    assert_eq!(items[15], "`| H <verb>` verb help, e.g. `| H F`");
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
fn repl_appearance_variants_respect_color_overrides_and_theme_defaults() {
    let plain_state = make_state(&[]);
    let plain = build_repl_appearance(repl_view(&plain_state));
    assert_eq!(plain.completion_text_style, None);
    assert_eq!(plain.completion_background_style, None);
    assert_eq!(plain.completion_highlight_style, None);
    assert_eq!(plain.command_highlight_style, None);
    assert_eq!(plain.history_menu_rows, 5);

    let mut rich_state = make_state(&[
        ("color.prompt.completion.text", "red"),
        ("color.prompt.completion.background", "blue"),
        ("color.prompt.completion.highlight", "bold green"),
        ("color.prompt.command", "yellow"),
        ("repl.history.menu_rows", "7"),
    ]);
    rich_state.runtime.ui.render_settings.mode = crate::core::output::RenderMode::Rich;
    rich_state.runtime.ui.render_settings.color = crate::core::output::ColorMode::Always;
    rich_state.runtime.ui.render_settings.unicode = crate::core::output::UnicodeMode::Always;
    rich_state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    rich_state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);

    let appearance = build_repl_appearance(repl_view(&rich_state));
    assert_eq!(appearance.completion_text_style.as_deref(), Some("red"));
    assert_eq!(
        appearance.completion_background_style.as_deref(),
        Some("blue")
    );
    assert_eq!(
        appearance.completion_highlight_style.as_deref(),
        Some("bold green")
    );
    assert_eq!(
        appearance.command_highlight_style.as_deref(),
        Some("yellow")
    );
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
        ("user.display_name", "Oistes"),
    ]);
    let expanded_rendered = render_repl_intro(repl_view(&expanded), &intro_surface(&["help"]));
    assert!(expanded_rendered.contains("Hello Oistes default"));
    assert!(expanded_rendered.contains(env!("CARGO_PKG_VERSION")));
    assert!(expanded_rendered.contains("{{missing}}"));

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
    let state = make_state(&[
        ("ui.presentation", "expressive"),
        (
            "repl.intro_template.full",
            "## OSP\nUser {{user.name}}\n## Keybindings\nKeys {{profile.active}}\n## Pipes\nPipe {{theme_display}}",
        ),
    ]);

    let payload =
        build_repl_intro_payload(repl_view(&state), &intro_surface(&["help", "config"]), None);
    let expected_theme_display =
        theme_display_name(&repl_view(&state).ui.render_settings.theme_name);

    assert_eq!(payload.sections.len(), 3);
    assert_eq!(payload.sections[0].paragraphs, vec!["  User anonymous"]);
    assert_eq!(payload.sections[1].paragraphs, vec!["  Keys default"]);
    assert_eq!(
        payload.sections[2].paragraphs,
        vec![format!("  Pipe {expected_theme_display}")]
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

    assert_eq!(payload.sections.len(), 2);
    assert_eq!(payload.sections[0].title, "Usage");
    assert_eq!(
        payload.sections[0].paragraphs,
        vec!["  [INVOCATION_OPTIONS] COMMAND [ARGS]..."]
    );
    assert_eq!(payload.sections[1].title, "Commands");
    assert_eq!(payload.sections[1].entries.len(), 2);
    assert_eq!(payload.sections[1].entries[0].name, "config");
    assert_eq!(
        payload.sections[1].entries[0].short_help,
        "Show and change config"
    );
    assert_eq!(payload.sections[1].entries[1].name, "theme");
    assert_eq!(
        payload.sections[1].entries[1].short_help,
        "List and use themes"
    );

    let authored = make_state(&[
        ("ui.presentation", "expressive"),
        (
            "repl.intro_template.full",
            "Before overview\n## Summary\n{{ overview }}\n## Footer\nAfter overview",
        ),
    ]);
    let authored_payload = build_repl_intro_payload(repl_view(&authored), &surface, None);

    assert_eq!(authored_payload.preamble, vec!["Before overview"]);
    assert_eq!(authored_payload.sections.len(), 3);
    assert_eq!(authored_payload.sections[0].title, "Usage");
    assert_eq!(authored_payload.sections[1].title, "Commands");
    assert_eq!(
        authored_payload.sections[1]
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["config", "theme"]
    );
    assert_eq!(authored_payload.sections[2].title, "Footer");
    assert_eq!(
        authored_payload.sections[2].paragraphs,
        vec!["  After overview"]
    );
}

#[test]
fn repl_prompt_renders_custom_template_with_prompt_style() {
    let mut state = make_state(&[
        ("repl.prompt", "{user}@{domain} {indicator} {profile}> "),
        ("color.prompt.text", "green"),
    ]);
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
