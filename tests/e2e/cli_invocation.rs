#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{first_json_row, osp_command, parse_json_stdout, write_config};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;

#[cfg(unix)]
#[test]
fn process_level_profile_selection_affects_builtin_command_output() {
    let home = make_temp_dir("osp-e2e-cli-profile-home");
    write_config(
        home.path(),
        r#"
[default]
profile.default = "uio"

[profile.uio]
theme.name = "nord"

[profile.tsd]
theme.name = "dracula"
"#,
    );

    let default_output = osp_command(home.path())
        .args(["--json", "config", "get", "theme.name"])
        .assert()
        .success()
        .get_output()
        .clone();
    let default_payload = parse_json_stdout(&default_output.stdout);
    let default_row = first_json_row(&default_payload, "default profile config get");
    assert_eq!(default_row["value"], "nord");

    let selected_output = osp_command(home.path())
        .args(["--json", "tsd", "config", "get", "theme.name"])
        .assert()
        .success()
        .get_output()
        .clone();
    let selected_payload = parse_json_stdout(&selected_output.stdout);
    let selected_row = first_json_row(&selected_payload, "selected profile config get");
    assert_eq!(selected_row["value"], "dracula");
    assert!(
        selected_output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&selected_output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn explicit_config_path_env_is_respected_by_real_binary() {
    let home = make_temp_dir("osp-e2e-cli-config-path-home");
    write_config(
        home.path(),
        r#"
[default]
profile.default = "uio"
theme.name = "nord"
"#,
    );

    let explicit_config = home.path().join("explicit.toml");
    std::fs::write(
        &explicit_config,
        r#"
[default]
profile.default = "uio"
theme.name = "dracula"
"#,
    )
    .expect("explicit config should be written");

    let output = osp_command(home.path())
        .env("OSP_CONFIG_FILE", &explicit_config)
        .args(["--json", "config", "get", "theme.name"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "explicit config path env");
    assert_eq!(row["value"], "dracula");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
