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
