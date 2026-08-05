use osp_cli::completion::{
    CommandSpec, CompletionEngine, CompletionTreeBuilder, ContextScope, FlagNode, SuggestionEntry,
    SuggestionOutput,
};
use std::collections::BTreeMap;

fn suggestion_values(outputs: Vec<SuggestionOutput>) -> Vec<String> {
    outputs
        .into_iter()
        .filter_map(|entry| match entry {
            SuggestionOutput::Item(item) => Some(item.text),
            SuggestionOutput::PathSentinel => None,
        })
        .collect()
}

fn provider_cursor(line: &str) -> usize {
    line.find("--provider").expect("provider flag in test line") - 1
}

fn completion_tree(context_scope: ContextScope) -> osp_cli::completion::CompletionTree {
    CompletionTreeBuilder
        .build_from_specs(
            &[
                CommandSpec::new("service").subcommand(CommandSpec::new("deploy").flag(
                    "--image",
                    FlagNode {
                        suggestions_by_provider: BTreeMap::from([
                            ("alpha".to_string(), vec![SuggestionEntry::from("red")]),
                            ("beta".to_string(), vec![SuggestionEntry::from("blue")]),
                        ]),
                        suggestions: vec![
                            SuggestionEntry::from("red"),
                            SuggestionEntry::from("blue"),
                        ],
                        ..FlagNode::default()
                    },
                )),
                CommandSpec::new("hidden").flag(
                    "--provider",
                    FlagNode {
                        suggestions: vec![
                            SuggestionEntry::from("alpha"),
                            SuggestionEntry::from("beta"),
                        ],
                        context_only: true,
                        context_scope,
                        ..FlagNode::default()
                    },
                ),
            ],
            [],
        )
        .expect("completion tree should build")
}

#[test]
fn completion_engine_merges_global_context_flags_from_later_tokens() {
    let engine = CompletionEngine::new(completion_tree(ContextScope::Global));
    let line = "service deploy --image  --provider alpha";
    let cursor = provider_cursor(line);

    let (_, suggestions) = engine.complete(line, cursor);
    let values = suggestion_values(suggestions);
    assert!(values.contains(&"red".to_string()));
    assert!(!values.contains(&"blue".to_string()));

    let analysis = engine.analyze(line, cursor);
    assert_eq!(analysis.context.matched_path, vec!["service", "deploy"]);
    assert_eq!(analysis.context.flag_scope_path, vec!["service", "deploy"]);
    assert_eq!(
        analysis
            .parsed
            .cursor_cmd
            .flag_values("--provider")
            .expect("provider should merge into cursor context"),
        &vec!["alpha".to_string()][..]
    );
}

#[test]
fn completion_engine_keeps_subtree_context_flags_outside_matched_scope() {
    let engine = CompletionEngine::new(completion_tree(ContextScope::Subtree));
    let line = "service deploy --image  --provider alpha";

    let (_, suggestions) = engine.complete(line, provider_cursor(line));
    let values = suggestion_values(suggestions);
    assert!(values.contains(&"red".to_string()));
    assert!(values.contains(&"blue".to_string()));
}
