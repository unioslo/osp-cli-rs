use std::collections::BTreeMap;

use super::{CommandLine, FlagOccurrence, SuggestionEntry, TailItem};

#[test]
fn command_line_and_node_data_structs_preserve_internal_state_unit() {
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

    let mut flag = super::FlagNode::default();
    flag.suggestions_by_provider
        .insert("nrec".to_string(), vec![SuggestionEntry::from("alma")]);
    flag.os_provider_map
        .insert("alma".to_string(), vec!["nrec".to_string()]);

    let mut node = super::CompletionNode::default();
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
