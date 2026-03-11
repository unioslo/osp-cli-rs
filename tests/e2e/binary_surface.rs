#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{first_json_row, osp_command, parse_json_stdout, stderr_utf8};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;

#[cfg(unix)]
#[test]
fn binary_help_exits_zero_and_writes_stdout_only() {
    let home = make_temp_dir("osp-e2e-binary-help");
    let output = osp_command(home.path())
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    assert!(stdout.contains("OSP CLI"));
    assert!(stdout.contains("osp [OPTIONS] [COMMAND]"));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn binary_version_exits_zero_and_writes_stdout_only() {
    let home = make_temp_dir("osp-e2e-binary-version");
    let output = osp_command(home.path())
        .arg("--version")
        .assert()
        .success()
        .get_output()
        .clone();

    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        format!("osp {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn binary_invalid_subcommand_exits_nonzero_and_writes_usage_to_stderr() {
    let home = make_temp_dir("osp-e2e-binary-invalid-subcommand");
    let output = osp_command(home.path())
        .args(["config", "nope"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .clone();

    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = stderr_utf8(output.stderr);
    assert!(stderr.contains("unrecognized subcommand 'nope'"));
    assert!(stderr.contains("Usage: osp config [OPTIONS] <COMMAND>"));
}

#[cfg(unix)]
#[test]
fn binary_builtin_json_command_exits_zero_and_keeps_stdout_machine_readable() {
    let home = make_temp_dir("osp-e2e-binary-json-command");
    let output = osp_command(home.path())
        .args([
            "--json",
            "--no-env",
            "--no-config-file",
            "config",
            "get",
            "theme.name",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "binary json config get");
    assert_eq!(row["key"], "theme.name");
    assert_eq!(row["value"], "rose-pine-moon");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
