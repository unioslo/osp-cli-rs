use super::{ArgDef, CommandDef, CommandPolicyDef, FlagDef, ValueKind};
use crate::core::command_policy::VisibilityMode;

#[test]
fn builder_methods_shape_command_definitions_unit() {
    let policy = CommandPolicyDef {
        visibility: VisibilityMode::Authenticated,
        required_capabilities: vec!["plugins.write".to_string()],
        feature_flags: vec!["beta".to_string()],
    };

    let def = CommandDef::new("plugins")
        .about("Manage plugins")
        .long_about("Long plugin help")
        .usage("osp plugins [OPTIONS]")
        .before_help("before text")
        .after_help("after text")
        .alias("plug")
        .aliases(["pl", "plugins2"])
        .hidden()
        .sort("10")
        .policy(policy.clone())
        .arg(ArgDef::new("name"))
        .args([ArgDef::new("scope")])
        .flag(FlagDef::new("raw"))
        .flags([FlagDef::new("json")])
        .subcommand(CommandDef::new("list"))
        .subcommands([CommandDef::new("enable"), CommandDef::new("disable")]);

    assert_eq!(def.name, "plugins");
    assert_eq!(def.about.as_deref(), Some("Manage plugins"));
    assert_eq!(def.long_about.as_deref(), Some("Long plugin help"));
    assert_eq!(def.usage.as_deref(), Some("osp plugins [OPTIONS]"));
    assert_eq!(def.before_help.as_deref(), Some("before text"));
    assert_eq!(def.after_help.as_deref(), Some("after text"));
    assert_eq!(
        def.aliases,
        vec!["plug".to_string(), "pl".to_string(), "plugins2".to_string()]
    );
    assert!(def.hidden);
    assert_eq!(def.sort_key.as_deref(), Some("10"));
    assert_eq!(def.policy, policy);
    assert_eq!(def.args.len(), 2);
    assert_eq!(def.flags.len(), 2);
    assert_eq!(
        def.subcommands
            .iter()
            .map(|command| command.name.as_str())
            .collect::<Vec<_>>(),
        vec!["list", "enable", "disable"]
    );
}

#[cfg(feature = "clap")]
#[test]
fn clap_command_def_caches_usage_aliases_and_help_unit() {
    use clap::{Arg, Command};

    let command = Command::new("theme")
        .about("Inspect themes")
        .visible_alias("skins")
        .before_help("before text")
        .after_help("after text")
        .arg(
            Arg::new("name")
                .help("Theme name")
                .value_hint(clap::ValueHint::DirPath),
        )
        .arg(
            Arg::new("raw")
                .long("raw")
                .visible_alias("plain")
                .help("Show raw values"),
        );

    let def = CommandDef::from_clap(command);

    assert_eq!(def.usage.as_deref(), Some("theme [OPTIONS] [name]"));
    assert_eq!(def.aliases, vec!["skins".to_string()]);
    assert_eq!(def.before_help.as_deref(), Some("before text"));
    assert_eq!(def.after_help.as_deref(), Some("after text"));
    assert_eq!(def.args[0].value_kind, Some(ValueKind::Path));
    assert!(def.flags[0].aliases.contains(&"--plain".to_string()));
}
