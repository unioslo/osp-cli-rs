use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn cli_reports_workspace_version_with_long_flag_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.arg("--version");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "osp {}\n",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn cli_reports_workspace_version_with_short_flag_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.arg("-V");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "osp {}\n",
            env!("CARGO_PKG_VERSION")
        )));
}
