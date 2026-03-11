use crate::temp_support::make_temp_dir;
use crate::test_env::isolated_env;
use assert_cmd::Command;

fn run_cli_stdout(args: &[&str]) -> String {
    let home = make_temp_dir("osp-cli-command-surfaces");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("LANG", "C.UTF-8");
    for (key, value) in isolated_env(home.path()) {
        cmd.env(key, value);
    }
    let output = cmd
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("stdout should be utf-8")
}

#[test]
fn intro_command_emits_semantic_json_with_explicit_format_contract() {
    let stdout = run_cli_stdout(&["--no-env", "--no-config-file", "--json", "intro"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("intro JSON should parse");
    let rows = json.as_array().expect("guide output should be row array");
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].get("sections").is_some() || rows[0].get("preamble").is_some(),
        "expected semantic intro payload, got: {stdout}"
    );
}

#[test]
fn top_level_help_supports_all_explicit_output_formats_contract() {
    let json = run_cli_stdout(&["--no-env", "--no-config-file", "--json", "--help"]);
    let json_value: serde_json::Value =
        serde_json::from_str(&json).expect("help json should parse");
    let json_rows = json_value
        .as_array()
        .expect("help json should be row array");
    assert_eq!(json_rows.len(), 1);
    assert!(json_rows[0].get("usage").is_some());

    let guide = run_cli_stdout(&["--no-env", "--no-config-file", "--guide", "--help"]);
    assert!(guide.contains("Usage"));
    assert!(guide.contains("Commands"));

    let markdown = run_cli_stdout(&["--no-env", "--no-config-file", "--md", "--help"]);
    assert!(markdown.contains("## Usage"));
    assert!(markdown.contains("## Commands"));
    assert!(markdown.contains("- `help` "));
    assert!(!markdown.contains("| name"));

    let table = run_cli_stdout(&["--no-env", "--no-config-file", "--table", "--help"]);
    assert!(table.contains("preamble"));
    assert!(table.contains("usage"));
    assert!(table.contains("commands"));

    let mreg = run_cli_stdout(&["--no-env", "--no-config-file", "--mreg", "--help"]);
    assert!(mreg.contains("preamble:"));
    assert!(mreg.contains("commands ("));

    let value = run_cli_stdout(&["--no-env", "--no-config-file", "--value", "--help"]);
    assert!(value.contains("OSP CLI"));
    assert!(value.contains("osp [OPTIONS] [COMMAND]"));
    assert!(value.contains("Inspect and mutate CLI configuration"));
    assert!(!value.contains("Usage"));
    assert!(!value.contains("Commands"));
}

#[test]
fn intro_command_supports_all_explicit_output_formats_contract() {
    let json = run_cli_stdout(&["--no-env", "--no-config-file", "--json", "intro"]);
    let json_value: serde_json::Value =
        serde_json::from_str(&json).expect("intro json should parse");
    let json_rows = json_value
        .as_array()
        .expect("intro json should be row array");
    assert_eq!(json_rows.len(), 1);
    assert!(json_rows[0].get("sections").is_some());

    let guide = run_cli_stdout(&["--no-env", "--no-config-file", "--guide", "intro"]);
    assert!(guide.contains("OSP"));
    assert!(guide.contains("Commands"));
    let osp = guide.find("OSP").expect("OSP section should render");
    let keybindings = guide
        .find("Keybindings")
        .expect("Keybindings section should render");
    let pipes = guide.find("Pipes").expect("Pipes section should render");
    let usage = guide.find("Usage").expect("Usage section should render");
    let commands = guide
        .find("Commands")
        .expect("Commands section should render");
    assert!(osp < keybindings);
    assert!(keybindings < pipes);
    assert!(pipes < usage);
    assert!(usage < commands);

    let markdown = run_cli_stdout(&["--no-env", "--no-config-file", "--md", "intro"]);
    assert!(markdown.contains("## OSP"));
    assert!(markdown.contains("## Commands"));
    assert!(markdown.contains("- `help` Show this command overview."));
    assert!(!markdown.contains("| name"));

    let table = run_cli_stdout(&["--no-env", "--no-config-file", "--table", "intro"]);
    assert!(table.contains("sections"));

    let mreg = run_cli_stdout(&["--no-env", "--no-config-file", "--mreg", "intro"]);
    assert!(mreg.contains("sections ("));
    assert!(mreg.contains("title:"));

    let value = run_cli_stdout(&["--no-env", "--no-config-file", "--value", "intro"]);
    assert!(value.contains("Welcome"));
    assert!(value.contains("Show this command overview."));
    assert!(value.contains("Inspect and edit runtime config"));
}
