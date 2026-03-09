use clap::CommandFactory;
use osp_cli::cli::Cli;
use std::collections::BTreeSet;

fn set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn subcommand_names(cmd: &clap::Command) -> BTreeSet<String> {
    cmd.get_subcommands()
        .map(|sub| sub.get_name().to_string())
        .filter(|name| !is_ignored_subcommand(name))
        .collect()
}

fn is_ignored_subcommand(name: &str) -> bool {
    matches!(name, "help" | "completion" | "external" | "repl" | "intro")
}

fn assert_subcommands(cmd: &clap::Command, name: &str, allowed: BTreeSet<String>) {
    let sub = cmd
        .get_subcommands()
        .find(|sub| sub.get_name() == name)
        .unwrap_or_else(|| panic!("missing subcommand: {name}"));
    let actual = subcommand_names(sub);
    assert_eq!(actual, allowed, "unexpected subcommands for `{name}`");
}

#[test]
fn builtin_command_verbs_are_consistent() {
    let cmd = Cli::command();
    let top_level = subcommand_names(&cmd);
    let allowed_top = set(&["plugins", "doctor", "theme", "config", "history"]);
    assert_eq!(
        top_level, allowed_top,
        "top-level commands should be nouns only"
    );

    assert_subcommands(
        &cmd,
        "plugins",
        set(&[
            "list",
            "commands",
            "config",
            "refresh",
            "enable",
            "disable",
            "clear-state",
            "select-provider",
            "clear-provider",
            "doctor",
        ]),
    );
    assert_subcommands(&cmd, "theme", set(&["list", "show", "use"]));
    assert_subcommands(
        &cmd,
        "config",
        set(&["show", "get", "explain", "set", "unset", "doctor"]),
    );
    assert_subcommands(&cmd, "history", set(&["list", "prune", "clear"]));
    // doctor uses selectors rather than verbs.
    assert_subcommands(
        &cmd,
        "doctor",
        set(&["config", "plugins", "theme", "all", "last"]),
    );

    for name in top_level {
        assert_eq!(
            name,
            name.to_ascii_lowercase(),
            "command `{name}` should be lowercase"
        );
    }
}
