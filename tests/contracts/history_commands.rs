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

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}
