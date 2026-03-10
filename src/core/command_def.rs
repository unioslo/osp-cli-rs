use crate::core::command_policy::VisibilityMode;

/// Declarative command description used for help, completion, and plugin metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandDef {
    /// Canonical command name shown in the command path.
    pub name: String,
    /// Short summary used in compact help output.
    pub about: Option<String>,
    /// Expanded description used in detailed help output.
    pub long_about: Option<String>,
    /// Explicit usage line when generated usage should be overridden.
    pub usage: Option<String>,
    /// Text inserted before generated help content.
    pub before_help: Option<String>,
    /// Text appended after generated help content.
    pub after_help: Option<String>,
    /// Alternate visible names accepted for the command.
    pub aliases: Vec<String>,
    /// Whether the command should be hidden from generated discovery output.
    pub hidden: bool,
    /// Optional presentation key used to order sibling commands.
    pub sort_key: Option<String>,
    /// Policy metadata that controls command visibility and availability.
    pub policy: CommandPolicyDef,
    /// Positional arguments accepted by the command.
    pub args: Vec<ArgDef>,
    /// Flags and options accepted by the command.
    pub flags: Vec<FlagDef>,
    /// Nested subcommands exposed below this command.
    pub subcommands: Vec<CommandDef>,
}

/// Simplified policy description attached to a [`CommandDef`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPolicyDef {
    /// Visibility mode applied to the command.
    pub visibility: VisibilityMode,
    /// Capabilities required to execute or reveal the command.
    pub required_capabilities: Vec<String>,
    /// Feature flags that must be enabled for the command to exist.
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
    /// Returns `true` when the policy matches the default public, unrestricted state.
    pub fn is_empty(&self) -> bool {
        self.visibility == VisibilityMode::Public
            && self.required_capabilities.is_empty()
            && self.feature_flags.is_empty()
    }
}

/// Positional argument definition for a command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArgDef {
    /// Stable identifier for the argument.
    pub id: String,
    /// Placeholder shown for the argument value in help text.
    pub value_name: Option<String>,
    /// Help text shown for the argument.
    pub help: Option<String>,
    /// Optional help section heading for the argument.
    pub help_heading: Option<String>,
    /// Whether the argument must be supplied.
    pub required: bool,
    /// Whether the argument accepts multiple values.
    pub multi: bool,
    /// Semantic hint for completions and UI presentation.
    pub value_kind: Option<ValueKind>,
    /// Enumerated values suggested for the argument.
    pub choices: Vec<ValueChoice>,
    /// Default values applied when the argument is omitted.
    pub defaults: Vec<String>,
}

/// Flag or option definition for a command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlagDef {
    /// Stable identifier for the flag or option.
    pub id: String,
    /// Single-character short form without the leading `-`.
    pub short: Option<char>,
    /// Long form without the leading `--`.
    pub long: Option<String>,
    /// Alternate visible spellings accepted for the flag.
    pub aliases: Vec<String>,
    /// Help text shown for the flag.
    pub help: Option<String>,
    /// Optional help section heading for the flag.
    pub help_heading: Option<String>,
    /// Whether the flag consumes a value.
    pub takes_value: bool,
    /// Placeholder shown for the flag value in help text.
    pub value_name: Option<String>,
    /// Whether the flag must be supplied.
    pub required: bool,
    /// Whether the flag accepts multiple values or occurrences.
    pub multi: bool,
    /// Whether the flag should be hidden from generated discovery output.
    pub hidden: bool,
    /// Semantic hint for the flag value.
    pub value_kind: Option<ValueKind>,
    /// Enumerated values suggested for the flag.
    pub choices: Vec<ValueChoice>,
    /// Default values applied when the flag is omitted.
    pub defaults: Vec<String>,
}

/// Semantic type hint for argument and option values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Filesystem path input.
    Path,
    /// Value chosen from a fixed set of named options.
    Enum,
    /// Unstructured text input.
    FreeText,
}

/// Suggested value for an argument or flag.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValueChoice {
    /// Underlying value passed to the command.
    pub value: String,
    /// Help text describing when to use this value.
    pub help: Option<String>,
    /// Alternate label shown instead of the raw value.
    pub display: Option<String>,
    /// Optional presentation key used to order sibling values.
    pub sort_key: Option<String>,
}

impl CommandDef {
    /// Creates a command definition with the provided command name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Sets the short help text and returns the updated definition.
    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = Some(about.into());
        self
    }

    /// Sets the long help text and returns the updated definition.
    pub fn long_about(mut self, long_about: impl Into<String>) -> Self {
        self.long_about = Some(long_about.into());
        self
    }

    /// Sets the explicit usage line and returns the updated definition.
    pub fn usage(mut self, usage: impl Into<String>) -> Self {
        self.usage = Some(usage.into());
        self
    }

    /// Sets text shown before generated help output.
    pub fn before_help(mut self, text: impl Into<String>) -> Self {
        self.before_help = Some(text.into());
        self
    }

    /// Sets text shown after generated help output.
    pub fn after_help(mut self, text: impl Into<String>) -> Self {
        self.after_help = Some(text.into());
        self
    }

    /// Appends a visible alias and returns the updated definition.
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Appends multiple visible aliases and returns the updated definition.
    pub fn aliases(mut self, aliases: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    /// Marks the command as hidden from generated help and discovery output.
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Sets a sort key used when presenting the command alongside peers.
    pub fn sort(mut self, sort_key: impl Into<String>) -> Self {
        self.sort_key = Some(sort_key.into());
        self
    }

    /// Replaces the command policy metadata.
    pub fn policy(mut self, policy: CommandPolicyDef) -> Self {
        self.policy = policy;
        self
    }

    /// Appends a positional argument definition.
    pub fn arg(mut self, arg: ArgDef) -> Self {
        self.args.push(arg);
        self
    }

    /// Appends multiple positional argument definitions.
    pub fn args(mut self, args: impl IntoIterator<Item = ArgDef>) -> Self {
        self.args.extend(args);
        self
    }

    /// Appends a flag definition.
    pub fn flag(mut self, flag: FlagDef) -> Self {
        self.flags.push(flag);
        self
    }

    /// Appends multiple flag definitions.
    pub fn flags(mut self, flags: impl IntoIterator<Item = FlagDef>) -> Self {
        self.flags.extend(flags);
        self
    }

    /// Appends a nested subcommand definition.
    pub fn subcommand(mut self, subcommand: CommandDef) -> Self {
        self.subcommands.push(subcommand);
        self
    }

    /// Appends multiple nested subcommand definitions.
    pub fn subcommands(mut self, subcommands: impl IntoIterator<Item = CommandDef>) -> Self {
        self.subcommands.extend(subcommands);
        self
    }
}

impl ArgDef {
    /// Creates a positional argument definition with the provided identifier.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    /// Sets the displayed value name for the argument.
    pub fn value_name(mut self, value_name: impl Into<String>) -> Self {
        self.value_name = Some(value_name.into());
        self
    }

    /// Sets the help text for the argument.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Marks the argument as required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Marks the argument as accepting multiple values.
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Sets the semantic value kind for the argument.
    pub fn value_kind(mut self, value_kind: ValueKind) -> Self {
        self.value_kind = Some(value_kind);
        self
    }

    /// Appends supported value choices for the argument.
    pub fn choices(mut self, choices: impl IntoIterator<Item = ValueChoice>) -> Self {
        self.choices.extend(choices);
        self
    }

    /// Appends default values for the argument.
    pub fn defaults(mut self, defaults: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.defaults.extend(defaults.into_iter().map(Into::into));
        self
    }
}

impl FlagDef {
    /// Creates a flag definition with the provided identifier.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ..Self::default()
        }
    }

    /// Sets the short option name.
    pub fn short(mut self, short: char) -> Self {
        self.short = Some(short);
        self
    }

    /// Sets the long option name without the leading `--`.
    pub fn long(mut self, long: impl Into<String>) -> Self {
        self.long = Some(long.into());
        self
    }

    /// Appends an alias name for this flag.
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.aliases.push(alias.into());
        self
    }

    /// Appends multiple alias names for this flag.
    pub fn aliases(mut self, aliases: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.aliases.extend(aliases.into_iter().map(Into::into));
        self
    }

    /// Sets the help text for the flag.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Marks the flag as taking a value and sets its displayed value name.
    pub fn takes_value(mut self, value_name: impl Into<String>) -> Self {
        self.takes_value = true;
        self.value_name = Some(value_name.into());
        self
    }

    /// Marks the flag as required.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Marks the flag as accepting multiple values or occurrences.
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Marks the flag as hidden from generated help and discovery output.
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    /// Sets the semantic value kind for the flag's value.
    pub fn value_kind(mut self, value_kind: ValueKind) -> Self {
        self.value_kind = Some(value_kind);
        self
    }

    /// Appends supported value choices for the flag.
    pub fn choices(mut self, choices: impl IntoIterator<Item = ValueChoice>) -> Self {
        self.choices.extend(choices);
        self
    }

    /// Appends default values for the flag.
    pub fn defaults(mut self, defaults: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.defaults.extend(defaults.into_iter().map(Into::into));
        self
    }

    /// Marks the flag as not taking a value and clears any stored value name.
    pub fn takes_no_value(mut self) -> Self {
        self.takes_value = false;
        self.value_name = None;
        self
    }
}

impl ValueChoice {
    /// Creates a suggested value entry.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            ..Self::default()
        }
    }

    /// Sets the help text associated with this suggested value.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Sets the display label shown for this suggested value.
    pub fn display(mut self, display: impl Into<String>) -> Self {
        self.display = Some(display.into());
        self
    }

    /// Sets the presentation sort key for this suggested value.
    pub fn sort(mut self, sort_key: impl Into<String>) -> Self {
        self.sort_key = Some(sort_key.into());
        self
    }
}

#[cfg(feature = "clap")]
impl CommandDef {
    /// Converts a `clap` command tree into a [`CommandDef`] tree.
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
