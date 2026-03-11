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
use insta::assert_snapshot;
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

mod color_contracts {
    use super::{
        ColorMode, FULL_INTRO_TEMPLATE_FIXTURE, RenderMode, UnicodeMode,
        intro_surface_with_overview, make_intro_state, render_repl_intro, repl_view,
    };
    use crate::ui::chrome::{
        SectionRenderContext, SectionStyleTokens, render_section_divider_with_overrides,
    };
    use crate::ui::style::{StyleToken, apply_style_spec};

    fn intro_color_entries(intro_style: &str) -> Vec<(&str, &str)> {
        let template_key = match intro_style {
            "minimal" => "repl.intro_template.minimal",
            "compact" => "repl.intro_template.compact",
            "full" => "repl.intro_template.full",
            other => panic!("unsupported intro style for color test: {other}"),
        };

        vec![
            ("repl.intro", intro_style),
            (template_key, FULL_INTRO_TEMPLATE_FIXTURE),
            ("user.display_name", "Demo"),
            ("theme.name", "plain"),
            ("color.panel.border", "red"),
            ("color.panel.title", "blue"),
            ("color.key", "yellow"),
            ("color.value", "green"),
        ]
    }

    #[test]
    fn intro_chrome_and_help_tokens_follow_forced_palette_across_modes() {
        for intro_style in ["minimal", "compact", "full"] {
            let state = make_intro_state(
                &intro_color_entries(intro_style),
                RenderMode::Rich,
                ColorMode::Always,
                UnicodeMode::Always,
            );
            let rendered = render_repl_intro(
                repl_view(&state),
                &intro_surface_with_overview(&["help", "config", "theme", "plugins"]),
            );
            let resolved = state.runtime.ui.render_settings.resolve_render_settings();
            let section_divider = render_section_divider_with_overrides(
                "OSP",
                resolved.unicode,
                resolved.width,
                SectionRenderContext {
                    color: resolved.color,
                    theme: &resolved.theme,
                    style_overrides: &resolved.style_overrides,
                },
                SectionStyleTokens {
                    border: StyleToken::PanelBorder,
                    title: StyleToken::PanelTitle,
                },
            );

            assert!(
                rendered.contains(&section_divider),
                "expected styled OSP divider for intro style {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                rendered.contains(&apply_style_spec("  Welcome ", "green", true)),
                "expected intro label text to use the configured value color for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                rendered.contains(&apply_style_spec("Demo", "yellow", true)),
                "expected highlighted intro value to use the configured key color for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                rendered.contains(&apply_style_spec("Ctrl-D", "yellow", true)),
                "expected keybinding hints to use the configured key color for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                rendered.contains("reverse search history"),
                "expected full intro fixture keybinding copy for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                rendered.contains(&apply_style_spec("| H <verb>", "yellow", true)),
                "expected highlighted help shortcut hint for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                !rendered.contains(&apply_style_spec("Demo", "green", true)),
                "highlighted intro values should not fall back to the plain paragraph color for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                !rendered.contains(&apply_style_spec("Ctrl-D", "green", true)),
                "keybinding hints should not fall back to the paragraph color for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                rendered.contains("Show this command overview."),
                "expected help overview to render inside the intro for {intro_style}; rendered:\n{rendered}",
            );
            assert!(
                !rendered.contains(&apply_style_spec("help", "red", true)),
                "help command key should not reuse the border color for {intro_style}; rendered:\n{rendered}",
            );
        }
    }
}

mod unicode_contracts {
    use super::{
        ColorMode, FULL_INTRO_TEMPLATE_FIXTURE, RenderMode, UnicodeMode,
        intro_surface_with_overview, make_intro_state, render_repl_intro, repl_view, strip_ansi,
    };

    #[test]
    fn intro_chrome_switches_between_unicode_and_ascii_without_losing_text_shape() {
        let entries = [
            ("repl.intro", "full"),
            ("repl.intro_template.full", FULL_INTRO_TEMPLATE_FIXTURE),
        ];
        let unicode = make_intro_state(
            &entries,
            RenderMode::Rich,
            ColorMode::Always,
            UnicodeMode::Always,
        );
        let ascii = make_intro_state(
            &entries,
            RenderMode::Rich,
            ColorMode::Always,
            UnicodeMode::Never,
        );

        let unicode_rendered = render_repl_intro(
            repl_view(&unicode),
            &intro_surface_with_overview(&["help", "config", "theme", "plugins"]),
        );
        let ascii_rendered = render_repl_intro(
            repl_view(&ascii),
            &intro_surface_with_overview(&["help", "config", "theme", "plugins"]),
        );
        let unicode_plain = strip_ansi(&unicode_rendered);
        let ascii_plain = strip_ansi(&ascii_rendered);

        assert!(unicode_plain.contains('─'));
        assert!(unicode_plain.contains("OSP"));
        assert!(unicode_plain.contains("Commands"));
        assert!(unicode_plain.contains("Show this command overview."));
        assert!(!unicode_plain.contains("- OSP "));
        assert!(ascii_plain.contains("- OSP "));
        assert!(ascii_plain.contains("- Commands "));
        assert!(!ascii_plain.contains('─'));
        assert!(ascii_plain.contains("Show this command overview."));
    }
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
    fn intro_mode_matrix_keeps_help_and_structure_boundaries() {
        let none = render_style(Some("none"), None, MessageLevel::Success);
        assert!(none.trim().is_empty());

        let minimal = render_style(Some("minimal"), None, MessageLevel::Success);
        assert!(minimal.contains("Welcome anonymous."));
        assert!(minimal.contains("Commands: help, config, theme, plugins."));
        assert!(!minimal.contains("Keybindings"));
        assert!(!minimal.contains("Usage"));

        let compact = render_style(Some("compact"), None, MessageLevel::Success);
        assert!(compact.contains("Welcome anonymous."));
        assert!(compact.contains("Commands: help, config, theme, plugins."));
        assert!(!compact.contains("Keybindings"));
        assert!(!compact.contains("Usage"));

        let full = render_style(Some("full"), None, MessageLevel::Success);
        assert!(full.contains("Keybindings"));
        assert!(full.contains("Pipes"));
        assert!(full.contains("Usage"));
        assert!(full.contains("Commands"));
        assert!(full.contains("  [INVOCATION_OPTIONS] COMMAND [ARGS]..."));
        assert!(full.contains("  help"));
        assert!(full.contains("  config"));
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
fn repl_intro_expressive_includes_sections_and_user_context() {
    let state = make_state(&[
        ("ui.presentation", "expressive"),
        ("user.name", "oistes"),
        ("user.display_name", "Oistes"),
        ("theme.name", "rose-pine-moon"),
    ]);

    let rendered = render_repl_intro(
        repl_view(&state),
        &intro_surface(&["help", "config", "theme", "plugins"]),
    );
    assert!(rendered.contains("OSP"));
    assert!(rendered.contains("Keybindings"));
    assert!(rendered.contains("Pipes"));
    assert!(rendered.contains("Oistes"));
    assert!(rendered.contains("oistes"));
    assert!(rendered.contains("Rose Pine Moon"));
    assert_snapshot!("repl_intro_expressive", rendered);
}

#[test]
fn repl_intro_shared_ruled_sections_preserve_template_order_unit() {
    let state = make_intro_state(
        &[
            ("ui.presentation", "expressive"),
            ("ui.chrome.frame", "top-bottom"),
            ("ui.chrome.rule_policy", "shared"),
            ("user.name", "oistes"),
            ("theme.name", "rose-pine-moon"),
        ],
        RenderMode::Auto,
        ColorMode::Always,
        UnicodeMode::Always,
    );

    let rendered = strip_ansi(&render_repl_intro(
        repl_view(&state),
        &intro_surface_with_overview(&["help", "config", "theme", "plugins"]),
    ));

    let osp = rendered.find("─ OSP ").expect("OSP section should render");
    let keybindings = rendered
        .find("─ Keybindings ")
        .expect("Keybindings section should render");
    let pipes = rendered
        .find("─ Pipes ")
        .expect("Pipes section should render");
    let usage = rendered
        .find("─ Usage ")
        .expect("Usage section should render");
    let commands = rendered
        .find("─ Commands ")
        .expect("Commands section should render");

    assert!(osp < keybindings);
    assert!(keybindings < pipes);
    assert!(pipes < usage);
    assert!(usage < commands);
    assert!(
        !rendered
            .lines()
            .any(|line| line.trim() == "─" || line.trim() == "──")
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
    assert_snapshot!("repl_overview_expressive", rendered);
}

#[test]
fn repl_appearance_respects_color_toggle_and_overrides() {
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
}

#[test]
fn repl_appearance_uses_black_popup_text_for_dracula_theme() {
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
fn repl_prompt_simple_mode_omits_blank_indicator_separator() {
    let state = make_state(&[
        ("ui.presentation", "compact"),
        ("repl.shell_indicator", "   "),
    ]);

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert_eq!(prompt, "default> ");
}

#[test]
fn repl_prompt_custom_indicator_template_can_be_literal() {
    let mut state = make_state(&[("repl.shell_indicator", "scoped")]);
    state.session.scope.enter("ldap");

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert!(prompt.contains("scoped"));
    assert!(!prompt.contains("ldap /"));
}

#[test]
fn repl_prompt_right_shows_unicode_incognito_when_history_is_disabled() {
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.mode = crate::core::output::RenderMode::Rich;
    settings.color = crate::core::output::ColorMode::Always;
    settings.unicode = crate::core::output::UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.runtime.locale_utf8 = Some(true);
    let resolved = settings.resolve_render_settings();

    let rendered =
        render_repl_prompt_right_for_test(&resolved, false, &DebugTimingState::default());
    assert!(rendered.contains("(⌐■_■)"));
}

#[test]
fn repl_prompt_right_combines_incognito_and_timing_badge() {
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.mode = crate::core::output::RenderMode::Rich;
    settings.color = crate::core::output::ColorMode::Always;
    settings.unicode = crate::core::output::UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.runtime.locale_utf8 = Some(true);
    let resolved = settings.resolve_render_settings();
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
fn theme_display_name_falls_back_when_slug_has_no_words() {
    assert_eq!(theme_display_name("---"), "---");
    assert_eq!(theme_display_name("nord_light"), "Nord Light");
}

#[test]
fn prompt_template_preserves_unknown_placeholders_and_appends_indicator() {
    let rendered = render_prompt_template("{profile} {unknown}", "u", "d", "prod", "[ldap]");
    assert_eq!(rendered, "prod {unknown} [ldap]");
}

#[test]
fn prompt_template_replaces_context_alias_and_indicator_placeholder() {
    let rendered = render_prompt_template("{context}:{indicator}", "u", "d", "prod", "[ldap]");
    assert_eq!(rendered, "prod:[ldap]");
}

#[test]
fn repl_intro_minimal_without_help_visibility_uses_completion_hint() {
    let state = make_state(&[
        ("ui.presentation", "compact"),
        ("auth.visible.builtins", "config,theme,plugins"),
    ]);

    let rendered = render_repl_intro(
        repl_view(&state),
        &intro_surface(&["config", "theme", "plugins"]),
    );
    assert!(rendered.contains("Use completion to explore commands."));
    assert!(!rendered.contains("See `help`"));
}

#[test]
fn repl_intro_template_expands_placeholders_and_preserves_unknowns() {
    let state = make_state(&[
        ("ui.presentation", "compact"),
        (
            "repl.intro_template.compact",
            "Hello {{display_name}} {{profile}} {{version}} {{missing}}",
        ),
        ("user.display_name", "Oistes"),
    ]);

    let rendered = render_repl_intro(repl_view(&state), &intro_surface(&["help"]));
    assert!(rendered.contains("Hello Oistes default"));
    assert!(rendered.contains(env!("CARGO_PKG_VERSION")));
    assert!(rendered.contains("{{missing}}"));
}

#[test]
fn repl_intro_template_does_not_expand_sensitive_placeholders() {
    let state = make_state(&[
        ("ui.presentation", "compact"),
        (
            "repl.intro_template.compact",
            "Token {{extensions.demo.token}}",
        ),
        ("extensions.demo.token", "secret"),
    ]);

    let rendered = render_repl_intro(repl_view(&state), &intro_surface(&["help"]));
    assert!(rendered.contains("{{extensions.demo.token}}"));
    assert!(!rendered.contains("Token secret"));
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
fn repl_intro_payload_help_placeholder_merges_overview_entries_unit() {
    let state = make_state(&[
        ("ui.presentation", "expressive"),
        ("repl.intro_template.full", "## Summary\n{{ help }}"),
    ]);
    let surface = ReplSurface {
        root_words: intro_commands(&["help", "config", "theme"]),
        intro_commands: intro_commands(&["help", "config", "theme"]),
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: vec![
            ReplOverviewEntry {
                name: "config".to_string(),
                summary: "Show and change config".to_string(),
            },
            ReplOverviewEntry {
                name: "theme".to_string(),
                summary: "List and use themes".to_string(),
            },
        ],
    };

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
}

#[test]
fn repl_intro_payload_overview_placeholder_preserves_authored_order_and_surrounding_copy_unit() {
    let state = make_state(&[
        ("ui.presentation", "expressive"),
        (
            "repl.intro_template.full",
            "Before overview\n## Summary\n{{ overview }}\n## Footer\nAfter overview",
        ),
    ]);
    let surface = ReplSurface {
        root_words: intro_commands(&["help", "config", "theme"]),
        intro_commands: intro_commands(&["help", "config", "theme"]),
        specs: Vec::new(),
        aliases: Vec::new(),
        overview_entries: vec![
            ReplOverviewEntry {
                name: "config".to_string(),
                summary: "Show and change config".to_string(),
            },
            ReplOverviewEntry {
                name: "theme".to_string(),
                summary: "List and use themes".to_string(),
            },
        ],
    };

    let payload = build_repl_intro_payload(repl_view(&state), &surface, None);

    assert_eq!(payload.preamble, vec!["Before overview"]);
    assert_eq!(payload.sections.len(), 3);
    assert_eq!(payload.sections[0].title, "Usage");
    assert_eq!(payload.sections[1].title, "Commands");
    assert_eq!(
        payload.sections[1]
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["config", "theme"]
    );
    assert_eq!(payload.sections[2].title, "Footer");
    assert_eq!(payload.sections[2].paragraphs, vec!["  After overview"]);
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

#[test]
fn repl_simple_prompt_includes_live_shell_indicator() {
    let mut state = make_state(&[("ui.presentation", "compact")]);
    state.session.scope.enter("ldap");

    let prompt = build_repl_prompt(repl_view(&state)).left;
    assert_eq!(prompt, "default [ldap]> ");
}
