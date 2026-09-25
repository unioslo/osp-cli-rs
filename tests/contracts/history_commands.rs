#[cfg(unix)]
use crate::temp_support::make_temp_dir;
use assert_cmd::Command;
use predicates::prelude::*;

#[cfg(unix)]
#[test]
fn history_list_is_available_outside_repl_contract() {
    let home = make_temp_dir("osp-cli-history-contract");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .args(["history", "list"]);

    cmd.assert()
        .success()
        .stderr(predicate::str::is_empty())
        .stdout(predicate::str::is_empty());

    // Empty history remains a usable collection in a one-shot pipeline.
    let output = Command::new(assert_cmd::cargo::cargo_bin!("osp"))
        .envs(crate::test_env::isolated_env(&home))
        .args(["--json", "history", "list", "|", "C"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output).unwrap(),
        serde_json::json!([{"count": 0}])
    );
}
