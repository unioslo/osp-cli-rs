use crate::temp_support::make_temp_dir;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_basic_repl(input: &[u8]) -> Output {
    let home = make_temp_dir("osp-cli-repl-basic-home");

    Command::new(env!("CARGO_BIN_EXE_osp"))
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("XDG_CACHE_HOME", home.path().join(".cache"))
        .env("XDG_STATE_HOME", home.path().join(".local/state"))
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .arg("--defaults-only")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .as_mut()
                .expect("stdin should be piped")
                .write_all(input)?;
            child.wait_with_output()
        })
        .expect("basic repl should run")
}

#[test]
fn repl_basic_mode_runs_help_and_exit_without_tty() {
    let files = make_temp_dir("osp-cli-repl-source");
    let commands = files.path().join("session commands.osp");
    std::fs::write(
        &commands,
        "# Read config through the same session\n\nconfig get theme.name\n",
    )
    .expect("command file should write");
    let first = files.path().join("first result.osp");
    let second = files.path().join("second result.osp");
    std::fs::write(&first, "theme show dracula | P id,name\n").unwrap();
    std::fs::write(&second, "theme show nord | P id,name\n").unwrap();
    let input = format!(
        "help\nsource '{}'\n!!\nconfig set --session ui.format json\nsource -- '{}' '{}'\nlast\nlast --raw\nexit\n",
        commands.display(),
        first.display(),
        second.display(),
    );
    let output = run_basic_repl(input.as_bytes());
    assert!(
        output.status.success(),
        "basic repl should exit successfully; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("default> "));
    assert!(stdout.contains("Commands"));
    assert!(stdout.contains("help"));
    assert!(stdout.contains("exit"));
    assert!(
        stdout.matches("rose-pine-moon").count() == 2,
        "source and recall should each execute the resolved config command; stdout:\n{stdout}"
    );
    assert!(stderr.contains("Warning: input is not a terminal"));

    let mut documents = Vec::new();
    let mut remaining = stdout.as_ref();
    while let Some(start) = remaining.find("[\n") {
        let mut document = serde_json::Deserializer::from_str(&remaining[start..])
            .into_iter::<serde_json::Value>();
        documents.push(
            document
                .next()
                .unwrap()
                .expect("each sourced or replayed result should be JSON"),
        );
        remaining = &remaining[start + document.byte_offset()..];
    }
    assert_eq!(
        documents[0],
        serde_json::json!([{"id": "dracula", "name": "Dracula"}])
    );
    let selected = serde_json::json!([{"id": "nord", "name": "Nord"}]);
    assert_eq!(documents[1], selected);
    assert_eq!(
        documents[2], selected,
        "last should replay the saved pipeline"
    );
    assert_eq!(documents[3][0]["id"], "nord");
    assert_eq!(documents[3][0]["name"], "Nord");
    assert_eq!(
        documents[3][0]["accent"], "#88c0d0",
        "raw replay should retain the theme palette"
    );
}

#[test]
fn repl_basic_mode_exits_cleanly_on_immediate_eof() {
    let output = run_basic_repl(b"");
    assert!(
        output.status.success(),
        "basic repl should exit cleanly on EOF; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("default> "));
    assert!(stderr.contains("Warning: input is not a terminal"));
}

#[test]
fn repl_basic_mode_restarts_after_refresh_without_tty() {
    let files = make_temp_dir("osp-cli-repl-source-refresh");
    let commands = files.path().join("refresh.osp");
    std::fs::write(&commands, "plugins refresh\n").unwrap();
    let input = format!(
        "source '{}'\ntheme show dracula --json\nexit\n",
        commands.display()
    );
    let output = run_basic_repl(input.as_bytes());
    assert!(
        output.status.success(),
        "basic repl restart path should succeed; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let prompt_count = stdout.matches("default> ").count();
    let warning_count = stderr.matches("Warning: input is not a terminal").count();

    assert!(
        prompt_count >= 2,
        "expected refresh to restart and render the prompt again; stdout:\n{stdout}"
    );
    assert!(
        warning_count >= 1,
        "expected basic-mode fallback warning; stderr:\n{stderr}"
    );
    assert!(stderr.contains(&format!(
        "{}:1: command requires a REPL restart",
        commands.display()
    )));
    let start = stdout
        .find("[\n")
        .expect("command after source reload should return JSON");
    let theme = serde_json::Deserializer::from_str(&stdout[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(theme[0]["id"], "dracula");
    assert_eq!(theme[0]["name"], "Dracula");
}
