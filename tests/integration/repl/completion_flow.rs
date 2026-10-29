use osp_cli::App;
use osp_cli::app::BufferedUiSink;

use crate::app::support::parse_json_output;

fn debug_complete_payload(line: &str, steps: &[&str]) -> serde_json::Value {
    let app = App::new();
    let mut sink = BufferedUiSink::default();
    let mut args = vec![
        "osp".to_string(),
        "--json".to_string(),
        "--defaults-only".to_string(),
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

    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    parse_json_output(
        "repl_completion_flow/debug_complete",
        &arg_refs,
        &sink.stdout,
        &sink.stderr,
    )
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
    let forget = match_labels(&debug_complete_payload("forget theme ", &[]));
    assert!(forget.contains(&"list".to_owned()));
    let remember = match_labels(&debug_complete_payload("remember --l", &[]));
    assert!(remember.contains(&"--list".to_owned()));

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
    assert_eq!(uncommitted_flag.first().map(String::as_str), Some("--raw"));

    let committed_flag = match_labels(&debug_complete_payload("config show --raw ", &[]));
    assert!(!committed_flag.contains(&"--raw".to_string()));
    assert!(committed_flag.contains(&"--sources".to_string()));

    for (line, accepted) in [
        (
            "config set --yes ui.format j",
            "config set --yes ui.format json ",
        ),
        (
            "config set --profile default ui.format j",
            "config set --profile default ui.format json ",
        ),
    ] {
        assert_eq!(tab_completed_line(line).as_deref(), Some(accepted));
    }
    let values = match_labels(&debug_complete_payload("config set --yes ui.format ", &[]));
    assert!(values.contains(&"json".to_string()));
    assert!(!values.contains(&"ui.format".to_string()));

    let pipeline = match_labels(&debug_complete_payload("config show | ", &[]));
    for verb in ["F", "P", "S", "G"] {
        assert!(pipeline.contains(&verb.to_string()), "{pipeline:?}");
    }

    let pipe_cursor = debug_complete_payload("config show|", &[]);
    assert_eq!(pipe_cursor["stub"], "");
    assert_eq!(pipe_cursor["replace_range"], serde_json::json!([12, 12]));
    assert!(match_labels(&pipe_cursor).contains(&"F".to_string()));

    for (line, accepted) in [
        ("config show|F", "config show|F "),
        ("config show |F", "config show |F "),
    ] {
        assert_eq!(tab_completed_line(line).as_deref(), Some(accepted));
    }
}

#[test]
fn repl_completion_flow_first_tab_returns_matches_for_current_slot() {
    let payload = debug_complete_payload("config ", &["tab"]);
    let frames = payload
        .as_array()
        .expect("stepped debug-complete should render frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["step"], "tab");
    assert!(
        frames[0]["state"]["matches"]
            .as_array()
            .expect("frame matches should render as an array")
            .iter()
            .any(|item| item["label"] == "show")
    );

    let inline_value = debug_complete_payload("config show --format=j", &[]);
    assert_eq!(inline_value["stub"], "j");
    assert_eq!(inline_value["replace_range"], serde_json::json!([21, 22]));
    assert_eq!(
        tab_completed_line("config show --format=j").as_deref(),
        Some("config show --format=json ")
    );
}

/// The line after one Tab, which completes a unique match like the editor.
fn tab_completed_line(line: &str) -> Option<String> {
    let payload = debug_complete_payload(line, &["tab"]);
    payload.as_array()?.last()?["state"]["line"]
        .as_str()
        .map(str::to_string)
}
