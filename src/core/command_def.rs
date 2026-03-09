use crate::core::command_policy::VisibilityMode;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandDef {
    pub name: String,
    pub about: Option<String>,
    pub long_about: Option<String>,
    pub usage: Option<String>,
    pub before_help: Option<String>,
    pub after_help: Option<String>,
    pub aliases: Vec<String>,
    pub hidden: bool,
    pub sort_key: Option<String>,
    pub policy: CommandPolicyDef,
    pub args: Vec<ArgDef>,
    pub flags: Vec<FlagDef>,
    pub subcommands: Vec<CommandDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPolicyDef {
    pub visibility: VisibilityMode,
    pub required_capabilities: Vec<String>,
    pub feature_flags: Vec<String>,
}

impl Default for CommandPolicyDef {
    fn default() -> Self {
        Self {
            visibility: VisibilityMode::Public,
            required_capabilities: Vec::new(),
            feature_flags: Vec::new(),
        }
    }
}

impl CommandPolicyDef {
    pub fn is_empty(&self) -> bool {
        self.visibility == VisibilityMode::Public
            && self.required_capabilities.is_empty()
            && self.feature_flags.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArgDef {
    pub id: String,
    pub value_name: Option<String>,
    pub help: Option<String>,
    pub help_heading: Option<String>,
    pub required: bool,
    pub multi: bool,
    pub value_kind: Option<ValueKind>,
    pub choices: Vec<ValueChoice>,
    pub defaults: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlagDef {
    pub id: String,
    pub short: Option<char>,
    pub long: Option<String>,
    pub aliases: Vec<String>,
    pub help: Option<String>,
    pub help_heading: Option<String>,
    pub takes_value: bool,
    pub value_name: Option<String>,
    pub required: bool,
    pub multi: bool,
    pub hidden: bool,
    pub value_kind: Option<ValueKind>,
    pub choices: Vec<ValueChoice>,
    pub defaults: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Path,
    Enum,
    FreeText,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValueChoice {
    pub value: String,
    pub help: Option<String>,
    pub display: Option<String>,
    pub sort_key: Option<String>,
}

impl CommandDef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = Some(about.into());
        self
    }

    pub fn long_about(mut self, long_about: impl Into<String>) -> Self {
        self.long_about = Some(long_about.into());
        self
    }

    pub fn usage(mut self, usage: impl Into<String>) -> Self {
        self.usage = Some(usage.into());
        self
    }

    pub fn before_help(mut self, text: impl Into<String>) -> Self {
        self.before_help = Some(text.into());
        self
    }

    pub fn after_help(mut self, text: impl Into<String>) -> Self {
        self.after_help = Some(text.into());
        self
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn aliases(mut self, aliases: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    pub fn sort(mut self, sort_key: impl Into<String>) -> Self {
        self.sort_key = Some(sort_key.into());
        self
    }

    pub fn policy(mut self, policy: CommandPolicyDef) -> Self {
        self.policy = policy;
        self
    }

    pub fn arg(mut self, arg: ArgDef) -> Self {
        self.args.push(arg);
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = ArgDef>) -> Self {
        self.args.extend(args);
        self
    }

    pub fn flag(mut self, flag: FlagDef) -> Self {
        self.flags.push(flag);
        self
    }

    pub fn flags(mut self, flags: impl IntoIterator<Item = FlagDef>) -> Self {
        self.flags.extend(flags);
        self
    }

    pub fn subcommand(mut self, subcommand: CommandDef) -> Self {
        self.subcommands.push(subcommand);
        self
    }

    pub fn subcommands(mut self, subcommands: impl IntoIterator<Item = CommandDef>) -> Self {
        self.subcommands.extend(subcommands);
        self
    }
}

impl ArgDef {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    pub fn value_name(mut self, value_name: impl Into<String>) -> Self {
        self.value_name = Some(value_name.into());
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    pub fn value_kind(mut self, value_kind: ValueKind) -> Self {
        self.value_kind = Some(value_kind);
        self
    }

    pub fn choices(mut self, choices: impl IntoIterator<Item = ValueChoice>) -> Self {
        self.choices.extend(choices);
        self
    }

    pub fn defaults(mut self, defaults: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.defaults.extend(defaults.into_iter().map(Into::into));
        self
    }
}

impl FlagDef {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    pub fn short(mut self, short: char) -> Self {
        self.short = Some(short);
        self
    }

    pub fn long(mut self, long: impl Into<String>) -> Self {
        self.long = Some(long.into());
        self
    }

    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    pub fn aliases(mut self, aliases: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn takes_value(mut self, value_name: impl Into<String>) -> Self {
        self.takes_value = true;
        self.value_name = Some(value_name.into());
        self
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    pub fn value_kind(mut self, value_kind: ValueKind) -> Self {
        self.value_kind = Some(value_kind);
        self
    }

    pub fn choices(mut self, choices: impl IntoIterator<Item = ValueChoice>) -> Self {
        self.choices.extend(choices);
        self
    }

    pub fn defaults(mut self, defaults: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.defaults.extend(defaults.into_iter().map(Into::into));
        self
    }

    pub fn takes_no_value(mut self) -> Self {
        self.takes_value = false;
        self.value_name = None;
        self
    }
}

impl ValueChoice {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            ..Self::default()
        }
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn display(mut self, display: impl Into<String>) -> Self {
        self.display = Some(display.into());
        self
    }

    pub fn sort(mut self, sort_key: impl Into<String>) -> Self {
        self.sort_key = Some(sort_key.into());
        self
    }
}

#[cfg(feature = "clap")]
impl CommandDef {
    pub fn from_clap(command: clap::Command) -> Self {
        clap_command_to_def(command)
    }
}

#[cfg(feature = "clap")]
fn clap_command_to_def(command: clap::Command) -> CommandDef {
    let mut usage_command = command.clone();
    let usage = normalize_usage_line(usage_command.render_usage().to_string());

    CommandDef {
        name: command.get_name().to_string(),
        about: styled_to_plain(command.get_about()),
        long_about: styled_to_plain(command.get_long_about()),
        usage,
        before_help: styled_to_plain(
            command
                .get_before_long_help()
                .or_else(|| command.get_before_help()),
        ),
        after_help: styled_to_plain(
            command
                .get_after_long_help()
                .or_else(|| command.get_after_help()),
        ),
        aliases: command
            .get_visible_aliases()
            .map(ToString::to_string)
            .collect(),
        hidden: command.is_hide_set(),
        sort_key: None,
        policy: CommandPolicyDef::default(),
        args: command
            .get_positionals()
            .filter(|arg| !arg.is_hide_set())
            .map(arg_def_from_clap)
            .collect(),
        flags: command
            .get_arguments()
            .filter(|arg| !arg.is_positional() && !arg.is_hide_set())
            .map(flag_def_from_clap)
            .collect(),
        subcommands: command
            .get_subcommands()
            .filter(|subcommand| !subcommand.is_hide_set())
            .map(|subcommand| clap_command_to_def(subcommand.clone()))
            .collect(),
    }
}

#[cfg(feature = "clap")]
fn arg_def_from_clap(arg: &clap::Arg) -> ArgDef {
    ArgDef {
        id: arg.get_id().as_str().to_string(),
        value_name: arg
            .get_value_names()
            .and_then(|names| names.first())
            .map(ToString::to_string),
        help: styled_to_plain(arg.get_long_help().or_else(|| arg.get_help())),
        help_heading: arg.get_help_heading().map(ToString::to_string),
        required: arg.is_required_set(),
        multi: arg.get_num_args().is_some_and(range_is_multiple)
            || matches!(arg.get_action(), clap::ArgAction::Append),
        value_kind: value_kind_from_hint(arg.get_value_hint()),
        choices: arg
            .get_possible_values()
            .into_iter()
            .filter(|value| !value.is_hide_set())
            .map(|value| {
                let mut choice = ValueChoice::new(value.get_name());
                if let Some(help) = value.get_help() {
                    choice = choice.help(help.to_string());
                }
                choice
            })
            .collect(),
        defaults: arg
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect(),
    }
}

#[cfg(feature = "clap")]
fn flag_def_from_clap(arg: &clap::Arg) -> FlagDef {
    let aliases = arg
        .get_long_and_visible_aliases()
        .into_iter()
        .flatten()
        .filter(|alias| Some(*alias) != arg.get_long())
        .map(|alias| format!("--{alias}"))
        .chain(
            arg.get_short_and_visible_aliases()
                .into_iter()
                .flatten()
                .filter(|alias| Some(*alias) != arg.get_short())
                .map(|alias| format!("-{alias}")),
        )
        .collect::<Vec<_>>();

    FlagDef {
        id: arg.get_id().as_str().to_string(),
        short: arg.get_short(),
        long: arg.get_long().map(ToString::to_string),
        aliases,
        help: styled_to_plain(arg.get_long_help().or_else(|| arg.get_help())),
        help_heading: arg.get_help_heading().map(ToString::to_string),
        takes_value: arg.get_action().takes_values(),
        value_name: arg
            .get_value_names()
            .and_then(|names| names.first())
            .map(ToString::to_string),
        required: arg.is_required_set(),
        multi: arg.get_num_args().is_some_and(range_is_multiple)
            || matches!(arg.get_action(), clap::ArgAction::Append),
        hidden: arg.is_hide_set(),
        value_kind: value_kind_from_hint(arg.get_value_hint()),
        choices: arg
            .get_possible_values()
            .into_iter()
            .filter(|value| !value.is_hide_set())
            .map(|value| {
                let mut choice = ValueChoice::new(value.get_name());
                if let Some(help) = value.get_help() {
                    choice = choice.help(help.to_string());
                }
                choice
            })
            .collect(),
        defaults: arg
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect(),
    }
}

#[cfg(feature = "clap")]
fn styled_to_plain(value: Option<&clap::builder::StyledStr>) -> Option<String> {
    value
        .map(ToString::to_string)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(feature = "clap")]
fn range_is_multiple(range: clap::builder::ValueRange) -> bool {
    range.min_values() > 1 || range.max_values() > 1
}

#[cfg(feature = "clap")]
fn value_kind_from_hint(hint: clap::ValueHint) -> Option<ValueKind> {
    match hint {
        clap::ValueHint::AnyPath
        | clap::ValueHint::FilePath
        | clap::ValueHint::DirPath
        | clap::ValueHint::ExecutablePath => Some(ValueKind::Path),
        _ => None,
    }
}

#[cfg(feature = "clap")]
fn normalize_usage_line(value: String) -> Option<String> {
    value
        .trim()
        .strip_prefix("Usage:")
        .map(str::trim)
        .filter(|usage| !usage.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
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
}
