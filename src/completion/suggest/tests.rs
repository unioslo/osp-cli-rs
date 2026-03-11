use std::collections::BTreeMap;

use crate::completion::CompletionEngine;
use crate::completion::model::{
    ArgNode, CommandLine, CompletionNode, CompletionTree, CursorState, FlagHints, FlagNode,
    FlagOccurrence, SuggestionEntry, SuggestionOutput, ValueType,
};

fn tree() -> CompletionTree {
    let mut provision = CompletionNode::default();
    provision.flags.insert(
        "--provider".to_string(),
        FlagNode {
            suggestions: vec![
                SuggestionEntry::from("nrec"),
                SuggestionEntry::from("vmware"),
            ],
            os_provider_map: BTreeMap::from([
                ("alma".to_string(), vec!["nrec".to_string()]),
                ("rhel".to_string(), vec!["vmware".to_string()]),
            ]),
            ..FlagNode::default()
        },
    );
    provision.flags.insert(
        "--os".to_string(),
        FlagNode {
            suggestions: vec![SuggestionEntry::from("alma"), SuggestionEntry::from("rhel")],
            ..FlagNode::default()
        },
    );

    let mut orch = CompletionNode::default();
    orch.children.insert("provision".to_string(), provision);

    CompletionTree {
        root: CompletionNode::default().with_child("orch", orch),
        pipe_verbs: BTreeMap::from([("F".to_string(), "Filter".to_string())]),
    }
}

fn values(output: Vec<SuggestionOutput>) -> Vec<String> {
    output
        .into_iter()
        .filter_map(|entry| match entry {
            SuggestionOutput::Item(item) => Some(item.text),
            SuggestionOutput::PathSentinel => None,
        })
        .collect()
}

fn generate(engine: &CompletionEngine, cmd: CommandLine, stub: &str) -> Vec<SuggestionOutput> {
    let analysis = engine.analyze_command(cmd.clone(), cmd, CursorState::synthetic(stub));
    engine.suggestions_for_analysis(&analysis)
}

fn values_for_line(engine: &CompletionEngine, line: &str) -> Vec<String> {
    let (_, output) = engine.complete(line, line.len());
    values(output)
}

fn command(head: &[&str]) -> CommandLine {
    let mut cmd = CommandLine::default();
    for segment in head {
        cmd.push_head(*segment);
    }
    cmd
}

fn with_flag(mut cmd: CommandLine, name: &str, values: &[&str]) -> CommandLine {
    cmd.push_flag_occurrence(FlagOccurrence {
        name: name.to_string(),
        values: values.iter().map(|value| (*value).to_string()).collect(),
    });
    cmd
}

fn tree_with_command(name: &str, node: CompletionNode) -> CompletionTree {
    CompletionTree {
        root: CompletionNode::default().with_child(name, node),
        ..CompletionTree::default()
    }
}

fn tags_and_mode_tree() -> CompletionTree {
    let mut cmd_node = CompletionNode::default();
    cmd_node.flags.insert(
        "--tags".to_string(),
        FlagNode {
            multi: true,
            suggestions: vec![
                SuggestionEntry::from("red"),
                SuggestionEntry::from("green"),
                SuggestionEntry::from("blue"),
            ],
            ..FlagNode::default()
        },
    );
    cmd_node.flags.insert(
        "--mode".to_string(),
        FlagNode {
            suggestions: vec![SuggestionEntry::from("fast"), SuggestionEntry::from("full")],
            ..FlagNode::default()
        },
    );
    tree_with_command("tag", cmd_node)
}

#[test]
fn command_and_pipe_suggestions_cover_scope_fuzzy_values_and_filters_unit() {
    let engine = CompletionEngine::new(tree());
    let cmd = command(&["orch", "provision"]);

    let option_values = values(generate(&engine, cmd.clone(), "--"));
    assert!(option_values.contains(&"--provider".to_string()));
    assert!(option_values.contains(&"--os".to_string()));

    let fuzzy_values = values(generate(&engine, cmd.clone(), "--prv"));
    assert!(fuzzy_values.contains(&"--provider".to_string()));

    let provider_values = values(generate(
        &engine,
        with_flag(cmd.clone(), "--provider", &[]),
        "",
    ));
    assert!(provider_values.contains(&"nrec".to_string()));
    assert!(provider_values.contains(&"vmware".to_string()));

    let cmd = with_flag(with_flag(cmd, "--os", &["alma"]), "--provider", &[]);
    let filtered_values = values(generate(&engine, cmd, ""));
    assert!(filtered_values.contains(&"nrec".to_string()));
    assert!(!filtered_values.contains(&"vmware".to_string()));

    let mut cmd = CommandLine::default();
    cmd.set_pipe(Vec::new());
    let output = generate(&engine, cmd, "F");
    assert!(
        output
            .iter()
            .any(|entry| matches!(entry, SuggestionOutput::Item(item) if item.text == "F"))
    );

    let mut fuzzy_tree = tree();
    fuzzy_tree
        .pipe_verbs
        .insert("VALUE".to_string(), "Extract values".to_string());
    fuzzy_tree
        .pipe_verbs
        .insert("VAL".to_string(), "Extract".to_string());
    let engine = CompletionEngine::new(fuzzy_tree);
    let mut cmd = CommandLine::default();
    cmd.set_pipe(Vec::new());

    let output = generate(&engine, cmd, "vlu");
    assert!(
        output
            .iter()
            .any(|entry| matches!(entry, SuggestionOutput::Item(item) if item.text == "VALUE"))
    );
    let completion_values = values(output);
    assert_eq!(completion_values.first().map(String::as_str), Some("VALUE"));
}

#[test]
fn flag_value_modes_cover_single_multi_repeatable_and_repeated_flags_unit() {
    let mut cmd_node = CompletionNode::default();
    cmd_node.flags.insert(
        "--context".to_string(),
        FlagNode {
            suggestions: vec![
                SuggestionEntry::from("uio"),
                SuggestionEntry::from("tsd"),
                SuggestionEntry::from("edu"),
            ],
            ..FlagNode::default()
        },
    );
    cmd_node.flags.insert(
        "--terminal".to_string(),
        FlagNode {
            suggestions: vec![SuggestionEntry::from("cli"), SuggestionEntry::from("repl")],
            ..FlagNode::default()
        },
    );

    let engine = CompletionEngine::new(tree_with_command("alias", cmd_node));
    let cmd = with_flag(command(&["alias"]), "--context", &["uio"]);
    let context_values = values(generate(&engine, cmd, ""));
    assert!(!context_values.contains(&"uio".to_string()));
    assert!(context_values.contains(&"--terminal".to_string()));

    let engine = CompletionEngine::new(tags_and_mode_tree());
    let cmd = with_flag(command(&["tag"]), "--tags", &["red"]);
    let values_for_space = values(generate(&engine, cmd.clone(), ""));
    assert!(values_for_space.contains(&"red".to_string()));
    assert!(!values_for_space.contains(&"--mode".to_string()));

    let values_for_dash = values(generate(&engine, cmd, "-"));
    assert!(values_for_dash.contains(&"--mode".to_string()));
    let repeated_tag_values = values_for_line(&engine, "tag --tags red --mode fast --tags ");
    assert!(repeated_tag_values.contains(&"red".to_string()));
    assert!(repeated_tag_values.contains(&"green".to_string()));
    assert!(!repeated_tag_values.contains(&"--mode".to_string()));

    let fuzzy_tag_values = values_for_line(&engine, "tag --tags red --tags bl");
    assert!(fuzzy_tag_values.contains(&"blue".to_string()));

    let cmd = with_flag(command(&["tag"]), "--tags", &["red"]);
    let dash_values = values(generate(&engine, cmd, "--"));
    assert!(dash_values.contains(&"--tags".to_string()));
    assert!(dash_values.contains(&"--mode".to_string()));
}

#[test]
fn args_after_double_dash_advance_index() {
    let cmd_node = CompletionNode {
        args: vec![
            ArgNode {
                suggestions: vec![SuggestionEntry::from("one")],
                ..ArgNode::default()
            },
            ArgNode {
                suggestions: vec![SuggestionEntry::from("two"), SuggestionEntry::from("three")],
                ..ArgNode::default()
            },
        ],
        ..CompletionNode::default()
    };
    let tree = CompletionTree {
        root: CompletionNode::default().with_child("cmd", cmd_node),
        ..CompletionTree::default()
    };
    let engine = CompletionEngine::new(tree);
    let mut cmd = command(&["cmd"]);
    cmd.push_positional("one");

    let values = values(generate(&engine, cmd, ""));
    assert!(values.contains(&"two".to_string()));
    assert!(values.contains(&"three".to_string()));
    assert!(!values.contains(&"one".to_string()));
}

#[test]
fn path_value_modes_emit_path_sentinels_for_args_and_flags_unit() {
    let cmd_node = CompletionNode {
        args: vec![ArgNode {
            value_type: Some(ValueType::Path),
            ..ArgNode::default()
        }],
        ..CompletionNode::default()
    };
    let engine = CompletionEngine::new(tree_with_command("cmd", cmd_node));
    let cmd = command(&["cmd"]);

    let output = generate(&engine, cmd, "");
    assert!(
        output
            .iter()
            .any(|entry| matches!(entry, SuggestionOutput::PathSentinel))
    );

    let mut node = CompletionNode::default();
    node.flags.insert(
        "--file".to_string(),
        FlagNode {
            value_type: Some(ValueType::Path),
            ..FlagNode::default()
        },
    );
    let engine = CompletionEngine::new(tree_with_command("cmd", node));
    let cmd = with_flag(command(&["cmd"]), "--file", &[]);

    let output = generate(&engine, cmd, "");
    assert!(
        output
            .iter()
            .any(|entry| matches!(entry, SuggestionOutput::PathSentinel))
    );
}

#[test]
fn flag_hints_filter_provider_specific_flags_and_alias_allowlists_unit() {
    let mut node = CompletionNode::default();
    node.flags
        .insert("--provider".to_string(), FlagNode::default());
    node.flags.insert(
        "--nrec".to_string(),
        FlagNode {
            flag_only: true,
            ..FlagNode::default()
        },
    );
    node.flags.insert(
        "--vmware".to_string(),
        FlagNode {
            flag_only: true,
            ..FlagNode::default()
        },
    );
    node.flags
        .insert("--comment".to_string(), FlagNode::default());
    node.flags
        .insert("--flavor".to_string(), FlagNode::default());
    node.flags
        .insert("--vcenter".to_string(), FlagNode::default());
    node.flag_hints = Some(FlagHints {
        common: vec![
            "--provider".to_string(),
            "--nrec".to_string(),
            "--vmware".to_string(),
            "--comment".to_string(),
        ],
        by_provider: BTreeMap::from([
            ("nrec".to_string(), vec!["--flavor".to_string()]),
            ("vmware".to_string(), vec!["--vcenter".to_string()]),
        ]),
        required_common: vec!["--comment".to_string()],
        required_by_provider: BTreeMap::from([("nrec".to_string(), vec!["--flavor".to_string()])]),
    });

    let tree = CompletionTree {
        root: CompletionNode::default().with_child("provision", node),
        ..CompletionTree::default()
    };
    let engine = CompletionEngine::new(tree);

    let cmd = with_flag(command(&["provision"]), "--provider", &["nrec"]);
    let output = generate(&engine, cmd, "--");
    let rendered_values = values(output.clone());
    assert!(rendered_values.contains(&"--comment".to_string()));
    assert!(rendered_values.contains(&"--flavor".to_string()));
    assert!(!rendered_values.contains(&"--provider".to_string()));
    assert!(!rendered_values.contains(&"--nrec".to_string()));
    assert!(!rendered_values.contains(&"--vmware".to_string()));
    assert!(!rendered_values.contains(&"--vcenter".to_string()));

    let items = output
        .into_iter()
        .filter_map(|entry| match entry {
            SuggestionOutput::Item(item) => Some(item),
            SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();
    let by_text = items
        .into_iter()
        .map(|item| (item.text.clone(), item))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        by_text
            .get("--comment")
            .and_then(|item| item.display.as_deref()),
        Some("--comment*")
    );
    assert_eq!(
        by_text
            .get("--flavor")
            .and_then(|item| item.display.as_deref()),
        Some("--flavor*")
    );

    let mut alias_node = CompletionNode::default();
    alias_node
        .flags
        .insert("--provider".to_string(), FlagNode::default());
    alias_node.flags.insert(
        "--nrec".to_string(),
        FlagNode {
            flag_only: true,
            ..FlagNode::default()
        },
    );
    alias_node
        .flags
        .insert("--flavor".to_string(), FlagNode::default());
    alias_node.flag_hints = Some(FlagHints {
        common: vec!["--provider".to_string(), "--nrec".to_string()],
        by_provider: BTreeMap::from([("nrec".to_string(), vec!["--flavor".to_string()])]),
        ..FlagHints::default()
    });

    let alias_tree = CompletionTree {
        root: CompletionNode::default().with_child("provision", alias_node),
        ..CompletionTree::default()
    };
    let alias_engine = CompletionEngine::new(alias_tree);
    let alias_cmd = with_flag(command(&["provision"]), "--nrec", &[]);
    let alias_values = values(generate(&alias_engine, alias_cmd, "--"));
    assert!(alias_values.contains(&"--flavor".to_string()));
    assert!(!alias_values.contains(&"--provider".to_string()));
}

#[test]
fn flag_suggestions_preserve_meta_and_display_fields() {
    let mut node = CompletionNode::default();
    node.flags.insert(
        "--flavor".to_string(),
        FlagNode {
            suggestions: vec![
                SuggestionEntry {
                    value: "m1.small".to_string(),
                    meta: Some("1 vCPU".to_string()),
                    display: Some("small".to_string()),
                    sort: Some("10".to_string()),
                },
                SuggestionEntry::from("m1.medium"),
            ],
            ..FlagNode::default()
        },
    );
    let tree = CompletionTree {
        root: CompletionNode::default().with_child("orch", node),
        ..CompletionTree::default()
    };
    let engine = CompletionEngine::new(tree);
    let cmd = with_flag(command(&["orch"]), "--flavor", &[]);

    let output = generate(&engine, cmd, "");
    let items = output
        .into_iter()
        .filter_map(|entry| match entry {
            SuggestionOutput::Item(item) => Some(item),
            SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    let rich = items
        .iter()
        .find(|item| item.text == "m1.small")
        .expect("m1.small suggestion should exist");
    assert_eq!(rich.meta.as_deref(), Some("1 vCPU"));
    assert_eq!(rich.display.as_deref(), Some("small"));
    assert_eq!(rich.sort.as_deref(), Some("10"));
}

#[test]
fn arg_suggestions_honor_numeric_sort_after_match_score() {
    let cmd_node = CompletionNode {
        args: vec![ArgNode {
            suggestions: vec![
                SuggestionEntry {
                    value: "v10".to_string(),
                    meta: None,
                    display: None,
                    sort: Some("10".to_string()),
                },
                SuggestionEntry {
                    value: "v2".to_string(),
                    meta: None,
                    display: None,
                    sort: Some("2".to_string()),
                },
            ],
            ..ArgNode::default()
        }],
        ..CompletionNode::default()
    };
    let tree = CompletionTree {
        root: CompletionNode::default().with_child("cmd", cmd_node),
        ..CompletionTree::default()
    };
    let engine = CompletionEngine::new(tree);
    let cmd = command(&["cmd"]);

    let values = values(generate(&engine, cmd, ""));
    assert_eq!(values, vec!["v2".to_string(), "v10".to_string()]);
}

#[test]
fn subcommand_suggestions_honor_child_sort_after_match_score() {
    let tree = CompletionTree {
        root: CompletionNode::default()
            .with_child("orch", CompletionNode::default().sort("20"))
            .with_child("config", CompletionNode::default().sort("10")),
        ..CompletionTree::default()
    };
    let engine = CompletionEngine::new(tree);

    let output = values(generate(&engine, CommandLine::default(), ""));

    assert_eq!(output[..2], ["config", "orch"]);
}

#[test]
fn suggestions_match_unicode_case_insensitively() {
    let cmd_node = CompletionNode {
        args: vec![ArgNode {
            suggestions: vec![
                SuggestionEntry::from("Ålesund"),
                SuggestionEntry::from("Oslo"),
            ],
            ..ArgNode::default()
        }],
        ..CompletionNode::default()
    };
    let tree = CompletionTree {
        root: CompletionNode::default().with_child("city", cmd_node),
        ..CompletionTree::default()
    };
    let engine = CompletionEngine::new(tree);
    let cmd = command(&["city"]);

    let values = values(generate(&engine, cmd, "å"));
    assert_eq!(values, vec!["Ålesund".to_string()]);
}
