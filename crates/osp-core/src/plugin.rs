use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PLUGIN_PROTOCOL_V1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeV1 {
    pub protocol_version: u32,
    pub plugin_id: String,
    pub plugin_version: String,
    pub min_osp_version: Option<String>,
    pub commands: Vec<DescribeCommandV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeCommandV1 {
    pub name: String,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub args: Vec<DescribeArgV1>,
    #[serde(default)]
    pub flags: BTreeMap<String, DescribeFlagV1>,
    #[serde(default)]
    pub subcommands: Vec<DescribeCommandV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DescribeValueTypeV1 {
    Path,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribeSuggestionV1 {
    pub value: String,
    #[serde(default)]
    pub meta: Option<String>,
    #[serde(default)]
    pub display: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribeArgV1 {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub value_type: Option<DescribeValueTypeV1>,
    #[serde(default)]
    pub suggestions: Vec<DescribeSuggestionV1>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribeFlagV1 {
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub flag_only: bool,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub value_type: Option<DescribeValueTypeV1>,
    #[serde(default)]
    pub suggestions: Vec<DescribeSuggestionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseV1 {
    pub protocol_version: u32,
    pub ok: bool,
    pub data: serde_json::Value,
    pub error: Option<ResponseErrorV1>,
    #[serde(default)]
    pub messages: Vec<ResponseMessageV1>,
    pub meta: ResponseMetaV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseErrorV1 {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponseMetaV1 {
    pub format_hint: Option<String>,
    pub columns: Option<Vec<String>>,
    #[serde(default)]
    pub column_align: Vec<ColumnAlignmentV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColumnAlignmentV1 {
    #[default]
    Default,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseMessageLevelV1 {
    Error,
    Warning,
    Success,
    Info,
    Trace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMessageV1 {
    pub level: ResponseMessageLevelV1,
    pub text: String,
}

impl DescribeV1 {
    #[cfg(feature = "clap")]
    pub fn from_clap_command(
        plugin_id: impl Into<String>,
        plugin_version: impl Into<String>,
        min_osp_version: Option<String>,
        command: clap::Command,
    ) -> Self {
        Self::from_clap_commands(
            plugin_id,
            plugin_version,
            min_osp_version,
            std::iter::once(command),
        )
    }

    #[cfg(feature = "clap")]
    pub fn from_clap_commands(
        plugin_id: impl Into<String>,
        plugin_version: impl Into<String>,
        min_osp_version: Option<String>,
        commands: impl IntoIterator<Item = clap::Command>,
    ) -> Self {
        Self {
            protocol_version: PLUGIN_PROTOCOL_V1,
            plugin_id: plugin_id.into(),
            plugin_version: plugin_version.into(),
            min_osp_version,
            commands: commands
                .into_iter()
                .map(DescribeCommandV1::from_clap)
                .collect(),
        }
    }

    pub fn validate_v1(&self) -> Result<(), String> {
        if self.protocol_version != PLUGIN_PROTOCOL_V1 {
            return Err(format!(
                "unsupported describe protocol version: {}",
                self.protocol_version
            ));
        }
        if self.plugin_id.trim().is_empty() {
            return Err("plugin_id must not be empty".to_string());
        }
        for command in &self.commands {
            validate_command(command)?;
        }
        Ok(())
    }
}

impl ResponseV1 {
    pub fn validate_v1(&self) -> Result<(), String> {
        if self.protocol_version != PLUGIN_PROTOCOL_V1 {
            return Err(format!(
                "unsupported response protocol version: {}",
                self.protocol_version
            ));
        }
        if self.ok && self.error.is_some() {
            return Err("ok=true requires error=null".to_string());
        }
        if !self.ok && self.error.is_none() {
            return Err("ok=false requires error payload".to_string());
        }
        if self
            .messages
            .iter()
            .any(|message| message.text.trim().is_empty())
        {
            return Err("response messages must not contain empty text".to_string());
        }
        Ok(())
    }
}

#[cfg(feature = "clap")]
impl DescribeCommandV1 {
    pub fn from_clap(command: clap::Command) -> Self {
        describe_command_from_clap(command)
    }
}

fn validate_command(command: &DescribeCommandV1) -> Result<(), String> {
    if command.name.trim().is_empty() {
        return Err("command name must not be empty".to_string());
    }

    for (name, flag) in &command.flags {
        if !name.starts_with('-') {
            return Err(format!("flag `{name}` must start with `-`"));
        }
        validate_suggestions(&flag.suggestions, &format!("flag `{name}`"))?;
    }

    for arg in &command.args {
        validate_suggestions(&arg.suggestions, "argument")?;
    }

    for subcommand in &command.subcommands {
        validate_command(subcommand)?;
    }

    Ok(())
}

fn validate_suggestions(suggestions: &[DescribeSuggestionV1], owner: &str) -> Result<(), String> {
    if suggestions
        .iter()
        .any(|entry| entry.value.trim().is_empty())
    {
        return Err(format!("{owner} suggestions must not contain empty values"));
    }
    Ok(())
}

#[cfg(feature = "clap")]
fn describe_command_from_clap(command: clap::Command) -> DescribeCommandV1 {
    let positionals = command
        .get_positionals()
        .filter(|arg| !arg.is_hide_set())
        .map(describe_arg_from_clap)
        .collect::<Vec<_>>();

    let mut flags = BTreeMap::new();
    for arg in command.get_arguments().filter(|arg| !arg.is_positional()) {
        if arg.is_hide_set() {
            continue;
        }
        let flag = describe_flag_from_clap(arg);
        for name in visible_flag_names(arg) {
            flags.insert(name, flag.clone());
        }
    }

    DescribeCommandV1 {
        name: command.get_name().to_string(),
        about: styled_to_plain(command.get_about()),
        args: positionals,
        flags,
        subcommands: command
            .get_subcommands()
            .filter(|subcommand| !subcommand.is_hide_set())
            .map(|subcommand| describe_command_from_clap(subcommand.clone()))
            .collect(),
    }
}

#[cfg(feature = "clap")]
fn describe_arg_from_clap(arg: &clap::Arg) -> DescribeArgV1 {
    DescribeArgV1 {
        name: arg
            .get_value_names()
            .and_then(|names| names.first())
            .map(ToString::to_string)
            .or_else(|| Some(arg.get_id().as_str().to_string())),
        about: Some(styled_to_plain(
            arg.get_long_help().or_else(|| arg.get_help()),
        ))
        .filter(|text| !text.is_empty()),
        multi: arg.get_num_args().is_some_and(range_is_multiple)
            || matches!(arg.get_action(), clap::ArgAction::Append),
        value_type: value_type_from_hint(arg.get_value_hint()),
        suggestions: describe_suggestions_from_clap(arg),
    }
}

#[cfg(feature = "clap")]
fn describe_flag_from_clap(arg: &clap::Arg) -> DescribeFlagV1 {
    DescribeFlagV1 {
        about: Some(styled_to_plain(
            arg.get_long_help().or_else(|| arg.get_help()),
        ))
        .filter(|text| !text.is_empty()),
        flag_only: !arg.get_action().takes_values(),
        multi: arg.get_num_args().is_some_and(range_is_multiple)
            || matches!(arg.get_action(), clap::ArgAction::Append),
        value_type: value_type_from_hint(arg.get_value_hint()),
        suggestions: describe_suggestions_from_clap(arg),
    }
}

#[cfg(feature = "clap")]
fn describe_suggestions_from_clap(arg: &clap::Arg) -> Vec<DescribeSuggestionV1> {
    arg.get_possible_values()
        .into_iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| DescribeSuggestionV1 {
            value: value.get_name().to_string(),
            meta: value.get_help().map(ToString::to_string),
            display: None,
            sort: None,
        })
        .collect()
}

#[cfg(feature = "clap")]
fn visible_flag_names(arg: &clap::Arg) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(longs) = arg.get_long_and_visible_aliases() {
        names.extend(longs.into_iter().map(|name| format!("--{name}")));
    }
    if let Some(shorts) = arg.get_short_and_visible_aliases() {
        names.extend(shorts.into_iter().map(|name| format!("-{name}")));
    }
    names
}

#[cfg(feature = "clap")]
fn value_type_from_hint(hint: clap::ValueHint) -> Option<DescribeValueTypeV1> {
    match hint {
        clap::ValueHint::AnyPath
        | clap::ValueHint::FilePath
        | clap::ValueHint::DirPath
        | clap::ValueHint::ExecutablePath => Some(DescribeValueTypeV1::Path),
        _ => None,
    }
}

#[cfg(feature = "clap")]
fn styled_to_plain(value: Option<&clap::builder::StyledStr>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

#[cfg(feature = "clap")]
fn range_is_multiple(range: clap::builder::ValueRange) -> bool {
    range.min_values() > 1 || range.max_values() > 1
}

#[cfg(all(test, feature = "clap"))]
mod clap_tests {
    use super::{DescribeCommandV1, DescribeV1, DescribeValueTypeV1};
    use clap::{Arg, ArgAction, Command, ValueHint};

    #[test]
    fn clap_helper_captures_subcommands_flags_and_args() {
        let command = Command::new("ldap").about("LDAP plugin").subcommand(
            Command::new("user")
                .about("Lookup LDAP users")
                .arg(Arg::new("uid").help("User id"))
                .arg(
                    Arg::new("attributes")
                        .long("attributes")
                        .short('a')
                        .help("Attributes to fetch")
                        .action(ArgAction::Set)
                        .value_parser(["uid", "cn", "mail"]),
                )
                .arg(
                    Arg::new("input")
                        .long("input")
                        .help("Read from file")
                        .value_hint(ValueHint::FilePath),
                ),
        );

        let describe =
            DescribeV1::from_clap_command("ldap", "0.1.0", Some("0.1.0".to_string()), command);

        assert_eq!(describe.commands.len(), 1);
        let ldap = &describe.commands[0];
        assert_eq!(ldap.name, "ldap");
        assert_eq!(ldap.subcommands.len(), 1);

        let user = &ldap.subcommands[0];
        assert_eq!(user.name, "user");
        assert_eq!(user.args[0].name.as_deref(), Some("uid"));
        assert!(user.flags.contains_key("--attributes"));
        assert!(user.flags.contains_key("-a"));
        assert_eq!(
            user.flags["--attributes"]
                .suggestions
                .iter()
                .map(|entry| entry.value.as_str())
                .collect::<Vec<_>>(),
            vec!["uid", "cn", "mail"]
        );
        assert_eq!(
            user.flags["--input"].value_type,
            Some(DescribeValueTypeV1::Path)
        );
    }

    #[test]
    fn clap_command_conversion_skips_hidden_items() {
        let command = Command::new("ldap")
            .subcommand(Command::new("visible"))
            .subcommand(Command::new("hidden").hide(true))
            .arg(Arg::new("secret").long("secret").hide(true));

        let describe = DescribeCommandV1::from_clap(command);

        assert_eq!(
            describe
                .subcommands
                .iter()
                .map(|subcommand| subcommand.name.as_str())
                .collect::<Vec<_>>(),
            vec!["visible"]
        );
        assert!(!describe.flags.contains_key("--secret"));
    }
}
