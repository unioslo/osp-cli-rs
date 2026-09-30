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

    assert!(output.contains("\u{1b}["));
    let plain = strip_ansi(&output);
    assert!(plain.contains("Usage"));
    assert!(plain.contains("list"));
    assert!(plain.contains("-h, --help"));
}

#[cfg(unix)]
#[test]
fn generated_bash_completion_supports_builtin_command_navigation_contract() {
    let home = make_temp_dir("osp-cli-bash-completion");
    let config_path = home.join("config.toml");
    fixture_config(&config_path);
    let script_path = home.join("osp.bash");
    std::fs::write(
        &script_path,
        run_with_config(&config_path, &["--no-env", "completions", "bash"]),
    )
    .expect("exported completion script should be written");

    let consumer = r#"
set -e
source "$1"
read -r -a registration <<< "$(complete -p osp)"
completion=
for ((i = 0; i < ${#registration[@]}; i++)); do
    if [[ ${registration[i]} == -F ]]; then
        completion=${registration[i + 1]}
        break
    fi
done
test -n "$completion"

candidates() {
    local scenario=$1
    shift
    COMP_WORDS=("$@")
    COMP_CWORD=$((${#COMP_WORDS[@]} - 1))
    "$completion" "${COMP_WORDS[0]}" "${COMP_WORDS[COMP_CWORD]}" "${COMP_WORDS[COMP_CWORD - 1]}"
    for candidate in "${COMPREPLY[@]}"; do
        printf '%s\t%s\n' "$scenario" "$candidate"
    done
}
candidates root osp conf
candidates command osp config g
candidates flag osp config get --s
candidates value osp config get --presentation c
"#;
    let output = Command::new("bash")
        .env_clear()
        .envs(crate::test_env::isolated_env(home.path()))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--noprofile",
            "--norc",
            "-c",
            consumer,
            "osp-completion-consumer",
        ])
        .arg(&script_path)
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).expect("completion candidates should be UTF-8");
    let candidates: Vec<_> = stdout
        .lines()
        .map(|line| {
            line.split_once('\t')
                .expect("candidate should identify its context")
        })
        .collect();
    assert_eq!(
        candidates,
        [
            ("root", "config"),
            ("command", "get"),
            ("flag", "--sources"),
            ("value", "compact"),
        ]
    );
}
