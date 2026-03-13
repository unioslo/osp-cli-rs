use crate::temp_support::make_temp_dir;
use crate::test_env::isolated_env;
use assert_cmd::Command;

fn run_cli_stdout(args: &[&str]) -> String {
    run_cli_stdout_with_config(None, args)
}

fn run_cli_stdout_with_config(config_toml: Option<&str>, args: &[&str]) -> String {
    let home = make_temp_dir("osp-cli-command-surfaces");
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
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

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            let _ = chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }

    out
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
    assert!(compact.contains("Show this command overview."));
    assert!(!compact.contains("Welcome anonymous."));

    assert_eq!(
        austere.trim(),
        format!(
            "Welcome anonymous. v{}. Commands: help, config, theme, plugins. See help for more.",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert!(!austere.contains("Usage:"));
}

#[test]
fn intro_command_switches_to_completion_hint_when_help_is_hidden_contract() {
    let stdout = run_cli_stdout_with_config(
        Some(
            r#"[default]
ui.presentation = "austere"
auth.visible.builtins = "config,theme,plugins"
"#,
        ),
        &["--user", "anonymous", "--no-env", "intro"],
    );

    assert!(stdout.contains("Use completion to explore commands."));
    assert!(!stdout.contains("See help for more."));
}

#[test]
fn intro_command_hides_protected_builtins_from_visible_intro_commands_contract() {
    let stdout = run_cli_stdout_with_config(
        Some(
            r#"[default]
ui.presentation = "compact"
auth.visible.builtins = "help"
"#,
        ),
        &["--user", "anonymous", "--no-env", "intro"],
    );

    assert!(stdout.contains("Commands:"), "{stdout:?}");
    assert!(stdout.contains("help"), "{stdout:?}");
    assert!(!stdout.contains("config"), "{stdout:?}");
    assert!(!stdout.contains("theme"), "{stdout:?}");
    assert!(!stdout.contains("plugins"), "{stdout:?}");
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
    assert!(none.trim().is_empty(), "{none:?}");

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
            "--mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "always",
            "--guide",
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
            "--mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "never",
            "--guide",
            "intro",
        ],
    );
    let unicode_plain = strip_ansi(&unicode);
    let ascii_plain = strip_ansi(&ascii);

    assert!(unicode_plain.contains("─ OSP "), "{unicode_plain:?}");
    assert!(unicode_plain.contains("Keybindings"), "{unicode_plain:?}");
    assert!(unicode_plain.contains("Usage"), "{unicode_plain:?}");
    assert!(!unicode_plain.contains("- OSP "), "{unicode_plain:?}");

    assert!(ascii_plain.contains("- OSP "), "{ascii_plain:?}");
    assert!(ascii_plain.contains("- Commands "), "{ascii_plain:?}");
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
user.display_name = "Oistes"
theme.name = "rose-pine-moon"
"#,
        ),
        &["--no-env", "--guide", "intro"],
    );

    assert!(guide.contains("OSP"), "{guide:?}");
    assert!(guide.contains("Keybindings"), "{guide:?}");
    assert!(guide.contains("Pipes"), "{guide:?}");
    assert!(guide.contains("Oistes"), "{guide:?}");
    assert!(guide.contains("oistes"), "{guide:?}");
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
        &["--no-env", "--guide", "intro"],
    );

    let osp = guide.find("- OSP ").expect("OSP section should render");
    let keybindings = guide
        .find("- Keybindings ")
        .expect("Keybindings section should render");
    let pipes = guide.find("- Pipes ").expect("Pipes section should render");
    let usage = guide.find("- Usage ").expect("Usage section should render");
    let commands = guide
        .find("- Commands ")
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
user.display_name = "Demo"
user.name = "oistes"
theme.name = "rose-pine-moon"
"#,
        ),
        &["--no-env", "--guide", "intro"],
    );

    assert!(stdout.contains("Welcome Demo!"), "{stdout}");
    assert!(stdout.contains("Logged in as  oistes"), "{stdout}");
    assert!(stdout.contains("Theme         Rose Pine Moon"), "{stdout}");
    assert!(stdout.contains("Show this command overview."), "{stdout}");

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
"color.panel.border" = "red"
"color.panel.title" = "blue"
"color.key" = "yellow"
"color.value" = "green"
"#;

    let unicode = run_cli_stdout_with_config(
        Some(config),
        &[
            "--no-env",
            "--guide",
            "--mode",
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
            "--guide",
            "--mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "never",
            "intro",
        ],
    );

    assert!(unicode.contains("\u{1b}[31m"), "{unicode:?}");
    assert!(unicode.contains("\u{1b}[34mOSP"), "{unicode:?}");
    assert!(unicode.contains("\u{1b}[33mCtrl-D"), "{unicode:?}");
    assert!(unicode.contains("\u{1b}[32m  Welcome "), "{unicode:?}");

    let unicode_plain = strip_ansi(&unicode);
    let ascii_plain = strip_ansi(&ascii);
    assert!(unicode_plain.contains("─ OSP "), "{unicode_plain:?}");
    assert!(unicode_plain.contains("Ctrl-D"), "{unicode_plain:?}");
    assert!(ascii_plain.contains("- OSP "), "{ascii_plain:?}");
    assert!(ascii_plain.contains("Ctrl-D"), "{ascii_plain:?}");
    assert!(!ascii_plain.contains('─'), "{ascii_plain:?}");
}
