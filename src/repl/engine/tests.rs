use crate::completion::{CompletionNode, CompletionTree, FlagNode};
use nu_ansi_term::Color;
use reedline::{
    Completer, EditCommand, Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus,
};
use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::editor::PaintedLine;
use super::{
    AutoCompleteEmacs, BasicInputReason, CompletionDebugOptions, DebugStep, HistoryConfig,
    HistoryShellContext, OspPrompt, PromptRightRenderer, ReplAppearance, ReplCompleter,
    ReplInputMode, ReplReloadKind, ReplResult, SharedHistory, SubmissionContext,
    basic_input_reason, build_history_picker_options, build_repl_highlighter,
    color_from_style_spec, debug_completion, debug_completion_steps, default_pipe_verbs,
    evaluate_repl_submission, history_picker_items, parse_cursor_position_report, path_suggestions,
    split_path_stub, trace_completion, trace_completion_enabled,
};
use crate::core::shell_words::QuoteStyle;
use crate::repl::LineProjection;
use crate::repl::ReplCompleteOn;
use crate::repl::menu::{OspCompletionMenu, SharedCompletionMenu};
use reedline::Emacs;

fn completion_tree_with_config_show() -> CompletionTree {
    let show = CompletionNode::default()
        .with_flag("--sources", FlagNode::new().flag_only())
        .with_flag("--raw", FlagNode::new().flag_only());

    let config = CompletionNode::default()
        .with_child("show", show)
        .with_child("get", CompletionNode::default())
        .with_child("explain", CompletionNode::default());

    let mut root = CompletionNode::default();
    root.children.insert("config".to_string(), config);
    CompletionTree {
        root,
        ..CompletionTree::default()
    }
}

fn completion_tree_with_root_commands() -> CompletionTree {
    let root = CompletionNode::default()
        .with_child("help", CompletionNode::default())
        .with_child("exit", CompletionNode::default())
        .with_child("quit", CompletionNode::default())
        .with_child("config", CompletionNode::default());

    CompletionTree {
        root,
        ..CompletionTree::default()
    }
}

fn history_config() -> HistoryConfig {
    HistoryConfig {
        enabled: true,
        max_entries: 32,
        dedupe: false,
        ..HistoryConfig::default()
    }
}

fn test_appearance() -> super::ReplAppearance {
    ReplAppearance::builder()
        .with_completion_text_style(Some("white".to_string()))
        .with_completion_background_style(Some("black".to_string()))
        .with_completion_highlight_style(Some("cyan".to_string()))
        .with_command_highlight_style(Some("green".to_string()))
        .with_history_menu_rows(5)
        .build()
}

fn disabled_history() -> SharedHistory {
    SharedHistory::new(HistoryConfig {
        enabled: false,
        max_entries: 0,
        shell_context: HistoryShellContext::default(),
        ..history_config()
    })
}

// History expansion and submission contracts.

#[test]
fn history_expansion_variants_resolve_recent_relative_and_prefix_queries_unit() {
    use crate::repl::history_store::BangCommand;

    let history = test_history(&["ldap user oistes", "ldap netgroup ucore"]);
    assert_eq!(
        history
            .with(|store| store.expand(&BangCommand::Last, None, false))
            .ok()
            .flatten(),
        Some("ldap netgroup ucore".to_string())
    );
    assert_eq!(
        history
            .with(|store| store.expand(&BangCommand::Relative(1), None, false))
            .ok()
            .flatten(),
        Some("ldap netgroup ucore".to_string())
    );
    assert_eq!(
        history
            .with(|store| store.expand(&BangCommand::Prefix("ldap user".to_string()), None, false))
            .ok()
            .flatten(),
        Some("ldap user oistes".to_string())
    );
}

#[test]
fn submission_delegates_help_and_exit_to_host() {
    let history = disabled_history();
    let mut seen = Vec::new();
    let mut execute = |event: crate::repl::ReplEvent<'_>, _: &SharedHistory| {
        let crate::repl::ReplEvent::Line(line) = event else {
            unreachable!()
        };
        seen.push(line.to_string());
        Ok(match line {
            "help" => ReplResult::Print("host help".to_string()),
            "!!" => ReplResult::ReplaceInput("ldap user oistes".to_string()),
            "exit" => ReplResult::Exit(7),
            other => ReplResult::Print(other.to_string()),
        })
    };
    let mut submission = SubmissionContext {
        root_input: false,
        exit_on_escape: false,
        history_store: &history,
        execute: &mut execute,
    };

    let help = evaluate_repl_submission("help", &mut submission);
    let bang = evaluate_repl_submission("!!", &mut submission);
    let exit = evaluate_repl_submission("exit", &mut submission);

    assert!(matches!(help, ReplResult::Print(text) if text == "host help"));
    assert!(matches!(bang, ReplResult::ReplaceInput(text) if text == "ldap user oistes"));
    assert!(matches!(exit, ReplResult::Exit(7)));
    assert_eq!(
        seen,
        vec!["help".to_string(), "!!".to_string(), "exit".to_string()]
    );
}

// Completion and highlight adapter contracts.

#[test]
fn completer_covers_prefix_fuzzy_and_pipe_scenarios_unit() {
    let mut root = CompletionNode::default();
    for command in ["ldap", "plugins", "theme"] {
        root.children
            .insert(command.to_string(), CompletionNode::default());
    }
    let tree = CompletionTree {
        root,
        pipe_verbs: default_pipe_verbs(),
    };
    let mut word_completer = ReplCompleter::new(tree.clone(), None);

    let prefix_values = word_completer
        .complete("ld", 2)
        .suggestions()
        .to_vec()
        .into_iter()
        .map(|suggestion| suggestion.value)
        .collect::<Vec<_>>();
    assert_eq!(prefix_values, vec!["ldap".to_string()]);

    let fuzzy_values = word_completer
        .complete("lap", 3)
        .suggestions()
        .to_vec()
        .into_iter()
        .map(|suggestion| suggestion.value)
        .collect::<Vec<_>>();
    assert!(fuzzy_values.contains(&"ldap".to_string()));

    let mut pipe_completer = ReplCompleter::new(tree, None);
    let pipe_values = pipe_completer
        .complete("ldap user | F", "ldap user | F".len())
        .suggestions()
        .to_vec()
        .into_iter()
        .map(|suggestion| suggestion.value)
        .collect::<Vec<_>>();
    assert!(pipe_values.contains(&"F".to_string()));
}

#[test]
fn default_pipe_verbs_include_extended_dsl_surface() {
    let verbs = default_pipe_verbs();

    assert_eq!(
        verbs.get("?"),
        Some(&"Clean rows / exists filter".to_string())
    );
    assert_eq!(verbs.get("JQ"), Some(&"Run jq-like expression".to_string()));
    assert_eq!(verbs.get("VALUE"), Some(&"Extract values".to_string()));
}

#[test]
fn completer_can_use_projected_line_for_host_flags_unit() {
    let tree = completion_tree_with_config_show();
    let projector =
        Arc::new(|line: &str| LineProjection::passthrough(line.replacen("--json", "      ", 1)));
    let mut completer = ReplCompleter::new(tree, Some(projector));

    let completions = completer.complete("--json config sh", "--json config sh".len());
    let values = completions
        .suggestions()
        .to_vec()
        .into_iter()
        .map(|suggestion| suggestion.value)
        .collect::<Vec<_>>();

    assert!(values.contains(&"show".to_string()));
}

#[test]
fn completer_hides_suggestions_requested_by_projection_unit() {
    let mut root = CompletionNode::default();
    root.flags
        .insert("--json".to_string(), FlagNode::new().flag_only());
    root.flags
        .insert("--debug".to_string(), FlagNode::new().flag_only());
    let tree = CompletionTree {
        root,
        ..CompletionTree::default()
    };
    let projector = Arc::new(|line: &str| {
        let mut hidden = BTreeSet::new();
        hidden.insert("--json".to_string());
        LineProjection {
            line: line.to_string(),
            hidden_suggestions: hidden,
        }
    });
    let mut completer = ReplCompleter::new(tree, Some(projector));

    let values = completer
        .complete("-", 1)
        .suggestions()
        .to_vec()
        .into_iter()
        .map(|suggestion| suggestion.value)
        .collect::<Vec<_>>();

    assert!(!values.contains(&"--json".to_string()));
    assert!(values.contains(&"--debug".to_string()));
}

#[test]
fn completer_uses_engine_metadata_for_subcommands() {
    let mut ldap = CompletionNode {
        tooltip: Some("Directory lookup".to_string()),
        ..CompletionNode::default()
    };
    ldap.children
        .insert("user".to_string(), CompletionNode::default());
    ldap.children
        .insert("host".to_string(), CompletionNode::default());

    let tree = CompletionTree {
        root: CompletionNode::default().with_child("ldap", ldap),
        ..CompletionTree::default()
    };

    let mut completer = ReplCompleter::new(tree, None);
    let completion = completer
        .complete("ld", 2)
        .suggestions()
        .to_vec()
        .into_iter()
        .find(|item| item.value == "ldap")
        .expect("ldap completion should exist");

    assert!(completion.description.as_deref().is_some_and(|value| {
        value.contains("Directory lookup")
            && value.contains("subcommands:")
            && value.contains("host")
            && value.contains("user")
    }));
}

#[test]
fn color_parser_extracts_hex_and_named_colors() {
    assert_eq!(
        color_from_style_spec("bold #ff79c6"),
        Some(Color::Rgb(255, 121, 198))
    );
    assert_eq!(
        color_from_style_spec("fg:cyan underline"),
        Some(Color::Cyan)
    );
    assert_eq!(color_from_style_spec("bg:ansi141"), Some(Color::Fixed(141)));
    assert_eq!(
        color_from_style_spec("fg:rgb(80,250,123)"),
        Some(Color::Rgb(80, 250, 123))
    );
    assert!(color_from_style_spec("not-a-color").is_none());
}

// Debug, editor, and path-helper contracts.

#[test]
fn debug_step_parse_round_trips_known_values_unit() {
    assert_eq!(DebugStep::Tab.as_str(), "tab");
    assert_eq!(DebugStep::Up.as_str(), "up");
    assert_eq!(DebugStep::Down.as_str(), "down");
    assert_eq!(DebugStep::Left.as_str(), "left");
    assert_eq!(DebugStep::parse("shift-tab"), Some(DebugStep::BackTab));
    assert_eq!(DebugStep::parse("ENTER"), Some(DebugStep::Accept));
    assert_eq!(DebugStep::parse("esc"), Some(DebugStep::Close));
    assert_eq!(DebugStep::Right.as_str(), "right");
    assert_eq!(DebugStep::parse("wat"), None);
}

#[test]
fn debug_completion_and_steps_surface_menu_state_unit() {
    let tree = completion_tree_with_config_show();
    let debug = debug_completion(
        &tree,
        "config sh",
        "config sh".len(),
        CompletionDebugOptions::new(80, 6),
    );
    assert_eq!(debug.stub, "sh");
    assert!(debug.matches.iter().any(|item| item.id == "show"));

    let frames = debug_completion_steps(
        &tree,
        "config sh",
        "config sh".len(),
        CompletionDebugOptions::new(80, 6),
        &[DebugStep::Tab],
    );
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].step, "tab");
    assert_eq!(frames[0].state.line, "config show ");
}

#[cfg_attr(miri, ignore = "filesystem-backed path completion integration test")]
#[test]
fn autocomplete_policy_and_path_helpers_cover_editing_and_lookup_edges_unit() {
    let painted = PaintedLine::default();
    let edit_mode = |mode| {
        AutoCompleteEmacs::new(
            Emacs::default(),
            SharedCompletionMenu::new(OspCompletionMenu::default()),
            mode,
            painted.clone(),
        )
    };
    let insert = |ch| [EditCommand::InsertChar(ch)];
    let tab = edit_mode(ReplCompleteOn::default());
    assert!(tab.opens_menu(&insert('-')));
    painted.set("host db", 7);
    assert!(!tab.opens_menu(&insert('-')));
    painted.set("", 0);
    assert!(!tab.opens_menu(&insert('x')));
    assert!(!tab.opens_menu(&insert(' ')));
    assert!(
        edit_mode(ReplCompleteOn {
            after_letters: Some(1),
            space: true,
            ..ReplCompleteOn::default()
        })
        .opens_menu(&insert('x'))
    );
    let after_two = edit_mode(ReplCompleteOn {
        after_letters: Some(2),
        ..ReplCompleteOn::default()
    });
    painted.set("ldap h", 6);
    assert!(after_two.opens_menu(&insert('o')));
    painted.set("ldap ", 5);
    assert!(!after_two.opens_menu(&insert('h')));

    let missing = path_suggestions(
        "/definitely/not/a/real/dir/",
        "/definitely/not/a/real/dir/",
        None,
        reedline::Span { start: 0, end: 0 },
    );
    assert!(missing.is_empty());

    let (lookup, insert_prefix, typed_prefix) = split_path_stub("/tmp/demo/");
    assert_eq!(lookup, PathBuf::from("/tmp/demo/"));
    assert_eq!(insert_prefix, "/tmp/demo/");
    assert!(typed_prefix.is_empty());

    let (lookup, insert_prefix, typed_prefix) = split_path_stub("do");
    assert_eq!(lookup, PathBuf::from("."));
    assert_eq!(insert_prefix, "");
    assert_eq!(typed_prefix, "do");
}

#[test]
fn completion_debug_options_builders_cover_appearance_and_empty_steps_unit() {
    let appearance = test_appearance();
    let options = CompletionDebugOptions::new(120, 40)
        .with_ansi(true)
        .with_unicode(true)
        .with_appearance(Some(&appearance));

    assert_eq!(options.width, 120);
    assert_eq!(options.height, 40);
    assert!(options.ansi);
    assert!(options.unicode);
    assert!(options.appearance.is_some());

    let tree = completion_tree_with_config_show();
    let frames = debug_completion_steps(&tree, "config sh", 9, options, &[]);
    assert!(frames.is_empty());
}

#[test]
fn debug_completion_navigation_and_empty_match_states_unit() {
    let tree = completion_tree_with_config_show();
    let frames = debug_completion_steps(
        &tree,
        "config sh",
        9,
        CompletionDebugOptions::new(80, 6),
        &[
            DebugStep::Tab,
            DebugStep::Down,
            DebugStep::Right,
            DebugStep::Left,
            DebugStep::Up,
            DebugStep::BackTab,
            DebugStep::Close,
        ],
    );

    assert_eq!(frames.len(), 7);
    assert_eq!(frames[0].step, "tab");
    assert_eq!(frames[1].step, "down");
    assert_eq!(frames[2].step, "right");
    assert_eq!(frames[3].step, "left");
    assert_eq!(frames[4].step, "up");
    assert_eq!(frames[5].step, "backtab");
    assert_eq!(frames[6].step, "close");
    let debug = debug_completion(&tree, "zzz", 99, CompletionDebugOptions::new(80, 6));

    assert_eq!(debug.line, "zzz");
    assert_eq!(debug.cursor, 3);
    assert!(debug.matches.is_empty());
    assert_eq!(debug.menu.selected_index, -1);
    assert_eq!(debug.stub, "zzz");
    assert_eq!(debug.replace_range, [0, 3]);
}

#[test]
fn completion_analysis_marks_uncommitted_root_tokens_as_subcommand_context_unit() {
    let root_tree = completion_tree_with_root_commands();
    let engine = crate::completion::CompletionEngine::new(root_tree.clone());
    let analysis = engine.analyze("help", "help".len());
    assert_eq!(analysis.cursor.token_stub, "help");
    assert_eq!(analysis.context.matched_path, Vec::<String>::new());
    assert!(analysis.context.subcommand_context);
}

#[test]
fn evaluate_repl_submission_handles_restart_and_error_paths_unit() {
    let history = disabled_history();

    let mut restart_execute = |_line: crate::repl::ReplEvent<'_>, _: &SharedHistory| {
        Ok(ReplResult::Restart {
            output: "restarting".to_string(),
            reload: ReplReloadKind::WithIntro,
        })
    };
    let mut submission = SubmissionContext {
        root_input: false,
        exit_on_escape: false,
        history_store: &history,
        execute: &mut restart_execute,
    };
    let restart = evaluate_repl_submission("config set", &mut submission);
    assert!(matches!(
        restart,
        ReplResult::Restart {
            output,
            reload: ReplReloadKind::WithIntro
        } if output == "restarting"
    ));

    let mut failing_execute =
        |_line: crate::repl::ReplEvent<'_>, _: &SharedHistory| -> anyhow::Result<ReplResult> {
            Err(anyhow::anyhow!("submit failed"))
        };
    let mut failing_submission = SubmissionContext {
        root_input: false,
        exit_on_escape: false,
        history_store: &history,
        execute: &mut failing_execute,
    };
    let result = evaluate_repl_submission("broken", &mut failing_submission);
    assert!(matches!(result, ReplResult::Continue));

    let mut noop_execute = |_line: crate::repl::ReplEvent<'_>, _: &SharedHistory| {
        Ok(ReplResult::Print("ignored".to_string()))
    };
    let mut noop_submission = SubmissionContext {
        root_input: false,
        exit_on_escape: false,
        history_store: &history,
        execute: &mut noop_execute,
    };
    let result = evaluate_repl_submission("   ", &mut noop_submission);
    assert!(matches!(result, ReplResult::Continue));
}

#[test]
fn highlighter_builder_requires_command_color_unit() {
    let tree = completion_tree_with_config_show();
    let none = build_repl_highlighter(tree.clone(), &super::ReplAppearance::default(), None);
    assert!(none.is_none());

    let some = build_repl_highlighter(
        tree.clone(),
        &ReplAppearance::builder()
            .with_command_highlight_style(Some("green".to_string()))
            .build(),
        None,
    );
    assert!(some.is_some());
}

#[cfg_attr(miri, ignore = "filesystem-backed path completion integration test")]
#[test]
fn path_suggestions_cover_files_directories_and_quote_variants_unit() {
    let root = make_temp_dir("osp-repl-paths");
    std::fs::write(root.join("alpha.txt"), "x").expect("file should be written");
    std::fs::create_dir_all(root.join("alpine")).expect("dir should be created");
    std::fs::write(root.join("team docs.txt"), "x").expect("file should be written");

    let file_and_dir_stub = format!("{}/al", root.display());
    let file_and_dir = path_suggestions(
        &file_and_dir_stub,
        &file_and_dir_stub,
        None,
        reedline::Span {
            start: 0,
            end: file_and_dir_stub.len(),
        },
    );
    let values = file_and_dir
        .iter()
        .map(|item| {
            (
                item.value.clone(),
                item.description.clone(),
                item.append_whitespace,
            )
        })
        .collect::<Vec<_>>();

    assert!(values.iter().any(|(value, desc, append)| {
        value.ends_with("alpha.txt") && desc.as_deref() == Some("file") && *append
    }));
    assert!(values.iter().any(|(value, desc, append)| {
        value.ends_with("alpine/") && desc.as_deref() == Some("dir") && !*append
    }));

    let spaced_stub = format!("{}/te", root.display());
    let unquoted = path_suggestions(
        &spaced_stub,
        &spaced_stub,
        None,
        reedline::Span {
            start: 0,
            end: spaced_stub.len(),
        },
    );
    assert!(
        unquoted
            .iter()
            .any(|item| item.value.ends_with("team\\ docs.txt"))
    );

    let quoted_raw_stub = format!("\"{spaced_stub}");
    let quoted = path_suggestions(
        &quoted_raw_stub,
        &spaced_stub,
        Some(QuoteStyle::Double),
        reedline::Span {
            start: 0,
            end: quoted_raw_stub.len(),
        },
    );
    assert!(
        quoted
            .iter()
            .any(|item| item.value.ends_with("team docs.txt\""))
    );
}

#[cfg_attr(miri, ignore = "trace file filesystem integration test")]
#[test]
fn trace_completion_env_controls_and_jsonl_output_unit() {
    if crate::tests::isolated_test() {
        return;
    }

    let temp_dir = make_temp_dir("osp-repl-trace");
    let trace_path = temp_dir.join("trace.jsonl");
    let previous_enabled = std::env::var("OSP_REPL_TRACE_COMPLETION").ok();
    let previous_path = std::env::var("OSP_REPL_TRACE_PATH").ok();
    set_env_var_for_test("OSP_REPL_TRACE_COMPLETION", "1");
    set_env_var_for_test("OSP_REPL_TRACE_PATH", &trace_path);

    assert!(trace_completion_enabled());
    trace_completion(super::CompletionTraceEvent {
        event: "complete",
        line: "config sh",
        cursor: 9,
        stub: "sh",
        matches: vec!["show".to_string()],
        replace_range: Some([7, 9]),
        menu: None,
        buffer_before: None,
        buffer_after: None,
        cursor_before: None,
        cursor_after: None,
        accepted_value: None,
    });

    let contents = std::fs::read_to_string(&trace_path).expect("trace file should exist");
    assert!(contents.contains("\"event\":\"complete\""));
    assert!(contents.contains("\"stub\":\"sh\""));
    set_env_var_for_test("OSP_REPL_TRACE_COMPLETION", "off");
    assert!(!trace_completion_enabled());
    set_env_var_for_test("OSP_REPL_TRACE_COMPLETION", "yes");
    assert!(trace_completion_enabled());

    restore_env("OSP_REPL_TRACE_COMPLETION", previous_enabled);
    restore_env("OSP_REPL_TRACE_PATH", previous_path);
}

#[test]
fn cursor_position_reports_parse_only_well_formed_replies_unit() {
    assert_eq!(parse_cursor_position_report(b"\x1b[12;34R"), Some((34, 12)));
    assert_eq!(
        parse_cursor_position_report(b"\x1b[1;200R trailing"),
        Some((200, 1))
    );
    assert_eq!(parse_cursor_position_report(b"\x1b[;34R"), None);
    assert_eq!(parse_cursor_position_report(b"\x1b[12;R"), None);
    assert_eq!(parse_cursor_position_report(b"\x1b[12;34"), None);
}

#[test]
fn run_repl_with_reason_and_basic_input_detection_cover_basic_and_interactive_modes_unit() {
    assert_eq!(
        basic_input_reason(ReplInputMode::Basic),
        Some(BasicInputReason::Explicit)
    );

    for reason in [
        Some(BasicInputReason::Explicit),
        Some(BasicInputReason::NotATerminal),
        None,
    ] {
        let history = SharedHistory::new(history_config());
        let prompt = OspPrompt::new("left".to_string(), "> ".to_string(), None);
        let mut execute =
            |_line: crate::repl::ReplEvent<'_>, _history: &SharedHistory| Ok(ReplResult::Continue);
        let mut submission = SubmissionContext {
            root_input: false,
            exit_on_escape: false,
            history_store: &history,
            execute: &mut execute,
        };
        let mut basic_calls = 0usize;
        let mut interactive_calls = 0usize;

        let result = super::run_repl_with_reason(
            super::ReplRunContext {
                prompt,
                completion_tree: completion_tree_with_config_show(),
                appearance: test_appearance(),
                complete_on: ReplCompleteOn::default(),
                autosuggest: true,
                line_projector: None,
                history_store: history.clone(),
                actions: Vec::new(),
                on_start: None,
            },
            reason,
            &mut submission,
            |basic_prompt, _submission, _after_paint| {
                basic_calls += 1;
                assert_eq!(basic_prompt.left(), "left");
                Ok(ReplResult::Exit(7))
            },
            |config, interactive_history, _submission| {
                interactive_calls += 1;
                assert_eq!(config.prompt.left(), "left");
                assert!(config.completion_tree.root.children.contains_key("config"));
                assert_eq!(config.appearance.history_menu_rows, 5);
                assert_eq!(
                    interactive_history
                        .with(|store| store.history_enabled())
                        .unwrap_or(false),
                    history
                        .with(|store| store.history_enabled())
                        .unwrap_or(false)
                );
                assert_eq!(
                    interactive_history
                        .with(|store| store.recent_commands_for(store.shell_prefix().as_deref()))
                        .unwrap_or_default(),
                    history
                        .with(|store| store.recent_commands_for(store.shell_prefix().as_deref()))
                        .unwrap_or_default()
                );
                Ok(ReplResult::Exit(7))
            },
        )
        .expect("routing path should succeed");

        match reason {
            Some(_) => {
                assert_eq!(result, ReplResult::Exit(7));
                assert_eq!(basic_calls, 1);
                assert_eq!(interactive_calls, 0);
            }
            None => {
                assert_eq!(result, ReplResult::Exit(7));
                assert_eq!(basic_calls, 0);
                assert_eq!(interactive_calls, 1);
            }
        }
    }
}

#[test]
fn expand_home_and_prompt_renderers_behave_unit() {
    if crate::tests::isolated_test() {
        return;
    }

    let previous_home = std::env::var("HOME").ok();
    let home = crate::tests::make_temp_dir("osp-env");
    set_env_var_for_test("HOME", home.path().to_str().unwrap());
    assert_eq!(
        crate::normalize::expand_home("~"),
        Some(std::path::PathBuf::from(home.path()))
    );
    assert_eq!(
        crate::normalize::expand_home("~/cache"),
        Some(home.join("cache"))
    );
    assert_eq!(
        crate::normalize::expand_home("~\\cache"),
        Some(home.join("cache"))
    );
    assert_eq!(
        crate::normalize::expand_home("/etc/hosts"),
        Some(std::path::PathBuf::from("/etc/hosts"))
    );

    let right: PromptRightRenderer = Arc::new(|| "rhs".to_string());
    let prompt = OspPrompt::new("left".to_string(), "> ".to_string(), Some(right));
    assert_eq!(prompt.render_prompt_left(), "left");
    assert_eq!(prompt.render_prompt_right(), "rhs");
    assert_eq!(
        prompt.render_prompt_indicator(PromptEditMode::Default),
        "> "
    );
    assert_eq!(prompt.render_prompt_multiline_indicator(), "... ");
    assert_eq!(
        prompt.render_prompt_history_search_indicator(PromptHistorySearch {
            status: PromptHistorySearchStatus::Passing,
            term: "ldap".to_string(),
        }),
        "(reverse-search: ldap) "
    );

    restore_env("HOME", previous_home);
}

#[test]
fn interactive_editor_builder_and_driver_cover_signal_and_error_paths_unit() {
    let history = disabled_history();
    let appearance = test_appearance();
    let built_editor = super::session::build_interactive_editor(
        completion_tree_with_root_commands(),
        &appearance,
        ReplCompleteOn::default(),
        true,
        None,
        history.clone(),
        None,
    );
    assert_eq!(built_editor.current_buffer_contents(), "");

    let executed = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&executed);
    let mut execute = move |event: crate::repl::ReplEvent<'_>, _: &SharedHistory| {
        let crate::repl::ReplEvent::Line(line) = event else {
            unreachable!()
        };
        seen.lock()
            .expect("executed command log should not be poisoned")
            .push(line.to_string());
        Ok(ReplResult::Continue)
    };
    let mut submission = SubmissionContext {
        root_input: false,
        exit_on_escape: false,
        history_store: &history,
        execute: &mut execute,
    };
    let mut editor = reedline::Reedline::create();
    let mut reads = 0usize;
    let exit = super::session::drive_interactive_editor(
        &appearance,
        &history,
        &mut submission,
        &mut editor,
        |_editor| {
            reads += 1;
            Ok(if reads == 1 {
                reedline::Signal::Success("help".to_string())
            } else {
                reedline::Signal::CtrlD
            })
        },
    )
    .expect("success and ctrl-d signals should succeed");
    assert_eq!(exit, ReplResult::Exit(0));
    assert_eq!(
        *executed
            .lock()
            .expect("executed command log should not be poisoned"),
        vec!["help".to_string()]
    );

    let mut error_execute =
        |_line: crate::repl::ReplEvent<'_>, _history: &SharedHistory| Ok(ReplResult::Continue);
    let mut error_submission = SubmissionContext {
        root_input: false,
        exit_on_escape: false,
        history_store: &history,
        execute: &mut error_execute,
    };
    let err = super::session::drive_interactive_editor(
        &appearance,
        &history,
        &mut error_submission,
        &mut reedline::Reedline::create(),
        |_editor| Err(io::Error::other("boom")),
    )
    .expect_err("editor errors should bubble up");
    assert!(err.to_string().contains("boom"));
}

// History picker contracts.

#[test]
fn history_picker_items_keep_latest_unique_commands_and_flatten_multiline_unit() {
    let history = test_history(&[
        "doctor all",
        "help doctor --mreg",
        "doctor all",
        "first line\nsecond line",
    ]);

    let items = history_picker_items(&history);

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].command, "first line\nsecond line");
    assert!(items[0].label.contains("4  first line \\n second line"));
    assert_eq!(items[1].command, "doctor all");
    assert_eq!(items[2].command, "help doctor --mreg");
    assert_eq!(items[1].matching_range[0].1, items[1].label.len());
}

#[test]
fn history_picker_options_use_configured_rows_query_and_skin_unit() {
    let appearance = ReplAppearance::builder()
        .with_completion_text_style(Some("white".to_string()))
        .with_completion_background_style(Some("black".to_string()))
        .with_completion_highlight_style(Some("cyan".to_string()))
        .with_command_highlight_style(Some("green".to_string()))
        .with_history_menu_rows(7)
        .build();

    let options = build_history_picker_options(&appearance, "doctor mreg")
        .expect("history picker options should build");

    assert_eq!(options.height, "8");
    assert_eq!(options.query.as_deref(), Some("doctor mreg"));
    assert_eq!(options.prompt, "history> ");
    assert!(options.no_info);
    assert_eq!(
        options.color.as_deref(),
        Some(
            "normal:7,matched:7:underlined,current:7,current_match:7:underlined,query:7,prompt:7,cursor:7,selected:7,info:7,header:7,spinner:7,border:7,bg:0,matched_bg:0,current_bg:6,current_match_bg:6"
        )
    );
}

fn make_temp_dir(prefix: &str) -> crate::tests::TestTempDir {
    crate::tests::make_temp_dir(prefix)
}

fn restore_env(key: &str, value: Option<String>) {
    if let Some(value) = value {
        set_env_var_for_test(key, value);
    } else {
        remove_env_var_for_test(key);
    }
}

fn set_env_var_for_test(key: &str, value: impl AsRef<std::ffi::OsStr>) {
    // Only the isolated child test reaches this environment mutation.
    unsafe {
        std::env::set_var(key, value);
    }
}

fn remove_env_var_for_test(key: &str) {
    // See `set_env_var_for_test`; these tests intentionally restore the
    // process environment after probing env-dependent behavior.
    unsafe {
        std::env::remove_var(key);
    }
}

fn test_history(commands: &[&str]) -> SharedHistory {
    let history = SharedHistory::new(HistoryConfig {
        max_entries: 32,
        shell_context: HistoryShellContext::default(),
        ..history_config()
    });

    for command in commands {
        history
            .with(|store| store.save_command_line(command))
            .expect("history lock should be available")
            .expect("history save should succeed");
    }

    history
}
