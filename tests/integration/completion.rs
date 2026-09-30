use osp_cli::completion::{
    CommandSpec, CompletionEngine, CompletionTreeBuilder, ContextScope, FlagNode, PlanningHints,
    PlanningNumericColumn, PlanningRow, PlanningTable, PlanningValue, SuggestionEntry,
    SuggestionOutput, narrow_provider_candidates,
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
    let catalogue = osp_cli::completion::model::PrefixValues::default();
    let mut tree = completion_tree(ContextScope::Global);
    tree.root
        .children
        .get_mut("service")
        .unwrap()
        .children
        .get_mut("deploy")
        .unwrap()
        .flags
        .insert(
            "--hostname".into(),
            FlagNode {
                prefix_values: Some(catalogue.clone()),
                ..FlagNode::default()
            },
        );
    let engine = CompletionEngine::new(tree.clone());
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

    // A refreshed catalogue reaches an existing engine through its cloned tree.
    catalogue.replace(
        (0..30)
            .rev()
            .map(|index| format!("node-{index:02}"))
            .chain(["node-00".into(), "other-host".into()])
            .collect(),
    );
    let line = "service deploy --hostname node-";
    assert_eq!(
        suggestion_values(engine.complete(line, line.len()).1),
        (0..25)
            .map(|index| format!("node-{index:02}"))
            .collect::<Vec<_>>()
    );
    assert!(catalogue.contains("node-29"));
    catalogue.replace(vec!["node-refreshed".into()]);
    assert_eq!(
        suggestion_values(engine.complete(line, line.len()).1),
        vec!["node-refreshed"]
    );
}

#[test]
fn completion_planning_narrows_provider_and_runtime_choices() {
    // The same provider context scopes relational choices. Two runtime lanes
    // share a provider but have different images and available memory.
    let mut tree = completion_tree(ContextScope::Global);
    let deploy = tree
        .root
        .children
        .get_mut("service")
        .unwrap()
        .children
        .get_mut("deploy")
        .unwrap();
    deploy.flags.insert("--memory".into(), FlagNode::new());
    deploy.flags.insert("--provider".into(), FlagNode::new());
    let mut table = PlanningTable::new()
        .exact_column("provider")
        .exact_column("image")
        .minimum_column(
            "memory",
            PlanningNumericColumn::default().unit_scale("GiB", 1024),
        )
        .rows([
            PlanningRow::new()
                .value("provider", "alpha")
                .value("instance", "east")
                .value("instance_id", "one")
                .value("image", "red")
                .value("memory", 4096),
            PlanningRow::new()
                .value("provider", "alpha")
                .value("instance", "west")
                .value("instance_id", "two")
                .value("image", "green")
                .value("memory", "16GiB"),
            PlanningRow::new()
                .value("provider", "beta")
                .value("instance", "east")
                .value("instance_id", "three")
                .value("image", "blue")
                .value("memory", 8192),
        ]);
    table.exhaustive = true;
    deploy.planning = Some(
        PlanningHints::default()
            .provider_column("provider")
            .identity_column("provider")
            .identity_column("instance")
            .identity_column("instance_id")
            .table(table),
    );
    let engine = CompletionEngine::new(tree.clone());
    let node = &tree.root.children["service"].children["deploy"];
    for (line, expected) in [
        (
            "service deploy --image  --provider alpha:east:one",
            vec!["red"],
        ),
        (
            "service deploy --image  --provider alpha:west:two",
            vec!["green"],
        ),
        (
            "service deploy --memory 8GiB --image  --provider alpha",
            vec!["green"],
        ),
        (
            "service deploy --memory 8192 --image  --provider beta",
            vec!["blue"],
        ),
    ] {
        let cursor = line.find("--image").unwrap() + "--image ".len();
        assert_eq!(
            suggestion_values(engine.complete(line, cursor).1),
            expected,
            "{line}"
        );
    }
    for (memory, expected) in [
        ("8GiB", vec!["alpha", "beta"]),
        ("16GiB", vec!["alpha"]),
        ("32GiB", vec![]),
    ] {
        let line = format!("service deploy --memory {memory} ");
        let analysis = engine.analyze(&line, line.len());
        let narrowed = narrow_provider_candidates(&analysis.parsed.cursor_cmd, node);
        assert_eq!(narrowed.candidates().collect::<Vec<_>>(), expected);
        assert_eq!(narrowed.all().collect::<Vec<_>>(), vec!["alpha", "beta"]);
        assert_eq!(narrowed.is_contradictory(), memory == "32GiB");
    }
    let line = "service deploy --memory 16GiB --provider ";
    assert_eq!(
        suggestion_values(engine.complete(line, line.len()).1),
        vec!["alpha:west:two"]
    );

    // Incomplete runtime evidence is advisory: it must retain selectable
    // provider choices rather than treating an unknown capacity as zero.
    let deploy = tree
        .root
        .children
        .get_mut("service")
        .unwrap()
        .children
        .get_mut("deploy")
        .unwrap();
    let table = &mut deploy.planning.as_mut().unwrap().tables[0];
    table.rows.push(
        PlanningRow::new()
            .value("provider", "beta")
            .value("instance", PlanningValue::unknown())
            .value("memory", PlanningValue::unknown()),
    );
    let engine = CompletionEngine::new(tree.clone());
    let line = "service deploy --memory 32GiB --provider beta:west:four ";
    let analysis = engine.analyze(line, line.len());
    let narrowed = narrow_provider_candidates(
        &analysis.parsed.cursor_cmd,
        &tree.root.children["service"].children["deploy"],
    );
    assert_eq!(narrowed.explicit(), Some("beta"));
    assert_eq!(narrowed.candidates().collect::<Vec<_>>(), vec!["beta"]);

    // Runtime identities and exact constraints can be numeric even though the
    // editable command line and provider selectors carry textual values.
    let deploy = tree
        .root
        .children
        .get_mut("service")
        .unwrap()
        .children
        .get_mut("deploy")
        .unwrap();
    for flag in ["--revision", "--instance_id"] {
        deploy.flags.insert(flag.into(), FlagNode::new());
    }
    let table = &mut deploy.planning.as_mut().unwrap().tables[0];
    table
        .columns
        .extend(["revision".into(), "instance_id".into()]);
    for row in &mut table.rows {
        row.values
            .insert("revision".into(), PlanningValue::number(1));
    }
    table.rows.push(
        PlanningRow::new()
            .value("provider", "alpha")
            .value("instance", "north")
            .value("instance_id", PlanningValue::number(4))
            .value("revision", PlanningValue::number(2))
            .value("image", "amber")
            .value("memory", 4096),
    );
    let engine = CompletionEngine::new(tree.clone());
    for (line, flag, expected) in [
        (
            "service deploy --revision 2 --memory +4GiB --image  --provider alpha",
            "--image",
            vec!["amber"],
        ),
        (
            "service deploy --image  --provider alpha:north:4",
            "--image",
            vec!["amber"],
        ),
        (
            "service deploy --instance_id  --provider alpha:north:4",
            "--instance_id",
            vec!["4"],
        ),
        (
            "service deploy --revision 2 --memory +4GiB --provider ",
            "--provider",
            vec!["alpha:north:4"],
        ),
    ] {
        let cursor = line.find(flag).unwrap() + flag.len() + 1;
        assert_eq!(
            suggestion_values(engine.complete(line, cursor).1),
            expected,
            "{line}"
        );
    }

    // A partial catalogue augments static choices; overlapping evidence must
    // produce one selectable value rather than duplicate menu entries.
    let deploy = tree
        .root
        .children
        .get_mut("service")
        .unwrap()
        .children
        .get_mut("deploy")
        .unwrap();
    deploy.planning.as_mut().unwrap().tables[0].exhaustive = false;
    deploy
        .flags
        .get_mut("--image")
        .unwrap()
        .suggestions_by_provider
        .insert(
            "alpha".into(),
            vec![SuggestionEntry::from("amber"), SuggestionEntry::from("red")],
        );
    let engine = CompletionEngine::new(tree);
    let line = "service deploy --revision 2 --image  --provider alpha:north:4";
    let cursor = line.find("--image").unwrap() + "--image ".len();
    assert_eq!(
        suggestion_values(engine.complete(line, cursor).1),
        vec!["amber", "red"]
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
