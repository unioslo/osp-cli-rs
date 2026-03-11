use crate::completion::{ArgNode, CompletionNode, CompletionTree, FlagNode, SuggestionEntry};
use nu_ansi_term::Color;
use reedline::Span;
use reedline::{
    Completer, EditCommand, Menu, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus,
};
use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{
    AutoCompleteEmacs, BasicInputReason, CompletionDebugOptions, DebugStep, HISTORY_MENU_NAME,
    HistoryConfig, HistoryShellContext, OspPrompt, PromptRightRenderer, ReplAppearance,
    ReplCompleter, ReplHistoryCompleter, ReplInputMode, ReplLineResult, ReplReloadKind,
    ReplRunResult, SharedHistory, SubmissionContext, SubmissionResult, basic_input_reason,
    build_history_menu, build_history_picker_options, build_repl_highlighter,
    color_from_style_spec, contains_cursor_position_report, debug_completion,
    debug_completion_steps, debug_history_menu, debug_history_menu_steps, default_pipe_verbs,
    expand_history, expand_home, history_picker_items, is_cursor_position_error,
    parse_cursor_position_report, path_suggestions, process_submission, split_path_stub,
    trace_completion, trace_completion_enabled,
};
use crate::core::shell_words::QuoteStyle;
use crate::repl::LineProjection;

fn env_lock() -> &'static Mutex<()> {
    crate::tests::env_lock()
}

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

fn completion_tree_with_root_and_config_show() -> CompletionTree {
    let show = CompletionNode::default()
        .with_flag("--sources", FlagNode::new().flag_only())
        .with_flag("--raw", FlagNode::new().flag_only());
    let config = CompletionNode::default()
        .with_child("show", show)
        .with_child("get", CompletionNode::default())
        .with_child("explain", CompletionNode::default());

    let root = CompletionNode::default()
        .with_child("help", CompletionNode::default())
        .with_child("exit", CompletionNode::default())
        .with_child("quit", CompletionNode::default())
        .with_child("config", config)
        .with_child("doctor", CompletionNode::default());

    CompletionTree {
        root,
        ..CompletionTree::default()
    }
}

fn completion_tree_with_theme_show_values() -> CompletionTree {
    let show = CompletionNode {
        args: vec![ArgNode::named("theme").suggestions([
            SuggestionEntry::value("catppuccin"),
            SuggestionEntry::value("dracula"),
            SuggestionEntry::value("gruvbox"),
        ])],
        ..CompletionNode::default()
    };

    let theme = CompletionNode::default().with_child("show", show);
    let root = CompletionNode::default().with_child("theme", theme);

    CompletionTree {
        root,
        ..CompletionTree::default()
    }
}

fn suggestion_ids(outputs: Vec<crate::completion::SuggestionOutput>) -> Vec<String> {
    outputs
        .into_iter()
        .filter_map(|output| match output {
            crate::completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::completion::SuggestionOutput::PathSentinel => None,
        })
        .collect()
}

fn history_config() -> super::HistoryConfigBuilder {
    HistoryConfig::builder()
        .with_enabled(true)
        .with_max_entries(32)
        .with_dedupe(false)
        .with_profile_scoped(false)
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
    SharedHistory::new(
        history_config()
            .with_enabled(false)
            .with_max_entries(0)
            .with_shell_context(HistoryShellContext::default())
            .build(),
    )
    .expect("history config should build")
}

// History expansion and submission contracts.

#[test]
fn expands_double_bang() {
    let history = vec!["ldap user oistes".to_string()];
    assert_eq!(
        expand_history("!!", &history, None, false),
        Some("ldap user oistes".to_string())
    );
}

#[test]
fn expands_relative() {
    let history = vec![
        "ldap user oistes".to_string(),
        "ldap netgroup ucore".to_string(),
    ];
    assert_eq!(
        expand_history("!-1", &history, None, false),
        Some("ldap netgroup ucore".to_string())
    );
}

#[test]
fn expands_prefix() {
    let history = vec![
        "ldap user oistes".to_string(),
        "ldap netgroup ucore".to_string(),
    ];
    assert_eq!(
        expand_history("!ldap user", &history, None, false),
        Some("ldap user oistes".to_string())
    );
}

#[test]
fn submission_delegates_help_and_exit_to_host() {
    let history = disabled_history();
    let mut seen = Vec::new();
    let mut execute = |line: &str, _: &SharedHistory| {
        seen.push(line.to_string());
        Ok(match line {
            "help" => ReplLineResult::Continue("host help".to_string()),
            "!!" => ReplLineResult::ReplaceInput("ldap user oistes".to_string()),
            "exit" => ReplLineResult::Exit(7),
            other => ReplLineResult::Continue(other.to_string()),
        })
    };
    let mut submission = SubmissionContext {
        history_store: &history,
        execute: &mut execute,
    };

    let help = process_submission("help", &mut submission).expect("help should succeed");
    let bang = process_submission("!!", &mut submission).expect("bang should succeed");
    let exit = process_submission("exit", &mut submission).expect("exit should succeed");

    assert!(matches!(help, SubmissionResult::Print(text) if text == "host help"));
    assert!(matches!(bang, SubmissionResult::ReplaceInput(text) if text == "ldap user oistes"));
    assert!(matches!(exit, SubmissionResult::Exit(7)));
    assert_eq!(
        seen,
        vec!["help".to_string(), "!!".to_string(), "exit".to_string()]
    );
}

// Completion and highlight adapter contracts.

#[test]
fn completer_covers_prefix_fuzzy_and_pipe_scenarios_unit() {
    let mut word_completer = ReplCompleter::new(
        vec![
            "ldap".to_string(),
            "plugins".to_string(),
            "theme".to_string(),
        ],
        None,
        None,
    );

    let prefix_values = word_completer
        .complete("ld", 2)
        .into_iter()
        .map(|suggestion| suggestion.value)
        .collect::<Vec<_>>();
    assert_eq!(prefix_values, vec!["ldap".to_string()]);

    let fuzzy_values = word_completer
        .complete("lap", 3)
        .into_iter()
        .map(|suggestion| suggestion.value)
        .collect::<Vec<_>>();
    assert!(fuzzy_values.contains(&"ldap".to_string()));

    let mut pipe_completer = ReplCompleter::new(vec!["ldap".to_string()], None, None);
    let pipe_values = pipe_completer
        .complete("ldap user | F", "ldap user | F".len())
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
fn completer_with_tree_does_not_fallback_to_word_list() {
    let mut root = CompletionNode::default();
    root.children
        .insert("config".to_string(), CompletionNode::default());
    let tree = CompletionTree {
        root,
        ..CompletionTree::default()
    };

    let mut completer = ReplCompleter::new(vec!["ldap".to_string()], Some(tree), None);
    let completions = completer.complete("zzz", 3);
    assert!(completions.is_empty());
}

#[test]
fn completer_can_use_projected_line_for_host_flags_unit() {
    let tree = completion_tree_with_config_show();
    let projector =
        Arc::new(|line: &str| LineProjection::passthrough(line.replacen("--json", "      ", 1)));
    let mut completer = ReplCompleter::new(Vec::new(), Some(tree), Some(projector));

    let completions = completer.complete("--json config sh", "--json config sh".len());
    let values = completions
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
    let mut completer = ReplCompleter::new(Vec::new(), Some(tree), Some(projector));

    let values = completer
        .complete("-", 1)
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

    let mut completer = ReplCompleter::new(Vec::new(), Some(tree), None);
    let completion = completer
        .complete("ld", 2)
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
        &[DebugStep::Tab, DebugStep::Accept],
    );
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].step, "tab");
    assert!(frames[0].state.matches.iter().any(|item| item.id == "show"));
    assert_eq!(frames[1].step, "accept");
    assert_eq!(frames[1].state.line, "config show ");
}

#[test]
fn autocomplete_policy_and_path_helpers_cover_editing_and_lookup_edges_unit() {
    assert!(AutoCompleteEmacs::should_reopen_menu(&[
        EditCommand::InsertChar('x')
    ]));
    assert!(AutoCompleteEmacs::should_reopen_menu(&[
        EditCommand::BackspaceWord
    ]));
    assert!(!AutoCompleteEmacs::should_reopen_menu(&[
        EditCommand::MoveToStart { select: false }
    ]));
    assert!(!AutoCompleteEmacs::should_reopen_menu(&[
        EditCommand::MoveToLineEnd { select: false }
    ]));

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
    assert_eq!(debug.selected, -1);
    assert_eq!(debug.stub, "zzz");
    assert_eq!(debug.replace_range, [0, 3]);
}

#[test]
fn debug_completion_keeps_same_token_scope_until_a_space_commits_the_subcommand_unit() {
    let tree = completion_tree_with_config_show();

    let frames = debug_completion_steps(
        &tree,
        "config ",
        "config ".len(),
        CompletionDebugOptions::new(80, 6),
        &[DebugStep::Tab, DebugStep::Tab, DebugStep::Tab],
    );

    assert_eq!(frames[0].state.line, "config ");
    assert!(frames[0].state.matches.iter().any(|item| item.id == "show"));
    assert!(frames[0].state.matches.iter().any(|item| item.id == "get"));
    assert!(
        frames[0]
            .state
            .matches
            .iter()
            .any(|item| item.id == "explain")
    );

    assert!(matches!(
        frames[1].state.line.as_str(),
        "config explain" | "config get" | "config show"
    ));
    assert!(frames[1].state.matches.iter().any(|item| item.id == "show"));
    assert!(frames[1].state.matches.iter().any(|item| item.id == "get"));
    assert!(
        frames[1]
            .state
            .matches
            .iter()
            .any(|item| item.id == "explain")
    );

    assert!(matches!(
        frames[2].state.line.as_str(),
        "config explain" | "config get" | "config show"
    ));
    assert_ne!(frames[2].state.line, frames[1].state.line);
    assert!(frames[2].state.matches.iter().any(|item| item.id == "show"));
    assert!(frames[2].state.matches.iter().any(|item| item.id == "get"));
    assert!(
        frames[2]
            .state
            .matches
            .iter()
            .any(|item| item.id == "explain")
    );
}

#[test]
fn debug_completion_switches_to_show_flags_once_the_subcommand_is_committed_unit() {
    let tree = completion_tree_with_config_show();
    let debug = debug_completion(
        &tree,
        "config show ",
        "config show ".len(),
        CompletionDebugOptions::new(80, 6),
    );

    let ids = debug
        .matches
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["--raw", "--sources"]);
}

#[test]
fn debug_completion_keeps_root_command_scope_until_space_commits_command_unit() {
    let tree = completion_tree_with_root_commands();
    let engine = crate::completion::CompletionEngine::new(tree.clone());
    let analysis = engine.analyze("help", "help".len());
    assert_eq!(analysis.cursor.token_stub, "help");
    assert_eq!(analysis.context.matched_path, Vec::<String>::new());
    assert!(analysis.context.subcommand_context);

    let debug = debug_completion(
        &tree,
        "help",
        "help".len(),
        CompletionDebugOptions::new(80, 6),
    );

    let ids = debug
        .matches
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["config", "exit", "help", "quit"]);
}

#[test]
fn contract_first_tab_opens_the_menu_for_the_current_slot_unit() {
    let engine =
        crate::completion::CompletionEngine::new(completion_tree_with_root_and_config_show());

    let root = suggestion_ids(engine.complete("", 0).1);
    assert!(root.contains(&"help".to_string()));
    assert!(root.contains(&"config".to_string()));
    assert!(root.contains(&"doctor".to_string()));

    let config = suggestion_ids(engine.complete("config ", "config ".len()).1);
    assert_eq!(config, vec!["explain", "get", "show"]);

    let show = suggestion_ids(engine.complete("config show ", "config show ".len()).1);
    assert_eq!(show, vec!["--raw", "--sources"]);
}

#[test]
fn contract_token_is_not_committed_until_there_is_a_delimiter_unit() {
    let engine = crate::completion::CompletionEngine::new(completion_tree_with_config_show());

    let siblings = suggestion_ids(engine.complete("config show", "config show".len()).1);
    assert_eq!(siblings, vec!["explain", "get", "show"]);

    let children = suggestion_ids(engine.complete("config show ", "config show ".len()).1);
    assert_eq!(children, vec!["--raw", "--sources"]);
}

#[test]
fn contract_exact_matches_without_trailing_space_stay_in_sibling_scope_unit() {
    let engine =
        crate::completion::CompletionEngine::new(completion_tree_with_root_and_config_show());

    let root_siblings = suggestion_ids(engine.complete("config", "config".len()).1);
    assert_eq!(
        root_siblings,
        vec!["config", "doctor", "exit", "help", "quit"]
    );

    let config_siblings = suggestion_ids(engine.complete("config show", "config show".len()).1);
    assert_eq!(config_siblings, vec!["explain", "get", "show"]);
}

#[test]
fn contract_space_commits_scope_for_the_next_tab_unit() {
    let engine =
        crate::completion::CompletionEngine::new(completion_tree_with_root_and_config_show());

    let root_siblings = suggestion_ids(engine.complete("config", "config".len()).1);
    assert_eq!(
        root_siblings,
        vec!["config", "doctor", "exit", "help", "quit"]
    );

    let committed_children = suggestion_ids(engine.complete("config ", "config ".len()).1);
    assert_eq!(committed_children, vec!["explain", "get", "show"]);

    let committed_flags = suggestion_ids(engine.complete("config show ", "config show ".len()).1);
    assert_eq!(committed_flags, vec!["--raw", "--sources"]);
}

#[test]
fn contract_used_flags_disappear_once_committed_but_uncommitted_flags_stay_replaceable_unit() {
    let engine = crate::completion::CompletionEngine::new(completion_tree_with_config_show());

    let uncommitted = suggestion_ids(
        engine
            .complete("config show --raw", "config show --raw".len())
            .1,
    );
    assert_eq!(uncommitted, vec!["--raw", "--sources"]);

    let committed = suggestion_ids(
        engine
            .complete("config show --raw ", "config show --raw ".len())
            .1,
    );
    assert_eq!(committed, vec!["--sources"]);
}

#[test]
fn contract_exact_argument_values_stay_in_sibling_scope_until_delimited_unit() {
    let engine = crate::completion::CompletionEngine::new(completion_tree_with_theme_show_values());

    let uncommitted = suggestion_ids(
        engine
            .complete("theme show catppuccin", "theme show catppuccin".len())
            .1,
    );
    assert_eq!(uncommitted, vec!["catppuccin", "dracula", "gruvbox"]);

    let committed = suggestion_ids(
        engine
            .complete("theme show catppuccin ", "theme show catppuccin ".len())
            .1,
    );
    assert!(committed.is_empty());
}

#[test]
fn contract_repl_completer_keeps_active_hidden_token_visible_until_a_space_commits_it_unit() {
    let tree = completion_tree_with_root_commands();
    let projector = Arc::new(|line: &str| {
        let hidden = if line.starts_with("help") {
            BTreeSet::from(["help".to_string()])
        } else {
            BTreeSet::default()
        };
        LineProjection::passthrough(line).with_hidden_suggestions(hidden)
    });
    let mut completer = ReplCompleter::new(Vec::new(), Some(tree), Some(projector));

    let root = completer
        .complete("", 0)
        .into_iter()
        .map(|item| item.value)
        .collect::<Vec<_>>();
    assert!(root.contains(&"help".to_string()));

    let exact_uncommitted = completer
        .complete("help", "help".len())
        .into_iter()
        .map(|item| item.value)
        .collect::<Vec<_>>();
    assert!(exact_uncommitted.contains(&"help".to_string()));

    let committed = completer
        .complete("help ", "help ".len())
        .into_iter()
        .map(|item| item.value)
        .collect::<Vec<_>>();
    assert!(!committed.contains(&"help".to_string()));
}

#[test]
fn process_submission_handles_restart_and_error_paths_unit() {
    let history = disabled_history();

    let mut restart_execute = |_line: &str, _: &SharedHistory| {
        Ok(ReplLineResult::Restart {
            output: "restarting".to_string(),
            reload: ReplReloadKind::WithIntro,
        })
    };
    let mut submission = SubmissionContext {
        history_store: &history,
        execute: &mut restart_execute,
    };
    let restart = process_submission("config set", &mut submission).expect("restart should map");
    assert!(matches!(
        restart,
        SubmissionResult::Restart {
            output,
            reload: ReplReloadKind::WithIntro
        } if output == "restarting"
    ));

    let mut failing_execute = |_line: &str, _: &SharedHistory| -> anyhow::Result<ReplLineResult> {
        Err(anyhow::anyhow!("submit failed"))
    };
    let mut failing_submission = SubmissionContext {
        history_store: &history,
        execute: &mut failing_execute,
    };
    let result =
        process_submission("broken", &mut failing_submission).expect("error should be absorbed");
    assert!(matches!(result, SubmissionResult::Noop));

    let mut noop_execute =
        |_line: &str, _: &SharedHistory| Ok(ReplLineResult::Continue("ignored".to_string()));
    let mut noop_submission = SubmissionContext {
        history_store: &history,
        execute: &mut noop_execute,
    };
    let result = process_submission("   ", &mut noop_submission).expect("blank lines should noop");
    assert!(matches!(result, SubmissionResult::Noop));
}

#[test]
fn highlighter_builder_requires_command_color_unit() {
    let tree = completion_tree_with_config_show();
    let none = build_repl_highlighter(&tree, &super::ReplAppearance::default(), None);
    assert!(none.is_none());

    let some = build_repl_highlighter(
        &tree,
        &ReplAppearance::builder()
            .with_command_highlight_style(Some("green".to_string()))
            .build(),
        None,
    );
    assert!(some.is_some());
}

#[test]
fn path_suggestions_distinguish_files_and_directories_unit() {
    let root = make_temp_dir("osp-repl-paths");
    std::fs::write(root.join("alpha.txt"), "x").expect("file should be written");
    std::fs::create_dir_all(root.join("alpine")).expect("dir should be created");
    let stub = format!("{}/al", root.display());

    let suggestions = path_suggestions(
        &stub,
        &stub,
        None,
        reedline::Span {
            start: 0,
            end: stub.len(),
        },
    );
    let values = suggestions
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
}

#[test]
fn path_suggestions_escape_spaces_when_unquoted_unit() {
    let root = make_temp_dir("osp-repl-paths-quoted");
    std::fs::write(root.join("team docs.txt"), "x").expect("file should be written");
    let stub = format!("{}/te", root.display());

    let suggestions = path_suggestions(
        &stub,
        &stub,
        None,
        reedline::Span {
            start: 0,
            end: stub.len(),
        },
    );

    assert!(
        suggestions
            .iter()
            .any(|item| item.value.ends_with("team\\ docs.txt"))
    );
}

#[test]
fn path_suggestions_preserve_open_double_quote_context_unit() {
    let root = make_temp_dir("osp-repl-paths-double");
    std::fs::write(root.join("team docs.txt"), "x").expect("file should be written");
    let token_stub = format!("{}/te", root.display());
    let raw_stub = format!("\"{token_stub}");

    let suggestions = path_suggestions(
        &raw_stub,
        &token_stub,
        Some(QuoteStyle::Double),
        reedline::Span {
            start: 0,
            end: raw_stub.len(),
        },
    );

    assert!(
        suggestions
            .iter()
            .any(|item| item.value.ends_with("team docs.txt\""))
    );
}

#[test]
fn trace_completion_env_controls_and_jsonl_output_unit() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
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
fn cursor_position_errors_are_recognized_unit() {
    assert!(is_cursor_position_error(&io::Error::from_raw_os_error(25)));
    assert!(is_cursor_position_error(&io::Error::other(
        "Cursor position could not be read"
    )));
    assert!(!is_cursor_position_error(&io::Error::other(
        "permission denied"
    )));
}

#[test]
fn cursor_position_report_parser_distinguishes_valid_and_invalid_sequences_unit() {
    assert_eq!(parse_cursor_position_report(b"\x1b[12;34R"), Some((34, 12)));
    assert_eq!(
        parse_cursor_position_report(b"\x1b[1;200R trailing"),
        Some((200, 1))
    );
    assert!(contains_cursor_position_report(b"noise\x1b[22;7R"));
    assert_eq!(parse_cursor_position_report(b"\x1b[;34R"), None);
    assert_eq!(parse_cursor_position_report(b"\x1b[12;R"), None);
    assert_eq!(parse_cursor_position_report(b"\x1b[12;34"), None);
    assert!(!contains_cursor_position_report(b"\x1b[bad"));
}

#[test]
fn explicit_basic_input_mode_short_circuits_unit() {
    assert_eq!(
        basic_input_reason(ReplInputMode::Basic),
        Some(BasicInputReason::Explicit)
    );
}

#[test]
fn run_repl_with_reason_routes_basic_reasons_to_basic_handler_unit() {
    for reason in [
        BasicInputReason::Explicit,
        BasicInputReason::NotATerminal,
        BasicInputReason::CursorProbeUnsupported,
    ] {
        let history =
            SharedHistory::new(history_config().build()).expect("history config should build");
        let prompt = OspPrompt::new("left".to_string(), "> ".to_string(), None);
        let mut execute =
            |_line: &str, _history: &SharedHistory| Ok(ReplLineResult::Continue(String::new()));
        let mut submission = SubmissionContext {
            history_store: &history,
            execute: &mut execute,
        };
        let mut basic_calls = 0usize;
        let mut interactive_calls = 0usize;

        let result = super::run_repl_with_reason(
            super::ReplRunContext {
                prompt,
                completion_words: vec!["help".to_string()],
                completion_tree: Some(completion_tree_with_config_show()),
                appearance: test_appearance(),
                line_projector: None,
                history_store: history.clone(),
            },
            Some(reason),
            &mut submission,
            |prompt, _submission| {
                basic_calls += 1;
                assert_eq!(prompt.left(), "left");
                Ok(())
            },
            |_config, _history, _submission| {
                interactive_calls += 1;
                Ok(ReplRunResult::Exit(9))
            },
        )
        .expect("basic path should succeed");

        assert_eq!(result, ReplRunResult::Exit(0));
        assert_eq!(basic_calls, 1);
        assert_eq!(interactive_calls, 0);
    }
}

#[test]
fn run_repl_with_reason_routes_none_to_interactive_handler_unit() {
    let history =
        SharedHistory::new(history_config().build()).expect("history config should build");
    let prompt = OspPrompt::new("left".to_string(), "> ".to_string(), None);
    let tree = completion_tree_with_config_show();
    let mut execute =
        |_line: &str, _history: &SharedHistory| Ok(ReplLineResult::Continue(String::new()));
    let mut submission = SubmissionContext {
        history_store: &history,
        execute: &mut execute,
    };
    let mut basic_calls = 0usize;
    let mut interactive_calls = 0usize;

    let result = super::run_repl_with_reason(
        super::ReplRunContext {
            prompt,
            completion_words: vec!["help".to_string(), "exit".to_string()],
            completion_tree: Some(tree),
            appearance: test_appearance(),
            line_projector: None,
            history_store: history.clone(),
        },
        None,
        &mut submission,
        |_prompt, _submission| {
            basic_calls += 1;
            Ok(())
        },
        |config, interactive_history, _submission| {
            interactive_calls += 1;
            assert_eq!(config.prompt.left(), "left");
            assert_eq!(config.completion_words, vec!["help", "exit"]);
            assert!(config.completion_tree.is_some());
            assert_eq!(config.appearance.history_menu_rows, 5);
            assert_eq!(interactive_history.enabled(), history.enabled());
            assert_eq!(
                interactive_history.recent_commands(),
                history.recent_commands()
            );
            Ok(ReplRunResult::Exit(7))
        },
    )
    .expect("interactive path should succeed");

    assert_eq!(result, ReplRunResult::Exit(7));
    assert_eq!(basic_calls, 0);
    assert_eq!(interactive_calls, 1);
}

#[test]
fn expand_home_and_prompt_renderers_behave_unit() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let previous_home = std::env::var("HOME").ok();
    set_env_var_for_test("HOME", "/tmp/osp-home");
    assert_eq!(expand_home("~"), "/tmp/osp-home");
    assert_eq!(expand_home("~/cache"), "/tmp/osp-home/cache");
    assert_eq!(expand_home("/etc/hosts"), "/etc/hosts");

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

// History menu and history-debug contracts.

#[test]
fn history_completer_returns_latest_unique_entries_for_empty_query_unit() {
    let history = test_history(&[
        "ldap user alice",
        "mreg host foo",
        "ldap user alice",
        "config get theme",
        "help ldap",
        "doctor",
    ]);
    let mut completer = ReplHistoryCompleter::new(history);

    let suggestions = completer.complete("", 0);
    let values = suggestions
        .iter()
        .map(|suggestion| suggestion.value.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        values,
        vec![
            "doctor",
            "help ldap",
            "config get theme",
            "ldap user alice",
            "mreg host foo",
        ]
    );
    assert!(
        suggestions
            .iter()
            .all(|suggestion| suggestion.span == (Span { start: 0, end: 0 }))
    );
    assert_eq!(
        suggestions[0]
            .extra
            .as_ref()
            .and_then(|extra| extra.first())
            .cloned(),
        Some("6  doctor".to_string())
    );
}

#[test]
fn history_completer_ranks_exact_prefix_then_substring_unit() {
    let history = test_history(&[
        "config list",
        "ldap config",
        "config get theme",
        "show config",
        "CONFIG",
        "config",
    ]);
    let mut completer = ReplHistoryCompleter::new(history);

    let suggestions = completer.complete("config", "config".len());
    let values = suggestions
        .iter()
        .map(|suggestion| suggestion.value.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        values,
        vec![
            "config",
            "CONFIG",
            "config get theme",
            "config list",
            "show config",
        ]
    );
    assert!(suggestions.iter().all(|suggestion| {
        suggestion.span
            == (Span {
                start: 0,
                end: "config".len(),
            })
    }));
    assert_eq!(
        suggestions[0]
            .extra
            .as_ref()
            .and_then(|extra| extra.first())
            .cloned(),
        Some("6  config".to_string())
    );
}

#[test]
fn debug_history_menu_surfaces_numbered_labels_and_steps_unit() {
    let history = test_history(&["ldap user alice", "config get theme", "config"]);
    let menu = build_history_menu(&ReplAppearance::default());
    assert_eq!(menu.name(), HISTORY_MENU_NAME);
    assert!(!menu.can_quick_complete());

    let debug = debug_history_menu(
        &history,
        "config",
        "config".len(),
        CompletionDebugOptions::new(80, 6),
    );
    assert_eq!(debug.stub, "config");
    assert_eq!(debug.matches[0].label, "3  config");
    assert_eq!(debug.matches[0].id, "config");
    assert!(debug.matches.iter().all(|item| item.kind == "history"));

    let frames = debug_history_menu_steps(
        &history,
        "config",
        "config".len(),
        CompletionDebugOptions::new(80, 6),
        &[DebugStep::Tab, DebugStep::Accept],
    );
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].state.matches[0].label, "3  config");
    assert_eq!(frames[1].state.line, "config");
}

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

    let options = build_history_picker_options(&appearance, "doctor mreg");

    assert_eq!(options.height, "8");
    assert_eq!(options.query.as_deref(), Some("doctor mreg"));
    assert_eq!(options.prompt, "(reverse-i-search)> ");
    assert!(options.no_info);
    assert_eq!(
        options.color.as_deref(),
        Some(
            "normal:7,matched:7,current:7,current_match:7,query:7,prompt:7,cursor:7,selected:7,info:7,header:7,spinner:7,border:7,bg:0,matched_bg:0,current_bg:6,current_match_bg:6"
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
    // Test-only environment mutation is process-global on Rust 2024.
    // Keep the unsafe boundary explicit and local to these regression
    // tests instead of spreading raw calls through the module.
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
    let history = SharedHistory::new(
        history_config()
            .with_max_entries(32)
            .with_shell_context(HistoryShellContext::default())
            .build(),
    )
    .expect("history should initialize");

    for command in commands {
        history
            .save_command_line(command)
            .expect("history save should succeed");
    }

    history
}
