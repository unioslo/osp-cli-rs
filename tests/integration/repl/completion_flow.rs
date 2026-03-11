use osp_cli::app::{AppBuilder, BufferedUiSink};

fn debug_complete_payload(line: &str, steps: &[&str]) -> serde_json::Value {
    let app = AppBuilder::new().build();
    let mut sink = BufferedUiSink::default();
    let mut args = vec![
        "osp".to_string(),
        "--json".to_string(),
        "--no-env".to_string(),
        "--no-config-file".to_string(),
        "repl".to_string(),
        "debug-complete".to_string(),
        "--line".to_string(),
        line.to_string(),
    ];
    for step in steps {
        args.push("--step".to_string());
        args.push((*step).to_string());
    }

    let exit = app
        .run_with_sink(args.iter().map(String::as_str), &mut sink)
        .expect("debug-complete should run");
    assert_eq!(exit, 0);
    assert!(sink.stderr.is_empty(), "unexpected stderr: {}", sink.stderr);

    serde_json::from_str(&sink.stdout).expect("debug-complete stdout should be json")
}

fn match_labels(payload: &serde_json::Value) -> Vec<String> {
    payload["matches"]
        .as_array()
        .expect("matches should render as an array")
        .iter()
        .map(|item| {
            item["label"]
                .as_str()
                .expect("match label should be a string")
                .to_string()
        })
        .collect()
}

#[test]
fn repl_completion_flow_tracks_committed_scope_and_consumed_flags() {
    let root_stub = match_labels(&debug_complete_payload("config", &[]));
    assert!(root_stub.contains(&"config".to_string()));
    assert!(!root_stub.contains(&"show".to_string()));

    let scoped = match_labels(&debug_complete_payload("config ", &[]));
    assert!(scoped.contains(&"show".to_string()));
    assert!(scoped.contains(&"get".to_string()));
    assert!(scoped.contains(&"explain".to_string()));
    assert!(!scoped.contains(&"config".to_string()));

    let uncommitted_subcommand = match_labels(&debug_complete_payload("config show", &[]));
    assert!(uncommitted_subcommand.contains(&"show".to_string()));
    assert!(!uncommitted_subcommand.contains(&"--raw".to_string()));

    let flag_slot = match_labels(&debug_complete_payload("config show ", &[]));
    assert!(flag_slot.contains(&"--raw".to_string()));
    assert!(flag_slot.contains(&"--sources".to_string()));
    assert!(!flag_slot.contains(&"show".to_string()));

    let uncommitted_flag = match_labels(&debug_complete_payload("config show --raw", &[]));
    assert!(uncommitted_flag.contains(&"--raw".to_string()));
    assert!(uncommitted_flag.contains(&"--sources".to_string()));

    let committed_flag = match_labels(&debug_complete_payload("config show --raw ", &[]));
    assert!(!committed_flag.contains(&"--raw".to_string()));
    assert!(committed_flag.contains(&"--sources".to_string()));
}

#[test]
fn repl_completion_flow_keeps_hidden_help_token_visible_until_space_commits_it() {
    let exact = match_labels(&debug_complete_payload("help", &[]));
    assert!(exact.contains(&"help".to_string()));

    let committed = match_labels(&debug_complete_payload("help ", &[]));
    assert!(!committed.contains(&"help".to_string()));
    assert!(committed.is_empty());
}

#[test]
fn repl_completion_flow_first_tab_opens_menu_for_current_slot() {
    let payload = debug_complete_payload("config ", &["tab"]);
    let frames = payload
        .as_array()
        .expect("stepped debug-complete should render frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["step"], "tab");
    assert_eq!(frames[0]["state"]["selected"], 0);
    assert!(
        frames[0]["state"]["matches"]
            .as_array()
            .expect("frame matches should render as an array")
            .iter()
            .any(|item| item["label"] == "show")
    );
    assert!(
        !frames[0]["state"]["rendered"]
            .as_array()
            .expect("frame render should be an array")
            .is_empty()
    );
}
