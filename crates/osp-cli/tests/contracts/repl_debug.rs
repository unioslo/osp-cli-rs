use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn repl_debug_highlight_reports_help_alias_projection_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.args(["repl", "debug-highlight", "--line", "help history -"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "\"projected_line\": \"     history -\"",
        ))
        .stdout(predicate::str::contains("\"text\": \"help\""))
        .stdout(predicate::str::contains("\"text\": \"history\""))
        .stdout(predicate::str::contains("\"kind\": \"command_valid\""));
}

#[test]
fn repl_debug_highlight_reports_hex_literal_rgb_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.args(["repl", "debug-highlight", "--line", "#ff00cc"]);

    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"text\": \"#ff00cc\""))
        .stdout(predicate::str::contains("\"kind\": \"color_literal\""))
        .stdout(predicate::str::contains("\"rgb\": ["))
        .stdout(predicate::str::contains("255"))
        .stdout(predicate::str::contains("204"));
}
