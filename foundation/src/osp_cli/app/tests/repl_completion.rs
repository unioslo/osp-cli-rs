use super::*;

#[test]
fn repl_help_alias_rewrites_to_command_help_unit() {
    let state = make_completion_state(None);
    let rewritten = repl::ReplParsedLine::parse("help ldap user", state.runtime.config.resolved())
        .expect("help alias should parse");
    assert_eq!(
        rewritten.dispatch_tokens,
        vec!["ldap".to_string(), "user".to_string(), "--help".to_string()]
    );
}

#[test]
fn repl_help_alias_preserves_existing_help_flag_unit() {
    let state = make_completion_state(None);
    let rewritten =
        repl::ReplParsedLine::parse("help ldap --help", state.runtime.config.resolved())
            .expect("help alias should parse");
    assert_eq!(
        rewritten.dispatch_tokens,
        vec!["ldap".to_string(), "--help".to_string()]
    );
}

#[test]
fn repl_help_alias_skips_bare_help_unit() {
    let state = make_completion_state(None);
    let parsed = repl::ReplParsedLine::parse("help", state.runtime.config.resolved())
        .expect("bare help should parse");
    assert_eq!(parsed.command_tokens, vec!["help".to_string()]);
    assert_eq!(parsed.dispatch_tokens, vec!["help".to_string()]);
}

#[test]
fn repl_ui_projection_supports_flag_prefixed_help_and_completion_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let projected = crate::osp_cli::repl::input::project_repl_ui_line(
        "--json help orch prov",
        state.runtime.config.resolved(),
    )
    .expect("projection should succeed");

    let (_, suggestions) = engine.complete(&projected.line, projected.line.len());
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        crate::osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
    )));
}

#[test]
fn repl_scoped_help_alias_uses_relative_target_completion_unit() {
    let mut state = make_completion_state(None);
    state.session.scope.enter("orch");
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let projected = crate::osp_cli::repl::input::project_repl_ui_line(
        "help provision --p",
        state.runtime.config.resolved(),
    )
    .expect("projection should succeed");

    let (_, suggestions) = engine.complete(&projected.line, projected.line.len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item)
                if !projected.hidden_suggestions.contains(&item.text) =>
            {
                Some(item.text)
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"--provider".to_string()));
}

#[test]
fn repl_shellable_commands_include_ldap_unit() {
    assert!(repl::is_repl_shellable_command("ldap"));
    assert!(repl::is_repl_shellable_command("LDAP"));
    assert!(!repl::is_repl_shellable_command("theme"));
}

#[test]
fn repl_shell_prefix_applies_once_unit() {
    let mut stack = crate::osp_cli::state::ReplScopeStack::default();
    stack.enter("ldap");
    let bare = repl::apply_repl_shell_prefix(&stack, &["user".to_string(), "oistes".to_string()]);
    assert_eq!(
        bare,
        vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
    );

    let already_prefixed = repl::apply_repl_shell_prefix(
        &stack,
        &["ldap".to_string(), "user".to_string(), "oistes".to_string()],
    );
    assert_eq!(
        already_prefixed,
        vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
    );
}

#[test]
fn repl_shell_leave_message_unit() {
    let mut state = make_completion_state(None);
    state.session.scope.enter("ldap");
    let message = repl_dispatch::leave_repl_shell(&mut state.session).expect("shell should leave");
    assert_eq!(message, "Leaving ldap shell. Back at root.\n");
    assert!(state.session.scope.is_root());
}

#[test]
fn repl_shell_enter_only_from_root_unit() {
    let mut state = make_completion_state(None);
    let ldap = repl::ReplParsedLine::parse("ldap", state.runtime.config.resolved())
        .expect("ldap should parse");
    assert_eq!(ldap.shell_entry_command(&state.session.scope), Some("ldap"));
    state.session.scope.enter("ldap");
    let mreg = repl::ReplParsedLine::parse("mreg", state.runtime.config.resolved())
        .expect("mreg should parse");
    assert_eq!(mreg.shell_entry_command(&state.session.scope), Some("mreg"));
    assert_eq!(ldap.shell_entry_command(&state.session.scope), None);
}

#[test]
fn repl_partial_root_completion_does_not_enter_shell_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("or", 2);
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        crate::osp_completion::SuggestionOutput::Item(item) if item.text == "orch"
    )));

    let parsed = repl::ReplParsedLine::parse("or", state.runtime.config.resolved())
        .expect("partial command should parse");
    assert_eq!(parsed.shell_entry_command(&state.session.scope), None);
}

#[test]
fn repl_shell_scoped_completion_and_dispatch_prefix_align_unit() {
    let mut state = make_completion_state(None);
    state.session.scope.enter("orch");
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("prov", 4);
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        crate::osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
    )));

    let parsed =
        repl::ReplParsedLine::parse("provision --os alma", state.runtime.config.resolved())
            .expect("scoped command should parse");
    assert_eq!(
        parsed.prefixed_tokens(&state.session.scope),
        vec![
            "orch".to_string(),
            "provision".to_string(),
            "--os".to_string(),
            "alma".to_string()
        ]
    );
}

#[test]
fn repl_alias_partial_completion_does_not_trigger_shell_entry_unit() {
    let state = make_completion_state_with_entries(
        None,
        &[("alias.ops", "orch provision --provider vmware")],
    );
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("op", 2);
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        crate::osp_completion::SuggestionOutput::Item(item) if item.text == "ops"
    )));

    let parsed = repl::ReplParsedLine::parse("op", state.runtime.config.resolved())
        .expect("partial alias should parse");
    assert_eq!(parsed.shell_entry_command(&state.session.scope), None);
}

#[test]
fn repl_structural_alias_exposes_underlying_subcommands_unit() {
    let state = make_completion_state_with_entries(None, &[("alias.ops", "orch")]);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("ops prov", "ops prov".len());
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        crate::osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
    )));
}

#[test]
fn repl_alias_with_invocation_flags_exposes_underlying_subcommands_unit() {
    let state = make_completion_state_with_entries(None, &[("alias.ops", "--json orch")]);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("ops prov", "ops prov".len());
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        crate::osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
    )));
}

#[test]
fn repl_alias_with_prefilled_positional_args_inherits_target_flags_unit() {
    let state = make_completion_state_with_entries(None, &[("alias.me", "orch provision guest")]);
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree.clone());

    let alias_node = tree
        .root
        .children
        .get("me")
        .expect("alias node should exist");
    assert_eq!(alias_node.prefilled_positionals, vec!["guest".to_string()]);

    let (_, suggestions) = engine.complete("me --", "me --".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"--provider".to_string()));
    assert!(values.contains(&"--os".to_string()));
}

#[test]
fn repl_trailing_space_prefers_subcommands_over_flags_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("history ", "history ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(values, vec!["list", "prune", "clear"]);
}

#[test]
fn repl_dash_prefix_switches_from_subcommands_to_flags_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("history -", "history -".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"--json".to_string()));
    assert!(values.contains(&"--color".to_string()));
    assert!(!values.contains(&"list".to_string()));
}

#[test]
fn repl_help_alias_trailing_space_exposes_target_subcommands_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let projected =
        crate::osp_cli::repl::input::project_repl_ui_line("help history ", state.runtime.config.resolved())
            .expect("projection should succeed");

    let (_, suggestions) = engine.complete(&projected.line, projected.line.len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item)
                if !projected.hidden_suggestions.contains(&item.text) =>
            {
                Some(item.text)
            }
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
            crate::osp_completion::SuggestionOutput::Item(_) => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(values, vec!["list", "prune", "clear"]);
}

#[test]
fn repl_help_alias_dash_prefix_exposes_target_flags_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let projected =
        crate::osp_cli::repl::input::project_repl_ui_line("help history -", state.runtime.config.resolved())
            .expect("projection should succeed");

    let (_, suggestions) = engine.complete(&projected.line, projected.line.len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item)
                if !projected.hidden_suggestions.contains(&item.text) =>
            {
                Some(item.text)
            }
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
            crate::osp_completion::SuggestionOutput::Item(_) => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(values, vec!["--verbose"]);
    assert!(!values.contains(&"list".to_string()));
}

#[test]
fn repl_help_root_does_not_suggest_help_or_flags_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let projected =
        crate::osp_cli::repl::input::project_repl_ui_line("help ", state.runtime.config.resolved())
            .expect("projection should succeed");

    let (_, suggestions) = engine.complete(&projected.line, projected.line.len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(!values.contains(&"help".to_string()));
    assert!(!values.iter().any(|value| value.starts_with("--")));
}

#[test]
fn repl_alias_with_invocation_flags_inherits_target_flags_unit() {
    let state =
        make_completion_state_with_entries(None, &[("alias.me", "--json orch provision guest")]);
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree.clone());

    let alias_node = tree
        .root
        .children
        .get("me")
        .expect("alias node should exist");
    assert_eq!(alias_node.prefilled_positionals, vec!["guest".to_string()]);
    assert_eq!(
        alias_node.prefilled_flags.get("--format"),
        Some(&vec!["json".to_string()])
    );

    let (_, suggestions) = engine.complete("me --", "me --".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"--provider".to_string()));
    assert!(values.contains(&"--os".to_string()));
}

#[test]
fn repl_alias_prefilled_context_filters_provider_scoped_values_unit() {
    let state = make_completion_state_with_entries(
        None,
        &[("alias.me", "orch provision guest --provider vmware")],
    );
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("me --os ", "me --os ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"rhel".to_string()));
    assert!(!values.contains(&"alma".to_string()));
}

#[test]
fn repl_alias_with_invocation_flags_filters_provider_scoped_values_unit() {
    let state = make_completion_state_with_entries(
        None,
        &[("alias.me", "--json orch provision guest --provider vmware")],
    );
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("me --os ", "me --os ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"rhel".to_string()));
    assert!(!values.contains(&"alma".to_string()));
}

#[test]
fn repl_alias_placeholder_keeps_following_arg_slot_open_unit() {
    let state =
        make_completion_state_with_entries(None, &[("alias.me", "orch provision guest ${1}")]);
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("me ", "me ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"ubuntu".to_string()));
    assert!(values.contains(&"alma".to_string()));
}

#[test]
fn repl_scoped_relative_alias_keeps_alias_name_in_root_suggestions_unit() {
    let mut state = make_completion_state_with_entries(None, &[("alias.st", "status")]);
    state.session.scope.enter("orch");
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);

    let values = tree.root.args[0]
        .suggestions
        .iter()
        .map(|entry| entry.value.clone())
        .collect::<Vec<_>>();

    assert!(values.contains(&"st".to_string()));
}

#[test]
fn repl_scoped_relative_alias_exposes_shell_subcommands_unit() {
    let mut state = make_completion_state_with_entries(None, &[("alias.st", "status")]);
    state.session.scope.enter("orch");
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("st", "st".len());
    assert!(
        suggestions.is_empty(),
        "relative alias should resolve as a complete shell command, got {suggestions:?}"
    );
}

#[test]
fn repl_scoped_relative_alias_preserves_provider_scoped_values_unit() {
    let mut state = make_completion_state_with_entries(
        None,
        &[("alias.vm", "provision guest --provider vmware")],
    );
    state.session.scope.enter("orch");
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("vm --os ", "vm --os ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"rhel".to_string()));
    assert!(!values.contains(&"alma".to_string()));
}

#[test]
fn repl_scoped_global_alias_falls_back_to_full_command_resolution_unit() {
    let mut state = make_completion_state_with_entries(None, &[("alias.ops", "orch provision")]);
    state.session.scope.enter("orch");
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("ops guest ", "ops guest ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"ubuntu".to_string()));
    assert!(values.contains(&"alma".to_string()));
}

#[test]
fn repl_completion_tree_contains_builtin_and_plugin_commands_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    assert!(tree.root.children.contains_key("help"));
    assert!(tree.root.children.contains_key("exit"));
    assert!(tree.root.children.contains_key("quit"));
    assert!(tree.root.children.contains_key("plugins"));
    assert!(tree.root.children.contains_key("theme"));
    assert!(tree.root.children.contains_key("config"));
    assert!(tree.root.children.contains_key("history"));
    assert!(tree.root.children.contains_key("orch"));
    assert!(
        tree.root.children["orch"]
            .children
            .contains_key("provision")
    );
    assert_eq!(
        tree.root.children["orch"].tooltip.as_deref(),
        Some("Provision orchestrator resources")
    );
    assert!(tree.pipe_verbs.contains_key("F"));
}

#[test]
fn repl_completion_tree_injects_config_set_schema_keys_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let set_node = &tree.root.children["config"].children["set"];
    let ui_mode = &set_node.children["ui.mode"];
    assert!(ui_mode.value_key);
    assert!(ui_mode.children.contains_key("auto"));
    assert!(ui_mode.children.contains_key("plain"));
    assert!(ui_mode.children.contains_key("rich"));

    let repl_intro = &set_node.children["repl.intro"];
    assert!(repl_intro.children.contains_key("true"));
    assert!(repl_intro.children.contains_key("false"));
}

#[test]
fn repl_completion_tree_respects_builtin_visibility_unit() {
    let state = make_completion_state(Some("theme"));
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    assert!(tree.root.children.contains_key("theme"));
    assert!(!tree.root.children.contains_key("config"));
    assert!(!tree.root.children.contains_key("plugins"));
    assert!(!tree.root.children.contains_key("history"));
}

#[test]
fn repl_completion_tree_roots_to_active_shell_scope_unit() {
    let mut state = make_completion_state(None);
    state.session.scope.enter("orch");
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    assert!(!tree.root.children.contains_key("orch"));
    assert!(tree.root.children.contains_key("provision"));
    assert!(tree.root.children.contains_key("help"));
    assert!(tree.root.children.contains_key("exit"));
    assert!(tree.root.children.contains_key("quit"));
}

#[test]
fn repl_surface_drives_overview_and_completion_visibility_unit() {
    let state = make_completion_state(Some("theme config"));
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let names = surface
        .overview_entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names[..2], ["exit", "help"]);
    assert!(names.contains(&"theme"));
    assert!(names.contains(&"config"));
    assert!(names.contains(&"orch"));
    assert!(!names.contains(&"plugins"));
    assert!(!names.contains(&"history"));
    assert!(surface.root_words.contains(&"theme".to_string()));
    assert!(surface.root_words.contains(&"config".to_string()));
    assert!(surface.root_words.contains(&"orch".to_string()));
}

#[test]
fn compact_repl_surface_omits_options_overview_and_prioritizes_builtins_unit() {
    let state =
        make_completion_state_with_entries(Some("theme config"), &[("ui.presentation", "compact")]);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let names = surface
        .overview_entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names[..4], ["exit", "help", "theme", "config"]);
    assert!(!names.contains(&"options"));
    let orch_index = names
        .iter()
        .position(|name| *name == "orch")
        .expect("orch should be present");
    let config_index = names
        .iter()
        .position(|name| *name == "config")
        .expect("config should be present");
    assert!(config_index < orch_index);
}

#[test]
fn compact_root_completion_suggestions_prioritize_core_commands_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "compact")]);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = crate::osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("", 0);
    let labels = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            _ => None,
        })
        .take(6)
        .collect::<Vec<_>>();

    assert_eq!(
        labels[..6],
        ["help", "exit", "quit", "config", "theme", "plugins"]
    );
}

#[test]
fn repl_surface_exposes_selected_provider_for_conflicts_unit() {
    let state = make_completion_state(None);
    let surface = surface::build_repl_surface(
        repl_view(&state.runtime, &state.session),
        &sample_conflicted_catalog(),
    );

    let overview = surface
        .overview_entries
        .iter()
        .find(|entry| entry.name == "hello")
        .expect("hello overview should exist");
    assert!(overview.summary.contains("provider selection required"));
    assert!(overview.summary.contains("--plugin-provider"));
    assert!(overview.summary.contains("beta-provider (user)"));

    let spec = surface
        .specs
        .iter()
        .find(|entry| entry.name == "hello")
        .expect("hello command spec should exist");
    let tooltip = spec.tooltip.as_deref().expect("tooltip should exist");
    assert!(tooltip.contains("provider selection required"));
    assert!(tooltip.contains("beta-provider (user)"));
}
