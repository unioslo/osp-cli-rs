use assert_cmd::Command;
use serde_json::Value;

#[test]
fn repl_debug_highlight_reports_help_alias_projection_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.args(["repl", "debug-highlight", "--line", "help history -"]);

    let output = cmd.assert().success().get_output().clone();
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("debug-highlight stdout should be json");
    assert_eq!(payload["projected_line"], "     history -");
    let spans = payload["spans"]
        .as_array()
        .expect("spans should render as an array");
    assert!(
        spans
            .iter()
            .any(|span| span["text"] == "help" && span["kind"] == "command_valid")
    );
    assert!(
        spans
            .iter()
            .any(|span| span["text"] == "history" && span["kind"] == "command_valid")
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn repl_debug_highlight_reports_hex_literal_rgb_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.args(["repl", "debug-highlight", "--line", "#ff00cc"]);

    let output = cmd.assert().success().get_output().clone();
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("debug-highlight stdout should be json");
    let spans = payload["spans"]
        .as_array()
        .expect("spans should render as an array");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0]["text"], "#ff00cc");
    assert_eq!(spans[0]["kind"], "color_literal");
    assert_eq!(spans[0]["rgb"], serde_json::json!([255, 0, 204]));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
