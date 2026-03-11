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
    assert_contract_snapshot!("history_help_default", plain);

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
    assert_contract_snapshot!("history_help_verbose", plain);

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
}
