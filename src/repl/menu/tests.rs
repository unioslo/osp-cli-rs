use super::{OspCompletionMenu, needs_space_prefix};
use nu_ansi_term::{Color, Style};
use reedline::{Completer, Editor, Menu, MenuEvent, Span, Suggestion, UndoBehavior};
use std::path::PathBuf;
use std::sync::Mutex;
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
struct FixedCompleter {
    suggestions: Vec<Suggestion>,
}

impl Completer for FixedCompleter {
    fn complete(&mut self, _line: &str, _pos: usize) -> Vec<Suggestion> {
        self.suggestions.clone()
    }
}

#[derive(Clone)]
struct DynamicSpanCompleter;

impl Completer for DynamicSpanCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let start = line
            .get(..pos)
            .unwrap_or("")
            .rfind(|ch: char| ch.is_whitespace())
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let span = Span { start, end: pos };
        vec![suggestion("config", span), suggestion("doctor", span)]
    }
}

#[derive(Clone)]
struct ScopedConfigCompleter;

impl Completer for ScopedConfigCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let input = line.get(..pos).unwrap_or(line);
        match input {
            "config " | "config show" | "config get" | "config explain" => {
                let span = Span { start: 7, end: pos };
                vec![
                    suggestion("show", span),
                    suggestion("get", span),
                    suggestion("explain", span),
                ]
            }
            "config show " => {
                let span = Span {
                    start: pos,
                    end: pos,
                };
                vec![suggestion("--sources", span), suggestion("--raw", span)]
            }
            _ => Vec::new(),
        }
    }
}

fn set_buffer(editor: &mut Editor, buffer: &str) {
    editor.edit_buffer(
        |buf| buf.set_buffer(buffer.to_string()),
        UndoBehavior::CreateUndoPoint,
    );
}

fn suggestion(value: &str, span: Span) -> Suggestion {
    Suggestion {
        value: value.to_string(),
        span,
        append_whitespace: true,
        ..Suggestion::default()
    }
}

fn split_lines(output: &str) -> Vec<&str> {
    output.split_terminator("\r\n").collect()
}

fn env_lock() -> &'static Mutex<()> {
    crate::tests::env_lock()
}

fn make_temp_dir(prefix: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

fn restore_env(key: &str, value: Option<String>) {
    if let Some(value) = value {
        set_env_var_for_test(key, value);
    } else {
        remove_env_var_for_test(key);
    }
}

fn set_env_var_for_test(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    // Test-only environment mutation is process-global on Rust 2024.
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_env_var_for_test(key: &str) {
    unsafe {
        std::env::remove_var(key);
    }
}

#[test]
fn tab_cycles_selection_replaces_buffer() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "co");

    let mut completer = DynamicSpanCompleter;
    let mut menu = OspCompletionMenu::default();

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    let debug = super::debug_snapshot(&mut menu, &editor, 80, 5, false);
    assert_eq!(debug.selected_index, 0);
    assert_eq!(editor.line_buffer().get_buffer(), "co");

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);

    let debug = super::debug_snapshot(&mut menu, &editor, 80, 5, false);
    assert_eq!(debug.selected_index, 0);
    assert_eq!(editor.line_buffer().get_buffer(), "config");

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);

    let debug = super::debug_snapshot(&mut menu, &editor, 80, 5, false);
    assert_eq!(debug.selected_index, 1);
    assert_eq!(editor.line_buffer().get_buffer(), "doctor");
}

#[test]
fn accept_paths_apply_selected_completion() {
    for use_replace_in_buffer in [false, true] {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "doctor ");
        let insert_at = editor.line_buffer().len();

        let suggestions = vec![
            suggestion(
                "all",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
            suggestion(
                "config",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
            suggestion(
                "plugins",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
        ];
        let mut completer = FixedCompleter { suggestions };
        let mut menu = OspCompletionMenu::default();

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        if use_replace_in_buffer {
            menu.replace_in_buffer(&mut editor);
        } else {
            menu.accept_selection_in_buffer(&mut editor);
        }

        assert_eq!(editor.line_buffer().get_buffer(), "doctor all ");
    }
}

#[test]
fn menu_uses_display_text_when_present() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");
    let mut menu = OspCompletionMenu::default();

    let mut first = suggestion("config", Span { start: 0, end: 0 });
    first.extra = Some(vec!["Configure".to_string()]);
    let second = suggestion("doctor", Span { start: 0, end: 0 });

    let mut completer = FixedCompleter {
        suggestions: vec![first, second],
    };

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    let output = menu.menu_string(10, false);
    assert!(output.contains("Configure"));
    assert!(output.contains("doctor"));
}

#[test]
fn menu_description_rendering_tracks_width_and_available_lines() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");
    let mut menu = OspCompletionMenu::default();

    let mut first = suggestion("config", Span { start: 0, end: 0 });
    first.description = Some("Inspect and edit runtime config".to_string());

    let mut completer = FixedCompleter {
        suggestions: vec![first],
    };

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);

    let wide_output = menu.menu_string(10, false);
    let wide_last = split_lines(&wide_output)
        .last()
        .map(|line| line.trim())
        .unwrap_or_default()
        .to_string();
    assert!(!wide_last.is_empty());
    assert!("Inspect and edit runtime config".starts_with(&wide_last));

    menu.update_for_test(&mut editor, &mut completer, 10);
    let narrow_output = menu.menu_string(10, false);
    assert!(narrow_output.contains("Inspect"));
    assert!(!narrow_output.contains("runtime config"));

    let constrained_output = menu.menu_string(1, false);
    let constrained_lines = split_lines(&constrained_output);
    assert_eq!(constrained_lines.len(), 1);
    assert!(!constrained_output.contains("Inspect and edit runtime config"));
}

#[test]
fn menu_narrows_columns_on_small_width() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");

    let suggestions = vec![
        suggestion("alpha", Span { start: 0, end: 0 }),
        suggestion("bravo", Span { start: 0, end: 0 }),
        suggestion("charlie", Span { start: 0, end: 0 }),
        suggestion("delta", Span { start: 0, end: 0 }),
    ];

    let mut completer = FixedCompleter {
        suggestions: suggestions.clone(),
    };
    let mut menu_small = OspCompletionMenu::default();
    menu_small.menu_event(MenuEvent::Activate(false));
    menu_small.update_for_test(&mut editor, &mut completer, 10);
    assert_eq!(menu_small.columns_for_test(), 1);

    let mut completer = FixedCompleter { suggestions };
    let mut menu_large = OspCompletionMenu::default();
    menu_large.menu_event(MenuEvent::Activate(false));
    menu_large.update_for_test(&mut editor, &mut completer, 80);
    assert!(menu_large.columns_for_test() > 1);
}

#[test]
fn menu_respects_available_lines() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");
    let mut menu = OspCompletionMenu::default();

    let mut suggestions = Vec::new();
    for idx in 0..10 {
        suggestions.push(suggestion(&format!("item{idx}"), Span { start: 0, end: 0 }));
    }
    let mut completer = FixedCompleter { suggestions };

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    let output = menu.menu_string(1, false);
    let lines = split_lines(&output);
    assert!(!lines.is_empty());
    assert!(lines.len() <= 1);
}

#[test]
fn menu_ansi_coloring_preserves_case_and_resets() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");
    let mut menu = OspCompletionMenu::default();

    let suggestions = vec![
        suggestion("config", Span { start: 0, end: 0 }),
        suggestion("doctor", Span { start: 0, end: 0 }),
    ];
    let mut completer = FixedCompleter { suggestions };

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    let output = menu.menu_string(10, true);
    assert!(output.contains("config"));
    assert!(!output.contains("CONFIG"));
    assert!(output.contains("\u{1b}["));
    assert!(output.contains("\u{1b}[0m"));
}

#[test]
fn menu_non_ansi_marks_selected_item() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");
    let mut menu = OspCompletionMenu::default();

    let suggestions = vec![
        suggestion("config", Span { start: 0, end: 0 }),
        suggestion("doctor", Span { start: 0, end: 0 }),
    ];
    let mut completer = FixedCompleter { suggestions };

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);

    let output = menu.menu_string(10, false);
    assert!(output.contains("> config"));
    assert!(output.contains("  doctor"));
}

#[test]
fn initial_activation_renders_without_visible_selection() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");
    let mut menu = OspCompletionMenu::default();

    let suggestions = vec![
        suggestion("catppuccin", Span { start: 0, end: 0 }),
        suggestion("dracula", Span { start: 0, end: 0 }),
    ];
    let mut completer = FixedCompleter { suggestions };

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    let output = menu.menu_string(10, false);
    assert!(output.contains("  catppuccin"));
    assert!(output.contains("  dracula"));
    assert!(!output.contains("> catppuccin"));
}

#[test]
fn cycling_completion_keeps_menu_indent_anchored_to_original_span() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "theme use ");
    let insert_at = editor.line_buffer().len();
    let suggestions = vec![
        suggestion(
            "catppuccin",
            Span {
                start: insert_at,
                end: insert_at,
            },
        ),
        suggestion(
            "dracula",
            Span {
                start: insert_at,
                end: insert_at,
            },
        ),
        suggestion(
            "gruvbox",
            Span {
                start: insert_at,
                end: insert_at,
            },
        ),
    ];
    let mut completer = FixedCompleter { suggestions };
    let mut menu = OspCompletionMenu::default();

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 120);
    let initial = super::debug_snapshot(&mut menu, &editor, 120, 10, false);

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 120);
    let first_cycle = super::debug_snapshot(&mut menu, &editor, 120, 10, false);

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 120);
    let second_cycle = super::debug_snapshot(&mut menu, &editor, 120, 10, false);

    assert_eq!(first_cycle.indent, initial.indent);
    assert_eq!(second_cycle.indent, initial.indent);
    assert_eq!(editor.line_buffer().get_buffer(), "theme use dracula");
}

#[test]
fn menu_width_never_exceeds_screen_width_without_ansi() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");
    let mut menu = OspCompletionMenu::default();

    let suggestions = vec![
        suggestion("alpha", Span { start: 0, end: 0 }),
        suggestion("bravo", Span { start: 0, end: 0 }),
        suggestion("charlie", Span { start: 0, end: 0 }),
    ];
    let mut completer = FixedCompleter { suggestions };

    let screen_width = 20;
    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, screen_width);

    let output = menu.menu_string(10, false);
    for line in split_lines(&output) {
        assert!(line.width() <= screen_width as usize);
    }
}

#[test]
fn menu_debug_reports_styles_and_selection() {
    let span = Span { start: 0, end: 0 };
    let suggestions = vec![suggestion("config", span)];
    let mut menu = OspCompletionMenu::default()
        .with_text_style(Style::new().fg(Color::Red).on(Color::Black))
        .with_selected_text_style(Style::new().fg(Color::Green).on(Color::Blue))
        .with_description_text_style(Style::new().fg(Color::Yellow))
        .with_match_text_style(Style::new().fg(Color::Cyan))
        .with_selected_match_text_style(Style::new().fg(Color::Magenta));

    let mut editor = Editor::default();
    let mut completer = FixedCompleter { suggestions };

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 20);

    let debug = super::debug_snapshot(&mut menu, &editor, 20, 5, false);
    assert_eq!(debug.styles.text.foreground.as_deref(), Some("red"));
    assert_eq!(debug.styles.text.background.as_deref(), Some("black"));
    assert_eq!(
        debug.styles.selected_text.foreground.as_deref(),
        Some("green")
    );
    assert_eq!(
        debug.styles.selected_text.background.as_deref(),
        Some("blue")
    );
    assert_eq!(
        debug.styles.description.foreground.as_deref(),
        Some("yellow")
    );
    assert_eq!(debug.styles.match_text.foreground.as_deref(), Some("cyan"));
    assert_eq!(
        debug.styles.selected_match.foreground.as_deref(),
        Some("magenta")
    );
    assert_eq!(debug.selected_index, 0);
    assert_eq!(debug.selected_row, 0);
    assert_eq!(debug.selected_col, 0);
}

#[test]
fn indicator_is_empty_until_menu_has_values() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "co");
    let mut completer = DynamicSpanCompleter;
    let mut menu = OspCompletionMenu::default().with_marker(">> ");

    assert_eq!(menu.indicator(), "");

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    assert_eq!(menu.indicator(), ">> ");
    assert!(menu.menu_required_lines(80) >= 1);
}

#[test]
fn partial_complete_uses_buffer_prefix_when_requested() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "config sh");
    let cursor = editor.line_buffer().len();
    let mut completer = FixedCompleter {
        suggestions: vec![
            suggestion(
                "show",
                Span {
                    start: cursor - 2,
                    end: cursor,
                },
            ),
            suggestion(
                "shell",
                Span {
                    start: cursor - 2,
                    end: cursor,
                },
            ),
        ],
    };
    let mut menu = OspCompletionMenu::default().with_only_buffer_difference(true);

    assert!(menu.can_partially_complete(false, &mut editor, &mut completer));
}

#[test]
fn partial_complete_returns_false_without_common_extension() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "d");
    let mut completer = FixedCompleter {
        suggestions: vec![
            suggestion("config", Span { start: 0, end: 1 }),
            suggestion("help", Span { start: 0, end: 1 }),
        ],
    };
    let mut menu = OspCompletionMenu::default();

    assert!(!menu.can_partially_complete(false, &mut editor, &mut completer));
    assert_eq!(editor.line_buffer().get_buffer(), "d");
    assert_eq!(menu.get_values().len(), 2);
}

#[test]
fn replace_in_buffer_inserts_missing_space_before_completion() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "doctor");
    let cursor = editor.line_buffer().len();
    let mut completer = FixedCompleter {
        suggestions: vec![suggestion(
            "config",
            Span {
                start: cursor,
                end: cursor,
            },
        )],
    };
    let mut menu = OspCompletionMenu::default();

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);
    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);
    menu.accept_selection_in_buffer(&mut editor);

    assert_eq!(editor.line_buffer().get_buffer(), "doctor config ");
    assert!(needs_space_prefix("doctor", 6, 6));
    assert!(!needs_space_prefix("doctor ", 7, 7));
    assert!(!needs_space_prefix("a=", 2, 2));
}

#[test]
fn menu_accessors_and_cursor_fallback_indent_track_state() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "help");

    let mut menu = OspCompletionMenu::default()
        .with_name("history_menu")
        .with_quick_complete(false);
    menu.set_cursor_pos((12, 0));

    let debug = super::debug_snapshot(&mut menu, &editor, 80, 5, false);
    assert_eq!(menu.name(), "history_menu");
    assert!(!menu.can_quick_complete());
    assert!(!menu.is_active());
    assert!(menu.get_values().is_empty());
    assert_eq!(debug.indent, 12);
}

#[test]
fn reactivation_recomputes_indent_after_deactivate() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "theme use ");

    let insert_at = editor.line_buffer().len();
    let mut completer = FixedCompleter {
        suggestions: vec![
            suggestion(
                "catppuccin",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
            suggestion(
                "dracula",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
        ],
    };
    let mut menu = OspCompletionMenu::default();
    menu.set_cursor_pos((20, 0));

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 120);
    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 120);
    let anchored = super::debug_snapshot(&mut menu, &editor, 120, 10, false).indent;

    menu.menu_event(MenuEvent::Deactivate);
    let mut empty = FixedCompleter {
        suggestions: Vec::new(),
    };
    menu.update_for_test(&mut editor, &mut empty, 120);
    assert_eq!(menu.indicator(), "");
    assert!(!menu.is_active());
    assert!(menu.get_values().is_empty());

    set_buffer(&mut editor, "x ");
    let insert_at = editor.line_buffer().len();
    let mut completer = FixedCompleter {
        suggestions: vec![suggestion(
            "alpha",
            Span {
                start: insert_at,
                end: insert_at,
            },
        )],
    };
    menu.set_cursor_pos((8, 0));
    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    let reactivated = super::debug_snapshot(&mut menu, &editor, 80, 5, false).indent;
    assert_eq!(reactivated, 8);
    assert_ne!(reactivated, anchored);
}

#[test]
fn builders_shape_layout_and_min_rows() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "");

    let mut completer = FixedCompleter {
        suggestions: vec![
            suggestion("alpha", Span { start: 0, end: 0 }),
            suggestion("bravo", Span { start: 0, end: 0 }),
            suggestion("charlie", Span { start: 0, end: 0 }),
            suggestion("delta", Span { start: 0, end: 0 }),
            suggestion("echo", Span { start: 0, end: 0 }),
            suggestion("foxtrot", Span { start: 0, end: 0 }),
        ],
    };
    let mut menu = OspCompletionMenu::default()
        .with_columns(3)
        .with_column_padding(1)
        .with_max_rows(2)
        .with_description_rows(2);

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    assert_eq!(menu.columns_for_test(), 3);
    assert_eq!(menu.min_rows(), 1);
    assert_eq!(split_lines(&menu.menu_string(10, false)).len(), 2);
}

#[test]
fn navigation_events_defer_buffer_and_selection_updates_until_refresh_unit() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "config ");
    let mut completer = ScopedConfigCompleter;
    let mut menu = OspCompletionMenu::default();

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    menu.menu_event(MenuEvent::NextElement);
    assert_eq!(editor.line_buffer().get_buffer(), "config ");
    assert!(matches!(menu.event, Some(MenuEvent::NextElement)));

    menu.update_for_test(&mut editor, &mut completer, 80);
    assert_eq!(editor.line_buffer().get_buffer(), "config show");
    assert_eq!(menu.core.selected_index(), Some(0));

    menu.menu_event(MenuEvent::NextElement);
    assert_eq!(editor.line_buffer().get_buffer(), "config show");
    assert!(matches!(menu.event, Some(MenuEvent::NextElement)));

    menu.update_for_test(&mut editor, &mut completer, 80);
    assert_eq!(editor.line_buffer().get_buffer(), "config get");
    assert_eq!(menu.core.selected_index(), Some(1));
}

#[test]
fn cycling_without_space_stays_on_same_token_sibling_scope_unit() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "config ");
    let mut completer = ScopedConfigCompleter;
    let mut menu = OspCompletionMenu::default();

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);
    assert_eq!(editor.line_buffer().get_buffer(), "config show");

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);
    assert_eq!(editor.line_buffer().get_buffer(), "config get");

    let values = menu
        .get_values()
        .iter()
        .map(|suggestion| suggestion.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(values, vec!["show", "get", "explain"]);
}

#[test]
fn space_after_subcommand_switches_completion_to_child_scope_unit() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "config show ");
    let mut completer = ScopedConfigCompleter;
    let mut menu = OspCompletionMenu::default();

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test(&mut editor, &mut completer, 80);

    let values = menu
        .get_values()
        .iter()
        .map(|suggestion| suggestion.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(values, vec!["--sources", "--raw"]);

    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test(&mut editor, &mut completer, 80);
    assert_eq!(editor.line_buffer().get_buffer(), "config show --sources");
}

#[test]
fn preloaded_partial_completion_and_empty_refresh_cover_skip_paths() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "config sh");
    let cursor = editor.line_buffer().len();
    let mut completer = FixedCompleter {
        suggestions: vec![
            suggestion(
                "show",
                Span {
                    start: cursor - 2,
                    end: cursor,
                },
            ),
            suggestion(
                "shell",
                Span {
                    start: cursor - 2,
                    end: cursor,
                },
            ),
        ],
    };
    let mut menu = OspCompletionMenu::default().with_only_buffer_difference(true);

    menu.update_values(&mut editor, &mut completer);
    assert!(menu.can_partially_complete(true, &mut editor, &mut completer));

    menu.indent_anchor = Some(99);
    let mut empty = FixedCompleter {
        suggestions: Vec::new(),
    };
    menu.update_values(&mut editor, &mut empty);

    assert!(menu.get_values().is_empty());
    assert!(menu.indent_anchor.is_none());
}

#[test]
fn trace_paths_record_complete_cycle_and_accept_events() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let trace_dir = make_temp_dir("osp-menu-trace");
    let trace_path = trace_dir.join("trace.jsonl");
    let previous_enabled = std::env::var("OSP_REPL_TRACE_COMPLETION").ok();
    let previous_path = std::env::var("OSP_REPL_TRACE_PATH").ok();
    set_env_var_for_test("OSP_REPL_TRACE_COMPLETION", "1");
    set_env_var_for_test("OSP_REPL_TRACE_PATH", &trace_path);

    let mut editor = Editor::default();
    set_buffer(&mut editor, "co");
    let mut completer = DynamicSpanCompleter;
    let mut menu = OspCompletionMenu::default();

    menu.menu_event(MenuEvent::Activate(false));
    menu.update_for_test_with_available_lines(&mut editor, &mut completer, 40, 5);
    menu.menu_event(MenuEvent::NextElement);
    menu.update_for_test_with_available_lines(&mut editor, &mut completer, 40, 5);
    menu.accept_selection_in_buffer(&mut editor);

    restore_env("OSP_REPL_TRACE_COMPLETION", previous_enabled);
    restore_env("OSP_REPL_TRACE_PATH", previous_path);

    let contents = std::fs::read_to_string(&trace_path).expect("trace file should exist");
    assert!(contents.contains("\"event\":\"complete\""));
    assert!(contents.contains("\"event\":\"cycle\""));
    assert!(contents.contains("\"event\":\"accept\""));
    assert!(contents.contains("\"visible_rows\":"));
    assert!(contents.contains("\"menu_indent\""));
}

#[test]
fn helper_edges_cover_invalid_spans_and_value_based_indent() {
    let mut editor = Editor::default();
    set_buffer(&mut editor, "go");

    let mut menu = OspCompletionMenu::default();
    menu.core
        .set_values(vec![suggestion("config", Span { start: 2, end: 2 })]);
    menu.set_cursor_pos((6, 0));
    assert_eq!(super::compute_menu_indent(&menu, &editor), 6);

    set_buffer(&mut editor, "help");
    menu.replace_span = Some(Span { start: 10, end: 1 });
    menu.apply_selection_in_buffer(&mut editor, super::ApplyMode::Accept);

    assert_eq!(editor.line_buffer().get_buffer(), "help config ");
    assert!(!needs_space_prefix("hi", 5, 5));
}
