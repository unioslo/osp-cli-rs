use std::collections::BTreeMap;

use osp_completion::{
    CompletionEngine, CompletionNode, CompletionTree, FlagNode, SuggestionEntry, SuggestionOutput,
};

fn tree() -> CompletionTree {
    let mut provision = CompletionNode::default();
    provision.flags.insert(
        "--provider".to_string(),
        FlagNode {
            suggestions: vec![
                SuggestionEntry::value("vmware"),
                SuggestionEntry::value("nrec"),
            ],
            context_only: true,
            ..FlagNode::default()
        },
    );
    provision.flags.insert(
        "--os".to_string(),
        FlagNode {
            suggestions_by_provider: BTreeMap::from([
                ("vmware".to_string(), vec![SuggestionEntry::value("rhel")]),
                ("nrec".to_string(), vec![SuggestionEntry::value("alma")]),
            ]),
            suggestions: vec![
                SuggestionEntry::value("rhel"),
                SuggestionEntry::value("alma"),
            ],
            context_only: true,
            ..FlagNode::default()
        },
    );

    let mut ldap = CompletionNode::default();
    ldap.children
        .insert("user".to_string(), CompletionNode::default());
    ldap.children
        .insert("host".to_string(), CompletionNode::default());

    let mut root = CompletionNode::default();
    root.children
        .insert("orch".to_string(), CompletionNode::default());
    root.children
        .get_mut("orch")
        .expect("orch node should exist")
        .children
        .insert("provision".to_string(), provision);
    root.children.insert("ldap".to_string(), ldap);

    CompletionTree {
        root,
        pipe_verbs: BTreeMap::from([
            ("F".to_string(), "Filter rows".to_string()),
            ("VALUE".to_string(), "Extract values".to_string()),
        ]),
    }
}

fn values(line: &str, cursor: usize) -> Vec<String> {
    let engine = CompletionEngine::new(tree());
    let (_, suggestions) = engine.suggestions_with_stub(line, cursor);
    suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            SuggestionOutput::Item(item) => Some(item.text),
            SuggestionOutput::PathSentinel => None,
        })
        .collect()
}

#[test]
fn fuzzy_root_command_completion() {
    let suggestions = values("lap", 3);
    assert!(suggestions.contains(&"ldap".to_string()));
}

#[test]
fn two_pass_merge_uses_late_provider_for_os_values() {
    let line = "orch provision --os  --provider vmware";
    let cursor = line.find("--provider").expect("provider should be present") - 1;
    let suggestions = values(line, cursor);
    assert!(suggestions.contains(&"rhel".to_string()));
    assert!(!suggestions.contains(&"alma".to_string()));
}

#[test]
fn later_flag_is_not_resuggested_before_cursor() {
    let line = "orch provision  --provider vmware";
    let cursor = line.find("--provider").expect("provider should be present") - 2;
    let suggestions = values(line, cursor);
    assert!(!suggestions.contains(&"--provider".to_string()));
}

#[test]
fn fuzzy_pipe_completion_for_long_verb() {
    let suggestions = values("ldap user | vlu", "ldap user | vlu".len());
    assert!(suggestions.contains(&"VALUE".to_string()));
}
