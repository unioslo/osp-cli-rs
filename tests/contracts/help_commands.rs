use assert_cmd::Command;
use insta::assert_snapshot;

#[cfg(unix)]
fn fixture_config(path: &std::path::Path) {
    std::fs::write(
        path,
        r#"
[default]
theme.name = "plain"
"color.panel.border" = "red"
"color.panel.title" = "green"
"color.key" = "yellow"
"color.table.header" = "blue"
"color.message.success" = "bright-green"
"color.message.warning" = "yellow"
"color.message.error" = "red"
"color.message.info" = "blue"
"#,
    )
    .expect("fixture config should be written");
}

#[cfg(unix)]
fn run_with_config(config_path: &std::path::Path, args: &[&str]) -> String {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("PATH", "/usr/bin:/bin")
        .env("OSP_CONFIG_FILE", config_path)
        .env_remove("NO_COLOR")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("output should be utf-8")
}

#[cfg(unix)]
fn run_with_config_stderr(
    config_path: &std::path::Path,
    xdg_config_home: Option<&std::path::Path>,
    expect_success: bool,
    args: &[&str],
) -> String {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("PATH", "/usr/bin:/bin")
        .env("OSP_CONFIG_FILE", config_path)
        .env_remove("NO_COLOR");
    if let Some(path) = xdg_config_home {
        cmd.env("XDG_CONFIG_HOME", path);
    }
    let assert = cmd.args(args).assert();
    let output = if expect_success {
        assert.success().get_output().stderr.clone()
    } else {
        assert.failure().get_output().stderr.clone()
    };
    String::from_utf8(output).expect("stderr output should be utf-8")
}

#[cfg(unix)]
fn run_with_config_tty(config_path: &std::path::Path, args: &[&str]) -> String {
    let bin = assert_cmd::cargo::cargo_bin!("osp");
    let command = format!(
        "env -u NO_COLOR PATH=/usr/bin:/bin TERM=xterm-256color OSP_CONFIG_FILE={} {} {}",
        config_path.display(),
        bin.display(),
        args.join(" ")
    );
    let mut cmd = Command::new("script");
    let output = cmd
        .args(["-qfec", &command, "/dev/null"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("tty output should be utf-8")
}

#[cfg(unix)]
fn help_output(
    config_path: &std::path::Path,
    presentation: &str,
    color: &str,
    unicode: &str,
) -> String {
    run_with_config(
        config_path,
        &[
            "--no-env",
            "--mode",
            "rich",
            "--presentation",
            presentation,
            "--color",
            color,
            "--unicode",
            unicode,
            "--help",
        ],
    )
}

#[cfg(unix)]
fn table_output(
    config_path: &std::path::Path,
    presentation: &str,
    color: &str,
    unicode: &str,
) -> String {
    run_with_config(
        config_path,
        &[
            "--no-env",
            "--mode",
            "rich",
            "--presentation",
            presentation,
            "--color",
            color,
            "--unicode",
            unicode,
            "theme",
            "list",
        ],
    )
}

#[cfg(unix)]
fn success_info_output(
    config_path: &std::path::Path,
    presentation: &str,
    color: &str,
    unicode: &str,
) -> String {
    run_with_config_stderr(
        config_path,
        None,
        true,
        &[
            "--no-env",
            "--mode",
            "rich",
            "--presentation",
            presentation,
            "--color",
            color,
            "--unicode",
            unicode,
            "-v",
            "theme",
            "use",
            "plain",
        ],
    )
}

#[cfg(unix)]
fn warning_success_output(
    config_path: &std::path::Path,
    xdg_config_home: &std::path::Path,
    presentation: &str,
    color: &str,
    unicode: &str,
) -> String {
    run_with_config_stderr(
        config_path,
        Some(xdg_config_home),
        true,
        &[
            "--no-env",
            "--mode",
            "rich",
            "--presentation",
            presentation,
            "--color",
            color,
            "--unicode",
            unicode,
            "config",
            "set",
            "--config",
            "ui.prompt.secrets",
            "true",
        ],
    )
}

#[cfg(unix)]
fn error_output(
    config_path: &std::path::Path,
    presentation: &str,
    color: &str,
    unicode: &str,
) -> String {
    run_with_config_stderr(
        config_path,
        None,
        false,
        &[
            "--no-env",
            "--mode",
            "rich",
            "--presentation",
            presentation,
            "--color",
            color,
            "--unicode",
            unicode,
            "config",
            "get",
            "missing.key",
        ],
    )
}

#[cfg(unix)]
fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            chars.next();
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

#[cfg(unix)]
#[test]
fn help_color_unicode_presentation_matrix_contract() {
    let home = make_temp_dir("osp-cli-help-matrix");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let cases = [
        ("expressive", "always", "always"),
        ("expressive", "always", "never"),
        ("expressive", "never", "always"),
        ("expressive", "never", "never"),
        ("compact", "always", "always"),
        ("compact", "always", "never"),
        ("compact", "never", "always"),
        ("compact", "never", "never"),
        ("austere", "always", "always"),
        ("austere", "always", "never"),
        ("austere", "never", "always"),
        ("austere", "never", "never"),
    ];

    for (presentation, color, unicode) in cases {
        let output = help_output(&config_path, presentation, color, unicode);
        let plain = strip_ansi(&output);

        assert!(
            plain.contains("Usage"),
            "missing usage for {presentation}/{color}/{unicode}: {plain:?}"
        );

        if color == "always" {
            assert!(
                output.contains("\x1b[32mUsage"),
                "missing title color for {presentation}/{color}/{unicode}: {output:?}"
            );
        } else {
            assert!(
                !output.contains("\x1b["),
                "unexpected ANSI for {presentation}/{color}/{unicode}: {output:?}"
            );
        }

        match presentation {
            "austere" => {
                assert!(
                    plain.contains("Usage:\n  osp [OPTIONS] [COMMAND]"),
                    "austere help should stay borderless for {color}/{unicode}: {plain:?}"
                );
                assert!(
                    !output.contains("\x1b[31m"),
                    "austere help should not use border color for {color}/{unicode}: {output:?}"
                );
            }
            _ => {
                if color == "always" && unicode == "always" {
                    assert!(
                        output.contains("\x1b[31m─ "),
                        "missing unicode border color for {presentation}: {output:?}"
                    );
                } else if color == "always" && unicode == "never" {
                    assert!(
                        output.contains("\x1b[31m- "),
                        "missing ascii border color for {presentation}: {output:?}"
                    );
                }

                if unicode == "always" {
                    assert!(
                        plain.contains("─ Usage"),
                        "missing unicode chrome for {presentation}/{color}: {plain:?}"
                    );
                } else {
                    assert!(
                        plain.contains("- Usage"),
                        "missing ascii chrome for {presentation}/{color}: {plain:?}"
                    );
                }
            }
        }
    }

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn table_color_unicode_presentation_matrix_contract() {
    let home = make_temp_dir("osp-cli-table-matrix");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let cases = [
        ("expressive", "always", "always"),
        ("expressive", "always", "never"),
        ("expressive", "never", "always"),
        ("expressive", "never", "never"),
        ("compact", "always", "always"),
        ("compact", "always", "never"),
        ("compact", "never", "always"),
        ("compact", "never", "never"),
        ("austere", "always", "always"),
        ("austere", "always", "never"),
        ("austere", "never", "always"),
        ("austere", "never", "never"),
    ];

    for (presentation, color, unicode) in cases {
        let output = table_output(&config_path, presentation, color, unicode);
        let plain = strip_ansi(&output);

        assert!(
            plain.contains("id") && plain.contains("name"),
            "missing table headers for {presentation}/{color}/{unicode}: {plain:?}"
        );

        if color == "always" {
            assert!(
                output.contains("\x1b[34mid\x1b[0m"),
                "missing header color for {presentation}/{color}/{unicode}: {output:?}"
            );
        } else {
            assert!(
                !output.contains("\x1b["),
                "unexpected ANSI for {presentation}/{color}/{unicode}: {output:?}"
            );
        }

        if unicode == "always" {
            let expected_corner = if presentation == "expressive" {
                '╭'
            } else {
                '┏'
            };
            assert!(
                plain.starts_with(expected_corner),
                "unexpected unicode table chrome for {presentation}/{color}: {plain:?}"
            );
        } else {
            assert!(
                plain.starts_with('+'),
                "unexpected ascii table chrome for {presentation}/{color}: {plain:?}"
            );
        }
    }

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn command_help_hides_common_invocation_options_without_verbose_contract() {
    let home = make_temp_dir("osp-cli-help-history-default");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let output = run_with_config(&config_path, &["--no-env", "history", "--help"]);
    let plain = strip_ansi(&output);

    assert!(plain.contains("history"));
    assert!(!plain.contains("Common Invocation Options"));
    assert_snapshot!("history_help_default", plain);

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn command_help_shows_common_invocation_options_with_verbose_contract() {
    let home = make_temp_dir("osp-cli-help-history-verbose");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let output = run_with_config(&config_path, &["--no-env", "history", "--help", "-v"]);
    let plain = strip_ansi(&output);

    assert!(plain.contains("history"));
    assert!(plain.contains("Common Invocation Options"));
    assert_snapshot!("history_help_verbose", plain);

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn message_color_unicode_presentation_matrix_contract() {
    let home = make_temp_dir("osp-cli-message-matrix");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);

    let cases = [
        ("expressive", "always", "always"),
        ("expressive", "always", "never"),
        ("expressive", "never", "always"),
        ("expressive", "never", "never"),
        ("compact", "always", "always"),
        ("compact", "always", "never"),
        ("compact", "never", "always"),
        ("compact", "never", "never"),
        ("austere", "always", "always"),
        ("austere", "always", "never"),
        ("austere", "never", "always"),
        ("austere", "never", "never"),
    ];

    for (index, (presentation, color, unicode)) in cases.into_iter().enumerate() {
        let xdg_config_home = home.join(format!("xdg-{index}"));
        std::fs::create_dir_all(&xdg_config_home).expect("xdg config home should be created");

        let success_info = success_info_output(&config_path, presentation, color, unicode);
        let success_plain = strip_ansi(&success_info);
        assert!(
            success_plain.contains("active theme set to: plain"),
            "missing success body for {presentation}/{color}/{unicode}: {success_plain:?}"
        );
        assert!(
            success_plain.contains("theme change is for the current process"),
            "missing info body for {presentation}/{color}/{unicode}: {success_plain:?}"
        );

        let warning_success =
            warning_success_output(&config_path, &xdg_config_home, presentation, color, unicode);
        let warning_plain = strip_ansi(&warning_success);
        assert!(
            warning_plain.contains("writing a sensitive key to config store; prefer --secrets"),
            "missing warning body for {presentation}/{color}/{unicode}: {warning_plain:?}"
        );
        assert!(
            warning_plain.contains("set value for ui.prompt.secrets"),
            "missing success body for {presentation}/{color}/{unicode}: {warning_plain:?}"
        );

        let error = error_output(&config_path, presentation, color, unicode);
        let error_plain = strip_ansi(&error);
        assert!(
            error_plain.contains("config key not found: missing.key"),
            "missing error body for {presentation}/{color}/{unicode}: {error_plain:?}"
        );

        if color == "always" {
            if presentation == "austere" {
                assert!(
                    success_info.contains("\x1b[92msuccess: "),
                    "missing austere success color for {unicode}: {success_info:?}"
                );
                assert!(
                    success_info.contains("\x1b[34minfo: "),
                    "missing austere info color for {unicode}: {success_info:?}"
                );
                assert!(
                    warning_success.contains("\x1b[33mwarning: "),
                    "missing austere warning color for {unicode}: {warning_success:?}"
                );
                assert!(
                    error.contains("\x1b[31merror: "),
                    "missing austere error color for {unicode}: {error:?}"
                );
            } else {
                assert!(
                    success_info.contains("\x1b[92m") && success_info.contains(" Success "),
                    "missing grouped success color for {presentation}/{unicode}: {success_info:?}"
                );
                assert!(
                    success_info.contains("\x1b[34m") && success_info.contains(" Info "),
                    "missing grouped info color for {presentation}/{unicode}: {success_info:?}"
                );
                assert!(
                    warning_success.contains("\x1b[33m") && warning_success.contains(" Warnings "),
                    "missing grouped warning color for {presentation}/{unicode}: {warning_success:?}"
                );
                assert!(
                    error.contains("\x1b[31m") && error.contains(" Errors "),
                    "missing grouped error color for {presentation}/{unicode}: {error:?}"
                );
            }
        } else {
            assert!(!success_info.contains("\x1b["));
            assert!(!warning_success.contains("\x1b["));
            assert!(!error.contains("\x1b["));
        }

        match presentation {
            "austere" => {
                assert!(
                    success_plain.contains("success: active theme set to: plain"),
                    "austere success should be minimal for {color}/{unicode}: {success_plain:?}"
                );
                assert!(
                    warning_plain.contains("warning: writing a sensitive key"),
                    "austere warning should be minimal for {color}/{unicode}: {warning_plain:?}"
                );
                assert!(
                    error_plain.contains("error: config key not found: missing.key"),
                    "austere error should be minimal for {color}/{unicode}: {error_plain:?}"
                );
            }
            _ => {
                let rule = if unicode == "always" { '─' } else { '-' };
                assert!(
                    success_plain.contains(rule) && success_plain.contains("Success"),
                    "grouped success should keep framed chrome for {presentation}/{color}/{unicode}: {success_plain:?}"
                );
                assert!(
                    warning_plain.contains(rule) && warning_plain.contains("Warnings"),
                    "grouped warning should keep framed chrome for {presentation}/{color}/{unicode}: {warning_plain:?}"
                );
                assert!(
                    error_plain.contains(rule) && error_plain.contains("Errors"),
                    "grouped error should keep framed chrome for {presentation}/{color}/{unicode}: {error_plain:?}"
                );
            }
        }
    }

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn tty_subcommand_help_keeps_help_chrome_colors_contract() {
    let dir = make_temp_dir("osp-cli-help-tty");
    let config_path = dir.join("config.toml");
    fixture_config(&config_path);

    let output = run_with_config_tty(&config_path, &["history", "--help"]);

    assert!(output.contains("\u{1b}[32mUsage\u{1b}[0m"));
    assert!(output.contains("\u{1b}[33mlist\u{1b}[0m"));
    assert!(output.contains("\u{1b}[33m-h, --help\u{1b}[0m"));
    let _ = std::fs::remove_dir_all(&dir);
}
