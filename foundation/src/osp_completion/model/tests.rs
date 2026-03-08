use std::collections::BTreeMap;

use super::{
    ArgNode, CommandLine, CompletionNode, ContextScope, CursorState, FlagNode, FlagOccurrence,
    MatchKind, QuoteStyle, Suggestion, SuggestionEntry, TailItem, ValueType,
};

#[test]
fn cursor_state_constructors_preserve_stub_and_range() {
    let explicit = CursorState::new("tok", "\"tok", 3..7, Some(QuoteStyle::Double));
    assert_eq!(explicit.token_stub, "tok");
    assert_eq!(explicit.raw_stub, "\"tok");
    assert_eq!(explicit.replace_range, 3..7);
    assert_eq!(explicit.quote_style, Some(QuoteStyle::Double));

    let synthetic = CursorState::synthetic("abc");
    assert_eq!(synthetic.token_stub, "abc");
    assert_eq!(synthetic.raw_stub, "abc");
    assert_eq!(synthetic.replace_range, 0..3);
    assert_eq!(CursorState::default(), CursorState::synthetic(""));
}

#[test]
fn suggestion_and_arg_builders_capture_metadata() {
    let entry = SuggestionEntry::value("alma")
        .meta("linux")
        .display("AlmaLinux")
        .sort("02");
    let arg = ArgNode::named("image")
        .tooltip("Image name")
        .multi()
        .value_type(ValueType::Path)
        .suggestions([entry.clone()]);

    assert_eq!(entry.meta.as_deref(), Some("linux"));
    assert_eq!(entry.display.as_deref(), Some("AlmaLinux"));
    assert_eq!(entry.sort.as_deref(), Some("02"));
    assert_eq!(arg.name.as_deref(), Some("image"));
    assert_eq!(arg.tooltip.as_deref(), Some("Image name"));
    assert!(arg.multi);
    assert_eq!(arg.value_type, Some(ValueType::Path));
    assert_eq!(arg.suggestions, vec![entry]);
}

#[test]
fn flag_and_completion_node_builders_attach_children_and_flags() {
    let flag = FlagNode::new()
        .tooltip("Provider")
        .flag_only()
        .multi()
        .context_only(ContextScope::Global)
        .value_type(ValueType::Path)
        .suggestions([SuggestionEntry::from("vmware")]);

    assert_eq!(flag.tooltip.as_deref(), Some("Provider"));
    assert!(flag.flag_only);
    assert!(flag.multi);
    assert!(flag.context_only);
    assert_eq!(flag.context_scope, ContextScope::Global);
    assert_eq!(flag.value_type, Some(ValueType::Path));
    assert_eq!(flag.suggestions.len(), 1);

    let node = CompletionNode::default()
        .sort("01")
        .with_child("status", CompletionNode::default())
        .with_flag("--provider", flag.clone());
    assert_eq!(node.sort.as_deref(), Some("01"));
    assert!(node.children.contains_key("status"));
    assert_eq!(node.flags.get("--provider"), Some(&flag));
}

#[test]
fn command_line_tracks_head_tail_flags_and_pipes() {
    let mut command = CommandLine::default();
    command.push_head("orch");
    command.push_head("provision");
    command.push_flag_occurrence(FlagOccurrence {
        name: "--provider".to_string(),
        values: vec!["nrec".to_string()],
    });
    command.push_positional("guest01");
    command.merge_flag_values("--os", vec!["alma".to_string()]);
    command.prepend_positional_values(["".to_string(), "tenant-a".to_string(), " ".to_string()]);
    command.set_pipe(vec!["V ip".to_string()]);

    assert_eq!(
        command.head(),
        &["orch".to_string(), "provision".to_string()]
    );
    assert!(command.has_flag("--provider"));
    assert_eq!(
        command.flag_values("--provider"),
        Some(&["nrec".to_string()][..])
    );
    assert_eq!(command.flag_values_map()["--os"], vec!["alma".to_string()]);
    assert_eq!(
        command
            .last_flag_occurrence()
            .map(|flag| flag.name.as_str()),
        Some("--provider")
    );
    assert_eq!(
        command.positional_args().cloned().collect::<Vec<_>>(),
        vec!["tenant-a".to_string(), "guest01".to_string()]
    );
    assert_eq!(command.tail_len(), 3);
    assert!(matches!(command.tail()[0], TailItem::Positional(_)));
    assert_eq!(command.pipes(), &["V ip".to_string()]);
    assert!(command.has_pipe());
}

#[test]
fn match_kind_and_suggestion_defaults_are_stable() {
    let kinds = [
        MatchKind::Pipe,
        MatchKind::Flag,
        MatchKind::Command,
        MatchKind::Subcommand,
        MatchKind::Value,
    ];
    assert_eq!(
        kinds.iter().map(|kind| kind.as_str()).collect::<Vec<_>>(),
        vec!["pipe", "flag", "command", "subcommand", "value"]
    );

    let suggestion = Suggestion::new("hello");
    assert_eq!(suggestion.text, "hello");
    assert_eq!(suggestion.meta, None);
    assert_eq!(suggestion.display, None);
    assert!(!suggestion.is_exact);
    assert_eq!(suggestion.sort, None);
    assert_eq!(suggestion.match_score, u32::MAX);
}

#[test]
fn model_structs_keep_provider_maps_and_prefilled_values() {
    let mut flag = FlagNode::default();
    flag.suggestions_by_provider
        .insert("nrec".to_string(), vec![SuggestionEntry::from("alma")]);
    flag.os_provider_map
        .insert("alma".to_string(), vec!["nrec".to_string()]);

    let mut node = CompletionNode::default();
    node.prefilled_flags
        .insert("--provider".to_string(), vec!["nrec".to_string()]);
    node.prefilled_positionals = vec!["tenant-a".to_string()];
    node.flag_hints = Some(Default::default());

    assert_eq!(
        flag.suggestions_by_provider["nrec"],
        vec![SuggestionEntry::from("alma")]
    );
    assert_eq!(flag.os_provider_map["alma"], vec!["nrec".to_string()]);
    assert_eq!(
        node.prefilled_flags,
        BTreeMap::from([("--provider".to_string(), vec!["nrec".to_string()])])
    );
    assert_eq!(node.prefilled_positionals, vec!["tenant-a".to_string()]);
    assert!(node.flag_hints.is_some());
}
