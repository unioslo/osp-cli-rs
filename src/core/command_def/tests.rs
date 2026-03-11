use super::{CommandDef, ValueKind};

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
