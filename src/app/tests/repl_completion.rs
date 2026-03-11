use super::*;

fn completion_tree_for(
    state: &AppState,
    catalog: &[CommandCatalogEntry],
) -> crate::completion::CompletionTree {
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), catalog);
    completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface)
}

fn completion_engine_for(
    state: &AppState,
    catalog: &[CommandCatalogEntry],
) -> crate::completion::CompletionEngine {
    crate::completion::CompletionEngine::new(completion_tree_for(state, catalog))
}

fn suggestion_values(output: Vec<crate::completion::SuggestionOutput>) -> Vec<String> {
    output
        .into_iter()
        .filter_map(|entry| match entry {
            crate::completion::SuggestionOutput::Item(item) => Some(item.text),
            crate::completion::SuggestionOutput::PathSentinel => None,
        })
        .collect()
}

fn complete_values(engine: &crate::completion::CompletionEngine, line: &str) -> Vec<String> {
    let (_, suggestions) = engine.complete(line, line.len());
    suggestion_values(suggestions)
}

fn projected_visible_values(
    engine: &crate::completion::CompletionEngine,
    config: &crate::config::ResolvedConfig,
    line: &str,
) -> Vec<String> {
    let projected =
        crate::repl::input::project_repl_ui_line(line, config).expect("projection should succeed");
    let (_, suggestions) = engine.complete(&projected.line, projected.line.len());
    suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::completion::SuggestionOutput::Item(item)
                if !projected.hidden_suggestions.contains(&item.text) =>
            {
                Some(item.text)
            }
            crate::completion::SuggestionOutput::PathSentinel => None,
            crate::completion::SuggestionOutput::Item(_) => None,
        })
        .collect()
}

#[test]
fn repl_help_alias_parsing_and_completion_cover_root_and_scoped_paths_unit() {
    let state = make_completion_state(None);
    for (line, expected_dispatch) in [
        (
            "help ldap user",
            vec!["ldap".to_string(), "user".to_string(), "--help".to_string()],
        ),
        (
            "help ldap --help",
            vec!["ldap".to_string(), "--help".to_string()],
        ),
    ] {
        let parsed =
            repl::ReplParsedLine::parse(line, state.runtime.config.resolved()).expect("parse");
        assert_eq!(parsed.dispatch_tokens, expected_dispatch);
    }

    let parsed = repl::ReplParsedLine::parse("help", state.runtime.config.resolved())
        .expect("bare help should parse");
    assert_eq!(parsed.command_tokens, vec!["help".to_string()]);
    assert_eq!(parsed.dispatch_tokens, vec!["help".to_string()]);

    let catalog = sample_catalog();
    let engine = completion_engine_for(&state, &catalog);
    let values = projected_visible_values(
        &engine,
        state.runtime.config.resolved(),
        "--json help orch prov",
    );
    assert!(values.contains(&"provision".to_string()));

    assert_eq!(
        projected_visible_values(&engine, state.runtime.config.resolved(), "help history "),
        vec!["list", "prune", "clear"]
    );
    assert_eq!(
        projected_visible_values(&engine, state.runtime.config.resolved(), "help history -"),
        vec!["--verbose"]
    );

    let root_values = projected_visible_values(&engine, state.runtime.config.resolved(), "help ");
    assert!(!root_values.contains(&"help".to_string()));
    assert!(!root_values.iter().any(|value| value.starts_with("--")));

    let mut state = make_completion_state(None);
    state.session.scope.enter("orch");
    let catalog = sample_catalog_with_provision_context();
    let engine = completion_engine_for(&state, &catalog);
    let values = projected_visible_values(
        &engine,
        state.runtime.config.resolved(),
        "help provision --p",
    );
    assert!(values.contains(&"--provider".to_string()));
}

#[test]
fn repl_shell_and_scoped_alias_completion_cover_scope_rules_unit() {
    assert!(repl::is_repl_shellable_command("ldap"));
    assert!(repl::is_repl_shellable_command("LDAP"));
    assert!(!repl::is_repl_shellable_command("theme"));

    let mut stack = crate::app::ReplScopeStack::default();
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

    let mut state = make_completion_state(None);
    state.session.scope.enter("ldap");
    let message = repl_dispatch::leave_repl_shell(&mut state.session).expect("shell should leave");
    assert_eq!(message, "Leaving ldap shell. Back at root.\n");
    assert!(state.session.scope.is_root());

    let ldap = repl::ReplParsedLine::parse("ldap", state.runtime.config.resolved())
        .expect("ldap should parse");
    assert_eq!(ldap.shell_entry_command(&state.session.scope), Some("ldap"));
    state.session.scope.enter("ldap");
    let mreg = repl::ReplParsedLine::parse("mreg", state.runtime.config.resolved())
        .expect("mreg should parse");
    assert_eq!(mreg.shell_entry_command(&state.session.scope), Some("mreg"));
    assert_eq!(ldap.shell_entry_command(&state.session.scope), None);

    let mut state = make_completion_state(None);
    let catalog = sample_catalog();
    let engine = completion_engine_for(&state, &catalog);
    let values = complete_values(&engine, "or");
    assert!(values.contains(&"orch".to_string()));

    let parsed = repl::ReplParsedLine::parse("or", state.runtime.config.resolved())
        .expect("partial command should parse");
    assert_eq!(parsed.shell_entry_command(&state.session.scope), None);

    state.session.scope.enter("orch");
    let engine = completion_engine_for(&state, &catalog);
    let values = complete_values(&engine, "prov");
    assert!(values.contains(&"provision".to_string()));

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
    let state = make_completion_state_with_entries(
        None,
        &[("alias.ops", "orch provision --provider vmware")],
    );
    let catalog = sample_catalog();
    let engine = completion_engine_for(&state, &catalog);
    let values = complete_values(&engine, "op");
    assert!(values.contains(&"ops".to_string()));

    let parsed = repl::ReplParsedLine::parse("op", state.runtime.config.resolved())
        .expect("partial alias should parse");
    assert_eq!(parsed.shell_entry_command(&state.session.scope), None);

    let mut state = make_completion_state_with_entries(None, &[("alias.st", "status")]);
    state.session.scope.enter("orch");
    let catalog = sample_catalog();
    let tree = completion_tree_for(&state, &catalog);
    let values = tree.root.args[0]
        .suggestions
        .iter()
        .map(|entry| entry.value.clone())
        .collect::<Vec<_>>();
    assert!(values.contains(&"st".to_string()));

    let engine = crate::completion::CompletionEngine::new(tree);
    let (_, suggestions) = engine.complete("st", "st".len());
    assert!(
        suggestions.is_empty(),
        "relative alias should resolve as a complete shell command, got {suggestions:?}"
    );

    let mut state = make_completion_state_with_entries(
        None,
        &[("alias.vm", "provision guest --provider vmware")],
    );
    state.session.scope.enter("orch");
    let catalog = sample_catalog_with_provision_context();
    let engine = completion_engine_for(&state, &catalog);
    let values = complete_values(&engine, "vm --os ");
    assert!(values.contains(&"rhel".to_string()));
    assert!(!values.contains(&"alma".to_string()));

    let mut state = make_completion_state_with_entries(None, &[("alias.ops", "orch provision")]);
    state.session.scope.enter("orch");
    let catalog = sample_catalog_with_provision_context();
    let engine = completion_engine_for(&state, &catalog);
    let values = complete_values(&engine, "ops guest ");
    assert!(values.contains(&"ubuntu".to_string()));
    assert!(values.contains(&"alma".to_string()));
}

#[test]
fn repl_alias_completion_inherits_prefilled_context_unit() {
    for alias in ["orch", "--json orch"] {
        let state = make_completion_state_with_entries(None, &[("alias.ops", alias)]);
        let catalog = sample_catalog();
        let engine = completion_engine_for(&state, &catalog);
        let values = complete_values(&engine, "ops prov");
        assert!(values.contains(&"provision".to_string()));
    }

    for (alias, expected_flags) in [
        ("orch provision guest", None),
        (
            "--json orch provision guest",
            Some(vec!["json".to_string()]),
        ),
    ] {
        let state = make_completion_state_with_entries(None, &[("alias.me", alias)]);
        let catalog = sample_catalog_with_provision_context();
        let tree = completion_tree_for(&state, &catalog);
        let alias_node = tree
            .root
            .children
            .get("me")
            .expect("alias node should exist");
        assert_eq!(alias_node.prefilled_positionals, vec!["guest".to_string()]);
        if let Some(expected) = expected_flags {
            assert_eq!(alias_node.prefilled_flags.get("--format"), Some(&expected));
        } else {
            assert!(!alias_node.prefilled_flags.contains_key("--format"));
        }

        let engine = crate::completion::CompletionEngine::new(tree);
        let values = complete_values(&engine, "me --");
        assert!(values.contains(&"--provider".to_string()));
        assert!(values.contains(&"--os".to_string()));
    }

    for alias in [
        "orch provision guest --provider vmware",
        "--json orch provision guest --provider vmware",
    ] {
        let state = make_completion_state_with_entries(None, &[("alias.me", alias)]);
        let catalog = sample_catalog_with_provision_context();
        let engine = completion_engine_for(&state, &catalog);
        let values = complete_values(&engine, "me --os ");
        assert!(values.contains(&"rhel".to_string()));
        assert!(!values.contains(&"alma".to_string()));
    }

    let state =
        make_completion_state_with_entries(None, &[("alias.me", "orch provision guest ${1}")]);
    let catalog = sample_catalog_with_provision_context();
    let engine = completion_engine_for(&state, &catalog);
    let values = complete_values(&engine, "me ");
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
    assert!(repl_intro.children.contains_key("none"));
    assert!(repl_intro.children.contains_key("minimal"));
    assert!(repl_intro.children.contains_key("compact"));
    assert!(repl_intro.children.contains_key("full"));
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
    let engine = crate::completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("", 0);
    let labels = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            crate::completion::SuggestionOutput::Item(item) => Some(item.text),
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

#[test]
fn repl_surface_includes_plugin_auth_hints_in_overview_and_tooltip_unit() {
    let state = make_completion_state(None);
    let surface = surface::build_repl_surface(
        repl_view(&state.runtime, &state.session),
        &[crate::plugin::CommandCatalogEntry {
            name: "orch".to_string(),
            about: "orch plugin".to_string(),
            auth: Some(crate::core::plugin::DescribeCommandAuthV1 {
                visibility: Some(crate::core::plugin::DescribeVisibilityModeV1::CapabilityGated),
                required_capabilities: vec!["orch.approval.decide".to_string()],
                feature_flags: vec!["orch".to_string()],
            }),
            subcommands: vec!["approval".to_string()],
            completion: crate::completion::CommandSpec::new("orch"),
            provider: Some("orch".to_string()),
            providers: vec!["orch (explicit)".to_string()],
            conflicted: false,
            requires_selection: false,
            selected_explicitly: false,
            source: Some(crate::plugin::PluginSource::Explicit),
        }],
    );

    let overview = surface
        .overview_entries
        .iter()
        .find(|entry| entry.name == "orch")
        .expect("orch overview should exist");
    assert!(
        overview
            .summary
            .contains("[cap: orch.approval.decide; feature: orch]")
    );

    let spec = surface
        .specs
        .iter()
        .find(|entry| entry.name == "orch")
        .expect("orch command spec should exist");
    let tooltip = spec.tooltip.as_deref().expect("tooltip should exist");
    assert!(tooltip.contains("[cap: orch.approval.decide; feature: orch]"));
}
