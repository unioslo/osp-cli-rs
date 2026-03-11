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
    CompletionTreeBuilder.build_from_specs(
        &[
            CommandSpec::new("orch").subcommand(CommandSpec::new("provision").flag(
                "--os",
                FlagNode {
                    suggestions_by_provider: BTreeMap::from([
                        ("vmware".to_string(), vec![SuggestionEntry::from("rhel")]),
                        ("nrec".to_string(), vec![SuggestionEntry::from("alma")]),
                    ]),
                    suggestions: vec![SuggestionEntry::from("rhel"), SuggestionEntry::from("alma")],
                    ..FlagNode::default()
                },
            )),
            CommandSpec::new("hidden").flag(
                "--provider",
                FlagNode {
                    suggestions: vec![
                        SuggestionEntry::from("vmware"),
                        SuggestionEntry::from("nrec"),
                    ],
                    context_only: true,
                    context_scope,
                    ..FlagNode::default()
                },
            ),
        ],
        [],
    )
}

#[test]
fn completion_engine_merges_global_context_flags_from_later_tokens() {
    let engine = CompletionEngine::new(completion_tree(ContextScope::Global));
    let line = "orch provision --os  --provider vmware";
    let cursor = provider_cursor(line);

    let (_, suggestions) = engine.complete(line, cursor);
    let values = suggestion_values(suggestions);
    assert!(values.contains(&"rhel".to_string()));
    assert!(!values.contains(&"alma".to_string()));

    let analysis = engine.analyze(line, cursor);
    assert_eq!(analysis.context.matched_path, vec!["orch", "provision"]);
    assert_eq!(analysis.context.flag_scope_path, vec!["orch", "provision"]);
    assert_eq!(
        analysis
            .parsed
            .cursor_cmd
            .flag_values("--provider")
            .expect("provider should merge into cursor context"),
        &vec!["vmware".to_string()][..]
    );
}

#[test]
fn completion_engine_keeps_subtree_context_flags_outside_matched_scope() {
    let engine = CompletionEngine::new(completion_tree(ContextScope::Subtree));
    let line = "orch provision --os  --provider vmware";

    let (_, suggestions) = engine.complete(line, provider_cursor(line));
    let values = suggestion_values(suggestions);
    assert!(values.contains(&"rhel".to_string()));
    assert!(values.contains(&"alma".to_string()));
}
