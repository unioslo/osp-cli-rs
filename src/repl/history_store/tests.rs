use super::*;
use chrono::{TimeZone, Utc};

fn history_config() -> HistoryConfigBuilder {
    HistoryConfig::builder()
        .with_enabled(true)
        .with_max_entries(10)
        .with_dedupe(false)
        .with_profile_scoped(false)
}

#[test]
fn wildcard_matching_handles_prefix_and_infix() {
    assert!(matches_pattern("ldap user *", "ldap user bob"));
    assert!(matches_pattern("*token*", "auth token read"));
    assert!(!matches_pattern("auth", "auth token"));
    assert!(matches_pattern("auth*", "auth token"));
    assert!(matches_pattern("*user", "ldap user"));
    assert!(!matches_pattern("*user", "ldap user bob"));
}

#[test]
fn excluded_commands_respect_prefixes_and_patterns() {
    let excludes = vec![
        "help".to_string(),
        "exit".to_string(),
        "quit".to_string(),
        "history list".to_string(),
    ];
    assert!(is_excluded_command("help", &excludes));
    assert!(is_excluded_command("history list", &excludes));
    assert!(!is_excluded_command("history prune 10", &[]));
    assert!(is_excluded_command("ldap user --help", &[]));
    assert!(is_excluded_command(
        "login oistes",
        &[String::from("login *")]
    ));
}

#[test]
fn list_entries_filters_shell_and_excludes() {
    let shell = HistoryShellContext::new("ldap");
    let config = history_config()
        .with_exclude_patterns(["user *"])
        .with_shell_context(shell)
        .build();
    let mut store = OspHistoryStore::new(config).expect("history store should init");
    let _ = History::save(
        &mut store,
        HistoryItem::from_command_line("ldap user alice"),
    )
    .expect("save should succeed");
    let _ = History::save(
        &mut store,
        HistoryItem::from_command_line("ldap netgroup ucore"),
    )
    .expect("save should succeed");
    let _ = History::save(&mut store, HistoryItem::from_command_line("mreg host a"))
        .expect("save should succeed");

    let entries = store.list_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].command, "netgroup ucore");
    assert_eq!(entries[1].command, "mreg host a");
}

#[test]
fn list_entries_tracks_live_shell_context_updates() {
    let shell = HistoryShellContext::default();
    let config = history_config().with_shell_context(shell.clone()).build();
    let mut store = OspHistoryStore::new(config).expect("history store should init");
    let _ = History::save(
        &mut store,
        HistoryItem::from_command_line("ldap user alice"),
    )
    .expect("save should succeed");
    let _ = History::save(&mut store, HistoryItem::from_command_line("mreg host a"))
        .expect("save should succeed");

    shell.set_prefix("ldap");
    let ldap_entries = store.list_entries();
    assert_eq!(ldap_entries.len(), 1);
    assert_eq!(ldap_entries[0].command, "user alice");

    shell.set_prefix("mreg");
    let mreg_entries = store.list_entries();
    assert_eq!(mreg_entries.len(), 1);
    assert_eq!(mreg_entries[0].command, "host a");

    shell.clear();
    let root_entries = store.list_entries();
    assert_eq!(root_entries.len(), 2);
}

#[test]
fn explicit_scope_queries_override_live_shell_context() {
    let shell = HistoryShellContext::default();
    let config = history_config().with_shell_context(shell.clone()).build();
    let mut store = OspHistoryStore::new(config).expect("history store should init");
    let _ = History::save(
        &mut store,
        HistoryItem::from_command_line("ldap user alice"),
    )
    .expect("save should succeed");
    let _ = History::save(&mut store, HistoryItem::from_command_line("mreg host a"))
        .expect("save should succeed");

    shell.set_prefix("ldap");
    let mreg_entries = store.list_entries_for(Some("mreg"));
    assert_eq!(mreg_entries.len(), 1);
    assert_eq!(mreg_entries[0].command, "host a");

    let removed = store
        .prune_for(0, Some("mreg"))
        .expect("prune should succeed");
    assert_eq!(removed, 1);

    let root_entries = store.list_entries_for(None);
    assert_eq!(root_entries.len(), 1);
    assert_eq!(root_entries[0].command, "ldap user alice");
}

#[test]
fn save_expands_history_and_dedupes_with_shell_scope() {
    let shell = HistoryShellContext::new("ldap");
    let config = history_config()
        .with_dedupe(true)
        .with_shell_context(shell)
        .build();
    let mut store = OspHistoryStore::new(config).expect("history store should init");

    let first = History::save(&mut store, HistoryItem::from_command_line("user alice"))
        .expect("save should succeed");
    assert_eq!(first.command_line, "ldap user alice");

    let duplicate = History::save(&mut store, HistoryItem::from_command_line("!!"))
        .expect("history expansion should succeed");
    assert_eq!(duplicate.command_line, "!!");
    assert_eq!(store.list_entries().len(), 1);

    let second = History::save(&mut store, HistoryItem::from_command_line("netgroup ops"))
        .expect("save should succeed");
    assert_eq!(second.command_line, "ldap netgroup ops");

    let recent = store.recent_commands();
    assert_eq!(recent, vec!["ldap user alice", "ldap netgroup ops"]);
    let visible = store.list_entries();
    assert_eq!(visible[0].command, "user alice");
    assert_eq!(visible[1].command, "netgroup ops");
}

#[test]
fn search_respects_filters_direction_bounds_and_skip_logic() {
    let config = history_config()
        .with_shell_context(HistoryShellContext::default())
        .build();
    let mut store = OspHistoryStore::new(config).expect("history store should init");

    let mut first = HistoryItem::from_command_line("ldap user alice");
    first.cwd = Some("/srv/ldap".to_string());
    first.hostname = Some("ops-a".to_string());
    first.exit_status = Some(0);
    first.start_timestamp = Some(Utc.timestamp_millis_opt(1_000).single().unwrap());
    History::save(&mut store, first).expect("save should succeed");

    let mut second = HistoryItem::from_command_line("ldap user bob");
    second.cwd = Some("/srv/ldap/cache".to_string());
    second.hostname = Some("ops-b".to_string());
    second.exit_status = Some(1);
    second.start_timestamp = Some(Utc.timestamp_millis_opt(2_000).single().unwrap());
    History::save(&mut store, second).expect("save should succeed");

    let mut third = HistoryItem::from_command_line("mreg host a");
    third.cwd = Some("/srv/mreg".to_string());
    third.hostname = Some("ops-a".to_string());
    third.exit_status = Some(0);
    third.start_timestamp = Some(Utc.timestamp_millis_opt(3_000).single().unwrap());
    History::save(&mut store, third).expect("save should succeed");

    let mut filter = SearchFilter::anything(None);
    filter.command_line = Some(CommandLineSearch::Prefix("ldap".to_string()));
    filter.cwd_prefix = Some("/srv/ldap".to_string());
    filter.exit_successful = Some(true);
    filter.hostname = Some("ops-a".to_string());

    let forward = SearchQuery {
        direction: SearchDirection::Forward,
        start_time: Some(Utc.timestamp_millis_opt(500).single().unwrap()),
        end_time: Some(Utc.timestamp_millis_opt(1_500).single().unwrap()),
        start_id: None,
        end_id: Some(HistoryItemId::new(2)),
        limit: Some(5),
        filter,
    };
    let results = store.search(forward).expect("search should succeed");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].command_line, "ldap user alice");

    let mut backward = SearchQuery::everything(SearchDirection::Backward, None);
    backward.start_id = Some(HistoryItemId::new(1));
    backward.limit = Some(2);
    let results = store.search(backward).expect("search should succeed");
    let commands = results
        .iter()
        .map(|item| item.command_line.as_str())
        .collect::<Vec<_>>();
    assert_eq!(commands, vec!["ldap user alice"]);
    assert_eq!(
        store
            .count(SearchQuery::everything(SearchDirection::Forward, None))
            .expect("count should succeed"),
        3
    );
}

#[test]
fn persisted_records_skip_invalid_lines_and_trim_to_capacity() {
    let temp_dir = make_temp_dir("osp-repl-history-load");
    let path = temp_dir.join("history.jsonl");
    std::fs::write(
        &path,
        concat!(
            "\n",
            "{\"id\":5,\"command_line\":\"first\",\"timestamp_ms\":10}\n",
            "not-json\n",
            "{\"id\":6,\"command_line\":\"   \",\"timestamp_ms\":20}\n",
            "{\"id\":7,\"command_line\":\"second\",\"timestamp_ms\":30}\n"
        ),
    )
    .expect("history fixture should be written");

    let store = OspHistoryStore::new(
        history_config()
            .with_path(Some(path))
            .with_max_entries(1)
            .with_shell_context(HistoryShellContext::default())
            .build(),
    )
    .expect("history store should init");

    let entries = store.list_entries_for(None);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, 1);
    assert_eq!(entries[0].command, "second");
}

#[test]
fn shared_history_supports_save_load_prune_clear_and_sync() {
    let temp_dir = make_temp_dir("osp-repl-shared-history");
    let path = temp_dir.join("history.jsonl");
    let mut history = SharedHistory::new(
        history_config()
            .with_path(Some(path.clone()))
            .with_max_entries(8)
            .with_shell_context(HistoryShellContext::default())
            .build(),
    )
    .expect("shared history should init");

    history
        .save_command_line("config show")
        .expect("save should succeed");
    history
        .save_command_line("config get ui.format")
        .expect("save should succeed");
    assert!(history.enabled());
    assert_eq!(history.recent_commands().len(), 2);
    assert_eq!(
        history
            .load(HistoryItemId::new(0))
            .expect("load should succeed")
            .command_line,
        "config show"
    );

    assert_eq!(history.prune(1).expect("prune should succeed"), 1);
    assert_eq!(history.list_entries().len(), 1);
    history.sync().expect("sync should succeed");
    assert!(path.exists());
    assert_eq!(history.clear_for(None).expect("clear should succeed"), 1);
    assert!(history.list_entries().is_empty());
    History::clear(&mut history).expect("clear should succeed");
    assert!(!path.exists());
}

#[test]
fn shell_prefix_helpers_normalize_and_round_trip_commands() {
    assert_eq!(
        normalize_shell_prefix(" ldap ".to_string()),
        Some("ldap ".to_string())
    );
    assert_eq!(
        normalize_scope_prefix(Some("ldap")),
        Some("ldap ".to_string())
    );
    assert!(command_matches_shell_prefix(
        "ldap user alice",
        Some("ldap ")
    ));
    assert_eq!(
        apply_shell_prefix("user alice", Some("ldap ")),
        "ldap user alice"
    );
    assert_eq!(
        apply_shell_prefix("ldap user alice", Some("ldap ")),
        "ldap user alice"
    );
    assert_eq!(
        strip_shell_prefix("ldap user alice", Some("ldap ")),
        "user alice"
    );
}

#[test]
fn unsupported_history_mutations_surface_feature_errors() {
    let mut store = OspHistoryStore::new(
        history_config()
            .with_max_entries(4)
            .with_shell_context(HistoryShellContext::default())
            .build(),
    )
    .expect("history store should init");

    let update_err = store
        .update(HistoryItemId::new(0), &|item| item)
        .expect_err("update should stay unsupported");
    let delete_err = store
        .delete(HistoryItemId::new(0))
        .expect_err("delete should stay unsupported");

    assert!(update_err.to_string().contains("updating entries"));
    assert!(delete_err.to_string().contains("removing entries"));
    assert_eq!(store.session(), None);
}

#[test]
fn load_missing_history_item_returns_not_found_error() {
    let store = OspHistoryStore::new(
        history_config()
            .with_max_entries(4)
            .with_shell_context(HistoryShellContext::default())
            .build(),
    )
    .expect("history store should init");

    let err = store
        .load(HistoryItemId::new(7))
        .expect_err("missing entry should fail");
    assert!(err.to_string().contains("history item not found"));
}

#[test]
fn disabled_history_returns_original_item_without_persisting_records() {
    let mut store = OspHistoryStore::new(
        history_config()
            .with_enabled(false)
            .with_dedupe(true)
            .with_shell_context(HistoryShellContext::default())
            .build(),
    )
    .expect("history store should init");

    let item = History::save(
        &mut store,
        HistoryItem::from_command_line("ldap user alice"),
    )
    .expect("disabled history should be a no-op");

    assert_eq!(item.command_line, "ldap user alice");
    assert!(store.list_entries().is_empty());
    assert!(store.recent_commands().is_empty());
}

fn make_temp_dir(prefix: &str) -> crate::tests::TestTempDir {
    crate::tests::make_temp_dir(prefix)
}
