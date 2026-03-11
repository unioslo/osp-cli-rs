#[test]
fn completion_words_collect_flags_and_backbone_commands_unit() {
    let spec = crate::completion::CommandSpec::new("ldap")
        .flag("--json", crate::completion::FlagNode::new())
        .subcommand(
            crate::completion::CommandSpec::new("user")
                .subcommand(crate::completion::CommandSpec::new("show")),
        );

    let words = collect_completion_words(&spec);
    assert!(words.contains(&"ldap".to_string()));
    assert!(words.contains(&"--json".to_string()));
    assert!(words.contains(&"user".to_string()));
    assert!(words.contains(&"show".to_string()));

    let manager = PluginManager::new(Vec::new());
    assert_eq!(
        manager
            .completion_words()
            .expect("backbone completion words should render"),
        vec![
            "F".to_string(),
            "P".to_string(),
            "V".to_string(),
            "exit".to_string(),
            "help".to_string(),
            "quit".to_string(),
            "|".to_string(),
        ]
    );
    assert!(
        manager
            .repl_help_text()
            .expect("empty help should render")
            .contains("No plugin commands available.")
    );
}

#[test]
fn describe_command_helpers_preserve_nested_completion_metadata_unit() {
    let suggestion = DescribeSuggestionV1 {
        value: "json".to_string(),
        meta: Some("format".to_string()),
        display: Some("JSON".to_string()),
        sort: Some("01".to_string()),
    };
    let command = DescribeCommandV1 {
        name: "ldap".to_string(),
        about: "lookup users".to_string(),
        auth: None,
        args: vec![DescribeArgV1 {
            name: Some("uid".to_string()),
            about: Some("user id".to_string()),
            multi: true,
            value_type: Some(crate::core::plugin::DescribeValueTypeV1::Path),
            suggestions: vec![suggestion.clone()],
        }],
        flags: std::collections::BTreeMap::from([(
            "--format".to_string(),
            DescribeFlagV1 {
                about: Some("output format".to_string()),
                flag_only: false,
                multi: true,
                value_type: Some(crate::core::plugin::DescribeValueTypeV1::Path),
                suggestions: vec![suggestion.clone()],
            },
        )]),
        subcommands: vec![DescribeCommandV1 {
            name: "user".to_string(),
            about: String::new(),
            auth: None,
            args: Vec::new(),
            flags: Default::default(),
            subcommands: Vec::new(),
        }],
    };

    let spec = to_command_spec(&command);
    assert_eq!(spec.name, "ldap");
    assert_eq!(spec.tooltip.as_deref(), Some("lookup users"));
    assert_eq!(direct_subcommand_names(&spec), vec!["user".to_string()]);
    assert!(collect_completion_words(&spec).contains(&"--format".to_string()));

    let arg = to_arg_node(&command.args[0]);
    assert_eq!(arg.name.as_deref(), Some("uid"));
    assert_eq!(arg.tooltip.as_deref(), Some("user id"));
    assert!(arg.multi);
    assert_eq!(arg.value_type, Some(crate::completion::ValueType::Path));

    let flag = to_flag_node(command.flags.get("--format").expect("flag"));
    assert_eq!(flag.tooltip.as_deref(), Some("output format"));
    assert!(flag.multi);
    assert_eq!(flag.value_type, Some(crate::completion::ValueType::Path));

    let entry = to_suggestion_entry(&suggestion);
    assert_eq!(entry.value, "json");
    assert_eq!(entry.display.as_deref(), Some("JSON"));
    assert_eq!(
        to_value_type(crate::core::plugin::DescribeValueTypeV1::Path),
        Some(crate::completion::ValueType::Path)
    );
}
