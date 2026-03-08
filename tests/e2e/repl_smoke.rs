use assert_cmd::Command;

#[test]
#[ignore = "PTY interaction test will be enabled after prompt/session harness is added"]
fn repl_starts_and_exits_on_ctrl_d() {
    let cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let _ = cmd;
}
