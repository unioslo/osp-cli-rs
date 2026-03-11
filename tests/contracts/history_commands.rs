#[cfg(unix)]
use crate::temp_support::make_temp_dir;
use assert_cmd::Command;
use predicates::prelude::*;

#[cfg(unix)]
#[test]
fn history_command_is_rejected_outside_repl_contract() {
    let home = make_temp_dir("osp-cli-history-contract");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .args(["history", "list"]);

    cmd.assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("history commands are REPL-only"))
        .stdout(predicate::str::is_empty());
}
