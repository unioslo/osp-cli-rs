use crate::output_support::strip_ansi;
use crate::temp_support::make_temp_dir;
use crate::test_env::isolated_env;

fn run_cli_stdout(args: &[&str]) -> String {
    run_cli_stdout_with_config(None, args)
}

fn run_cli_stdout_with_config(config_toml: Option<&str>, args: &[&str]) -> String {
    let home = make_temp_dir("osp-cli-command-surfaces");
    let mut cmd = crate::test_env::command(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("LANG", "C.UTF-8");
    for (key, value) in isolated_env(home.path()) {
        cmd.env(key, value);
    }
    if let Some(config_toml) = config_toml {
        let config_dir = home.path().join(".config").join("osp");
        std::fs::create_dir_all(&config_dir).expect("config dir should be created");
        let config_path = config_dir.join("config.toml");
        std::fs::write(&config_path, config_toml).expect("config should be written");
        cmd.env("OSP_CONFIG_FILE", config_path);
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
fn alias_catalog_preserves_configured_pipeline_templates_contract() {
    let stdout = run_cli_stdout_with_config(
        Some("[default]\nalias.recent = 'history list | L 5'\n"),
        &[
            "--no-env",
            "--json",
            "alias",
            "list",
            "|",
            "F name=recent",
            "|",
            "P name template",
        ],
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout).unwrap(),
        serde_json::json!([{"name": "recent", "template": "history list | L 5"}])
    );
}

#[test]
fn intro_command_emits_semantic_json_with_explicit_format_contract() {
    let stdout = run_cli_stdout(&["--no-env", "--no-config-file", "--json", "intro"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("intro JSON should parse");
    let blocks = json["blocks"].as_array().expect("document blocks");
    let commands = blocks
        .iter()
        .find(|block| block["section"] == "Commands")
        .unwrap();
    assert!(
        commands["blocks"][0]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "help"
                && entry["description"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty()))
    );
}

#[test]
fn top_level_help_supports_all_explicit_output_formats_contract() {
    let json = run_cli_stdout(&["--no-env", "--no-config-file", "--json", "--help"]);
    let json_value: serde_json::Value =
        serde_json::from_str(&json).expect("help json should parse");
    let blocks = json_value["blocks"].as_array().unwrap();
    let usage = blocks
        .iter()
        .find(|block| block["section"] == "Usage")
        .unwrap();
    assert!(!usage["blocks"][0]["text"].as_str().unwrap().is_empty());

    for args in [
        &["--defaults-only", "--help", "|", "config"][..],
        &["--defaults-only", "help", "|", "config"],
        &["--defaults-only", "help | config"],
    ] {
        let filtered = run_cli_stdout(args);
        assert!(filtered.contains("Inspect and edit runtime config"));
        assert!(!filtered.contains("Inspect and manage plugins"));
    }

    let guide = run_cli_stdout(&["--no-env", "--no-config-file", "--format=auto", "--help"]);
    assert!(guide.contains("Usage"));
    assert!(guide.contains("Commands"));

    let markdown = run_cli_stdout(&["--no-env", "--no-config-file", "--md", "--help"]);
    assert!(markdown.contains("## Usage"));
    assert!(markdown.contains("## Commands"));
    assert!(markdown.contains("- `config` "));
    assert!(!markdown.contains("| name"));

    let table = run_cli_stdout(&["--no-env", "--no-config-file", "--table", "--help"]);
    let mut rows = table.lines().filter(|line| line.starts_with('|'));
    let cells = |line: &str| {
        line.trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        cells(rows.next().unwrap()),
        ["section", "text", "name", "description"]
    );
    assert_eq!(cells(rows.next().unwrap()), ["", "OSP CLI", "", ""]);
    assert_eq!(
        cells(rows.next().unwrap()),
        ["Usage", "osp [OPTIONS] [COMMAND]", "", ""]
    );
    let width = unicode_width::UnicodeWidthStr::width(table.lines().next().unwrap());
    assert!(width > 80);
    assert!(
        table
            .lines()
            .all(|line| unicode_width::UnicodeWidthStr::width(line) == width)
    );

    let mreg = run_cli_stdout(&["--no-env", "--no-config-file", "--mreg", "--help"]);
    assert!(mreg.contains("Usage:\n  text         osp [OPTIONS] [COMMAND]"));
    assert!(mreg.contains("description  Inspect and edit runtime config"));
    assert!(!mreg.lines().any(|line| line.starts_with('|')));

    let value = run_cli_stdout(&["--no-env", "--no-config-file", "--value", "--help"]);
    assert!(value.contains("OSP CLI"));
    assert!(value.contains("osp [OPTIONS] [COMMAND]"));
    assert!(value.contains("Inspect and edit runtime config"));
    assert!(!value.contains("Usage"));
    assert!(!value.contains("Commands"));
}

#[test]
fn intro_command_supports_all_explicit_output_formats_contract() {
    let json = run_cli_stdout(&["--no-env", "--no-config-file", "--json", "intro"]);
    let json_value: serde_json::Value =
        serde_json::from_str(&json).expect("intro json should parse");
    assert!(json_value["blocks"].is_array());

    let guide = run_cli_stdout(&["--no-env", "--no-config-file", "--format=auto", "intro"]);
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

    let table = run_cli_stdout(&["--no-env", "--no-config-file", "--table", "intro"]);
    assert!(table.contains("section"));

    let mreg = run_cli_stdout(&["--no-env", "--no-config-file", "--mreg", "intro"]);
    assert!(mreg.contains("Commands:\n  name         help\n"));
    assert!(mreg.contains("Show this command overview."));

    let value = run_cli_stdout(&["--no-env", "--no-config-file", "--value", "intro"]);
    assert!(value.contains("Welcome"));
    assert!(value.contains("Show this command overview."));
    assert!(value.contains("Inspect and edit runtime config"));
}

#[test]
fn intro_command_compact_and_austere_diverge_by_presentation_contract() {
    let compact = run_cli_stdout(&[
        "--user",
        "anonymous",
        "--no-env",
        "--no-config-file",
        "--presentation",
        "compact",
        "intro",
    ]);
    let austere = run_cli_stdout(&[
        "--user",
        "anonymous",
        "--no-env",
        "--no-config-file",
        "--presentation",
        "austere",
        "intro",
    ]);

    assert!(compact.contains("Usage: [INVOCATION_OPTIONS] COMMAND [ARGS]..."));
    assert!(compact.contains("Commands:"));
    assert!(compact.contains("help"));
    assert!(compact.contains("Show this command overview."));
    assert!(!compact.contains("Welcome anonymous."));

    assert_eq!(
        austere.trim(),
        format!(
            "Welcome anonymous. v{}. Commands: help, last, config, theme. See help for more.",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert!(!austere.contains("Usage:"));
}

#[test]
fn intro_command_shape_matrix_preserves_none_minimal_and_full_chrome_contract() {
    let none = run_cli_stdout_with_config(
        Some(
            r#"[default]
repl.intro = "none"
"#,
        ),
        &["--user", "anonymous", "--no-env", "--value", "intro"],
    );
    assert_eq!(none.trim(), "No results.", "{none:?}");

    let minimal = run_cli_stdout_with_config(
        Some(
            r#"[default]
repl.intro = "minimal"
"#,
        ),
        &["--user", "anonymous", "--no-env", "--value", "intro"],
    );
    assert!(minimal.contains("Welcome anonymous."), "{minimal:?}");
    assert!(minimal.contains("Commands:"), "{minimal:?}");
    assert!(!minimal.contains("Keybindings"), "{minimal:?}");
    assert!(!minimal.contains("Usage"), "{minimal:?}");

    let unicode = run_cli_stdout_with_config(
        Some(
            r#"[default]
repl.intro = "full"
"#,
        ),
        &[
            "--no-env",
            "--render-mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "always",
            "--format=auto",
            "intro",
        ],
    );
    let ascii = run_cli_stdout_with_config(
        Some(
            r#"[default]
repl.intro = "full"
"#,
        ),
        &[
            "--no-env",
            "--render-mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "never",
            "--format=auto",
            "intro",
        ],
    );
    let unicode_plain = strip_ansi(&unicode);
    let ascii_plain = strip_ansi(&ascii);

    assert!(unicode_plain.contains("OSP ─"), "{unicode_plain:?}");
    assert!(unicode_plain.contains("Keybindings"), "{unicode_plain:?}");
    assert!(unicode_plain.contains("Usage"), "{unicode_plain:?}");
    assert!(!unicode_plain.contains("OSP -"), "{unicode_plain:?}");

    assert!(ascii_plain.contains("OSP -"), "{ascii_plain:?}");
    assert!(ascii_plain.contains("Commands -"), "{ascii_plain:?}");
    assert!(ascii_plain.contains("Keybindings"), "{ascii_plain:?}");
    assert!(ascii_plain.contains("Usage"), "{ascii_plain:?}");
    assert!(!ascii_plain.contains('─'), "{ascii_plain:?}");
}

#[test]
fn intro_command_expressive_guide_surfaces_user_context_and_sections_contract() {
    let guide = run_cli_stdout_with_config(
        Some(
            r#"[default]
repl.intro = "full"
ui.presentation = "expressive"
user.name = "oistes"
user.display_names.oistes = "Oistes"
theme.name = "rose-pine-moon"
"#,
        ),
        &["--no-env", "--format=auto", "intro"],
    );

    assert!(guide.contains("OSP"), "{guide:?}");
    assert!(guide.contains("Keybindings"), "{guide:?}");
    assert!(guide.contains("Pipes"), "{guide:?}");
    assert!(guide.contains("Oistes"), "{guide:?}");
    assert!(guide.contains("Not authenticated"), "{guide:?}");
    assert!(guide.contains("Rose Pine Moon"), "{guide:?}");
}

#[test]
fn intro_command_shared_rule_layout_preserves_section_order_contract() {
    let guide = run_cli_stdout_with_config(
        Some(
            r#"[default]
ui.presentation = "expressive"
ui.chrome.frame = "top-bottom"
ui.chrome.rule_policy = "shared"
user.name = "oistes"
theme.name = "rose-pine-moon"
"#,
        ),
        &["--no-env", "--format=auto", "intro"],
    );

    let osp = guide.find("OSP -").expect("OSP section should render");
    let keybindings = guide
        .find("Keybindings -")
        .expect("Keybindings section should render");
    let pipes = guide.find("Pipes -").expect("Pipes section should render");
    let usage = guide.find("Usage:").expect("Usage section should render");
    let commands = guide
        .find("Commands -")
        .expect("Commands section should render");

    assert!(osp < keybindings, "{guide:?}");
    assert!(keybindings < pipes, "{guide:?}");
    assert!(pipes < usage, "{guide:?}");
    assert!(usage < commands, "{guide:?}");
    assert!(
        !guide
            .lines()
            .any(|line| { matches!(line.trim(), "-" | "--" | "─" | "──") }),
        "{guide:?}"
    );
}

#[test]
fn intro_command_expressive_guide_preserves_user_context_and_section_order_contract() {
    let stdout = run_cli_stdout_with_config(
        Some(
            r#"[default]
ui.presentation = "expressive"
repl.intro = "full"
repl.simple_prompt = true
user.display_names.oistes = "Demo"
user.name = "oistes"
theme.name = "rose-pine-moon"
"#,
        ),
        &["--no-env", "--format=auto", "intro"],
    );

    assert!(stdout.contains("Welcome Demo!"), "{stdout}");
    assert!(stdout.contains("User: Not authenticated"), "{stdout}");
    assert!(stdout.contains("Theme: Rose Pine Moon"), "{stdout}");
    assert!(stdout.contains("Show this command overview."), "{stdout}");
    assert!(stdout.contains("help"), "{stdout}");

    let osp = stdout.find("OSP").expect("OSP section should render");
    let keybindings = stdout
        .find("Keybindings")
        .expect("Keybindings section should render");
    let pipes = stdout.find("Pipes").expect("Pipes section should render");
    let usage = stdout.find("Usage").expect("Usage section should render");
    let commands = stdout
        .find("Commands")
        .expect("Commands section should render");
    assert!(osp < keybindings);
    assert!(keybindings < pipes);
    assert!(pipes < usage);
    assert!(usage < commands);
}

#[test]
fn intro_command_rich_guide_respects_color_and_unicode_contract() {
    let config = r#"[default]
ui.presentation = "expressive"
repl.intro = "full"
repl.simple_prompt = true
"#;

    let unicode = run_cli_stdout_with_config(
        Some(config),
        &[
            "--no-env",
            "--format=auto",
            "--render-mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "always",
            "intro",
        ],
    );
    let ascii = run_cli_stdout_with_config(
        Some(config),
        &[
            "--no-env",
            "--format=auto",
            "--render-mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "never",
            "intro",
        ],
    );

    assert!(unicode.contains("\u{1b}["), "{unicode:?}");

    let unicode_plain = strip_ansi(&unicode);
    let ascii_plain = strip_ansi(&ascii);
    assert!(unicode_plain.contains("OSP ─"), "{unicode_plain:?}");
    assert!(unicode_plain.contains("Ctrl-D"), "{unicode_plain:?}");
    assert!(ascii_plain.contains("OSP -"), "{ascii_plain:?}");
    assert!(ascii_plain.contains("Ctrl-D"), "{ascii_plain:?}");
    assert!(!ascii_plain.contains('─'), "{ascii_plain:?}");
}
