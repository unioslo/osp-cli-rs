//! Wire-format DTOs for the plugin protocol.
//!
//! This module exists to define the stable boundary between `osp-cli` and
//! external plugins. The app and plugin manager can evolve internally, but the
//! JSON shapes in this module are the contract that both sides need to agree
//! on.
//!
//! In broad terms:
//!
//! - `Describe*` types advertise commands, arguments, and policy metadata
//! - `Response*` types carry execution results, messages, and render hints
//! - validation helpers reject protocol-shape errors before higher-level code
//!   tries to trust the payload
//!
//! Wire flow:
//!
//! ```text
//! plugin executable
//!      │
//!      ├── `describe` -> DescribeV1 / DescribeCommandV1
//!      │                host builds command catalog, completion, and policy
//!      │
//!      └── `run`      -> ResponseV1
//!                       host validates payload before adapting/rendering it
//! ```
//!
//! Useful mental split:
//!
//! - plugin authors care about these types as the stable JSON contract
//! - host-side code cares about them as validated input before converting into
//!   command catalogs, policy registries, and rendered output
//!
//! Contract:
//!
//! - these types may depend on shared `core` metadata, but they should stay
//!   free of host runtime concerns
//! - any parsing/validation here should enforce protocol rules, not business
//!   policy
//! - caller-facing docs should describe stable wire behavior rather than
//!   internal plugin manager details

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::command_def::{
    ArgDef, CommandDef, CommandPolicyDef, FlagDef, ValueChoice, ValueKind,
};
use crate::core::command_policy::{CommandPath, CommandPolicy, VisibilityMode};

/// Current plugin wire protocol version understood by this crate.
pub const PLUGIN_PROTOCOL_V1: u32 = 1;

/// `describe` payload emitted by a plugin that speaks protocol v1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeV1 {
    /// Protocol version declared by the plugin.
    pub protocol_version: u32,
    /// Stable plugin identifier.
    pub plugin_id: String,
    /// Plugin version string.
    pub plugin_version: String,
    /// Minimum `osp-cli` version required by the plugin, if any.
    pub min_osp_version: Option<String>,
    /// Top-level commands exported by the plugin.
    pub commands: Vec<DescribeCommandV1>,
}

/// Recursive command description used in plugin metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeCommandV1 {
    /// Command name exposed by the plugin.
    pub name: String,
    /// Short help text for the command.
    #[serde(default)]
    pub about: String,
    /// Optional authorization metadata for the command.
    #[serde(default)]
    pub auth: Option<DescribeCommandAuthV1>,
    /// Positional argument descriptions in declaration order.
    #[serde(default)]
    pub args: Vec<DescribeArgV1>,
    /// Flag descriptions keyed by protocol flag spelling.
    #[serde(default)]
    pub flags: BTreeMap<String, DescribeFlagV1>,
    /// Nested subcommands under this command.
    #[serde(default)]
    pub subcommands: Vec<DescribeCommandV1>,
}

/// Authorization metadata attached to a described command.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribeCommandAuthV1 {
    /// Visibility level for the command.
    #[serde(default)]
    pub visibility: Option<DescribeVisibilityModeV1>,
    /// Capabilities required to run the command.
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    /// Feature flags that must be enabled for the command.
    #[serde(default)]
    pub feature_flags: Vec<String>,
}

/// Wire-format visibility mode used by plugin metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescribeVisibilityModeV1 {
    /// Command is visible and runnable without authentication.
    Public,
    /// Command requires an authenticated user.
    Authenticated,
    /// Command requires one or more capabilities.
    CapabilityGated,
    /// Command should be hidden from normal help surfaces.
    Hidden,
}

impl DescribeVisibilityModeV1 {
    /// Converts the protocol visibility label into the internal policy enum.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::command_policy::VisibilityMode;
    /// use osp_cli::core::plugin::DescribeVisibilityModeV1;
    ///
    /// assert_eq!(
    ///     DescribeVisibilityModeV1::CapabilityGated.as_visibility_mode(),
    ///     VisibilityMode::CapabilityGated
    /// );
    /// ```
    pub fn as_visibility_mode(self) -> VisibilityMode {
        match self {
            DescribeVisibilityModeV1::Public => VisibilityMode::Public,
            DescribeVisibilityModeV1::Authenticated => VisibilityMode::Authenticated,
            DescribeVisibilityModeV1::CapabilityGated => VisibilityMode::CapabilityGated,
            DescribeVisibilityModeV1::Hidden => VisibilityMode::Hidden,
        }
    }

    /// Returns the canonical protocol label for this visibility mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::plugin::DescribeVisibilityModeV1;
    ///
    /// assert_eq!(DescribeVisibilityModeV1::Hidden.as_label(), "hidden");
    /// ```
    pub fn as_label(self) -> &'static str {
        match self {
            DescribeVisibilityModeV1::Public => "public",
            DescribeVisibilityModeV1::Authenticated => "authenticated",
            DescribeVisibilityModeV1::CapabilityGated => "capability_gated",
            DescribeVisibilityModeV1::Hidden => "hidden",
        }
    }
}

impl DescribeCommandAuthV1 {
    /// Returns a compact help hint for non-default auth requirements.
    ///
    /// This is meant for help and completion surfaces where the full policy
    /// object would be too noisy.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::plugin::{DescribeCommandAuthV1, DescribeVisibilityModeV1};
    ///
    /// let auth = DescribeCommandAuthV1 {
    ///     visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
    ///     required_capabilities: vec!["ldap.write".to_string()],
    ///     feature_flags: vec!["write-mode".to_string()],
    /// };
    ///
    /// assert_eq!(
    ///     auth.hint().as_deref(),
    ///     Some("cap: ldap.write; feature: write-mode")
    /// );
    /// ```
    pub fn hint(&self) -> Option<String> {
        let mut parts = Vec::new();

        match self.visibility {
            Some(DescribeVisibilityModeV1::Public) | None => {}
            Some(DescribeVisibilityModeV1::Authenticated) => parts.push("auth".to_string()),
            Some(DescribeVisibilityModeV1::CapabilityGated) => {
                if self.required_capabilities.len() == 1 {
                    parts.push(format!("cap: {}", self.required_capabilities[0]));
                } else if self.required_capabilities.is_empty() {
                    parts.push("cap".to_string());
                } else {
                    parts.push(format!("caps: {}", self.required_capabilities.len()));
                }
            }
            Some(DescribeVisibilityModeV1::Hidden) => parts.push("hidden".to_string()),
        }

        match self.feature_flags.as_slice() {
            [] => {}
            [feature] => parts.push(format!("feature: {feature}")),
            features => parts.push(format!("features: {}", features.len())),
        }

        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

/// Wire-format type hint for plugin argument values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DescribeValueTypeV1 {
    /// Value represents a filesystem path.
    Path,
}

/// Suggested value emitted in plugin metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribeSuggestionV1 {
    /// Raw suggestion value inserted into the command line.
    pub value: String,
    /// Optional short metadata string.
    #[serde(default)]
    pub meta: Option<String>,
    /// Optional display label for menu rendering.
    #[serde(default)]
    pub display: Option<String>,
    /// Optional sort key used for ordering suggestions.
    #[serde(default)]
    pub sort: Option<String>,
}

/// Positional argument description emitted by a plugin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribeArgV1 {
    /// Positional name or value label.
    #[serde(default)]
    pub name: Option<String>,
    /// Short help text for the argument.
    #[serde(default)]
    pub about: Option<String>,
    /// Whether the argument may be repeated.
    #[serde(default)]
    pub multi: bool,
    /// Optional wire-format value type hint.
    #[serde(default)]
    pub value_type: Option<DescribeValueTypeV1>,
    /// Suggested values for the argument.
    #[serde(default)]
    pub suggestions: Vec<DescribeSuggestionV1>,
}

/// Flag description emitted by a plugin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescribeFlagV1 {
    /// Short help text for the flag.
    #[serde(default)]
    pub about: Option<String>,
    /// Whether the flag is boolean-only and takes no value.
    #[serde(default)]
    pub flag_only: bool,
    /// Whether the flag may be repeated.
    #[serde(default)]
    pub multi: bool,
    /// Optional wire-format value type hint.
    #[serde(default)]
    pub value_type: Option<DescribeValueTypeV1>,
    /// Suggested values for the flag.
    #[serde(default)]
    pub suggestions: Vec<DescribeSuggestionV1>,
}

/// Protocol v1 command response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseV1 {
    /// Protocol version declared by the response.
    pub protocol_version: u32,
    /// Whether the command completed successfully.
    pub ok: bool,
    /// Response payload produced by the plugin.
    pub data: serde_json::Value,
    /// Structured error payload present when `ok` is `false`.
    pub error: Option<ResponseErrorV1>,
    /// User-facing messages emitted alongside the payload.
    #[serde(default)]
    pub messages: Vec<ResponseMessageV1>,
    /// Rendering and presentation metadata for the payload.
    pub meta: ResponseMetaV1,
}

/// Structured error payload returned when `ok` is `false`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseErrorV1 {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Arbitrary structured error details.
    #[serde(default)]
    pub details: serde_json::Value,
}

/// Rendering hints attached to a plugin response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResponseMetaV1 {
    /// Preferred output format for rendering the payload.
    pub format_hint: Option<String>,
    /// Preferred column order for row-based payloads.
    pub columns: Option<Vec<String>>,
    /// Preferred alignment hints for displayed columns.
    #[serde(default)]
    pub column_align: Vec<ColumnAlignmentV1>,
}

/// Column alignment hint used in plugin response metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColumnAlignmentV1 {
    /// Use the renderer's default alignment.
    #[default]
    Default,
    /// Left-align the column.
    Left,
    /// Center-align the column.
    Center,
    /// Right-align the column.
    Right,
}

/// Message severity carried in plugin responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseMessageLevelV1 {
    /// Error-level message.
    Error,
    /// Warning-level message.
    Warning,
    /// Success-level message.
    Success,
    /// Informational message.
    Info,
    /// Trace or debug-style message.
    Trace,
}

/// User-facing message emitted alongside a plugin response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMessageV1 {
    /// Severity level for the message.
    pub level: ResponseMessageLevelV1,
    /// Human-readable message text.
    pub text: String,
}

impl DescribeV1 {
    #[cfg(feature = "clap")]
    /// Builds a v1 describe payload from a single `clap` command tree.
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
    /// Builds a v1 describe payload from multiple top-level `clap` commands.
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
                .map(CommandDef::from_clap)
                .map(|command| DescribeCommandV1::from(&command))
                .collect(),
        }
    }

    /// Validates the describe payload and returns an error string on protocol
    /// violations.
    ///
    /// Hosts should do this before trusting plugin describe data enough to turn
    /// it into command catalogs, completion trees, or policy registries.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::plugin::{DescribeV1, PLUGIN_PROTOCOL_V1};
    ///
    /// let describe = DescribeV1 {
    ///     protocol_version: PLUGIN_PROTOCOL_V1,
    ///     plugin_id: "ldap".to_string(),
    ///     plugin_version: "0.1.0".to_string(),
    ///     min_osp_version: None,
    ///     commands: Vec::new(),
    /// };
    ///
    /// assert!(describe.validate_v1().is_ok());
    ///
    /// let invalid = DescribeV1 {
    ///     plugin_id: "   ".to_string(),
    ///     ..describe.clone()
    /// };
    /// assert_eq!(invalid.validate_v1().unwrap_err(), "plugin_id must not be empty");
    /// ```
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

impl DescribeCommandV1 {
    /// Converts command auth metadata into an internal command policy for
    /// `path`.
    ///
    /// This is the host-side bridge from wire-format describe data into the
    /// runtime policy evaluator in [`crate::core::command_policy`].
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::command_policy::{CommandPath, VisibilityMode};
    /// use osp_cli::core::plugin::{
    ///     DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1,
    /// };
    /// use std::collections::BTreeMap;
    ///
    /// let command = DescribeCommandV1 {
    ///     name: "decide".to_string(),
    ///     about: String::new(),
    ///     auth: Some(DescribeCommandAuthV1 {
    ///         visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
    ///         required_capabilities: vec![" Orch.Approval.Decide ".to_string()],
    ///         feature_flags: vec![" Review ".to_string()],
    ///     }),
    ///     args: Vec::new(),
    ///     flags: BTreeMap::new(),
    ///     subcommands: Vec::new(),
    /// };
    ///
    /// let policy = command
    ///     .command_policy(CommandPath::new(["orch", "approval", "decide"]))
    ///     .unwrap();
    ///
    /// assert_eq!(policy.visibility, VisibilityMode::CapabilityGated);
    /// assert!(policy
    ///     .required_capabilities
    ///     .contains("orch.approval.decide"));
    /// assert!(policy.feature_flags.contains("review"));
    /// ```
    pub fn command_policy(&self, path: CommandPath) -> Option<CommandPolicy> {
        let auth = self.auth.as_ref()?;
        let mut policy = CommandPolicy::new(path);
        if let Some(visibility) = auth.visibility {
            policy = policy.visibility(visibility.as_visibility_mode());
        }
        for capability in &auth.required_capabilities {
            policy = policy.require_capability(capability.clone());
        }
        for feature in &auth.feature_flags {
            policy = policy.feature_flag(feature.clone());
        }
        Some(policy)
    }
}

impl ResponseV1 {
    /// Validates the response envelope before the app trusts its payload.
    ///
    /// Hosts should run this before adapting plugin JSON into rows, semantic
    /// output, or user-facing messages.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::plugin::{ResponseMetaV1, ResponseV1};
    /// use serde_json::json;
    ///
    /// let response = ResponseV1 {
    ///     protocol_version: 1,
    ///     ok: true,
    ///     data: json!({"uid": "alice"}),
    ///     error: None,
    ///     messages: Vec::new(),
    ///     meta: ResponseMetaV1::default(),
    /// };
    ///
    /// assert!(response.validate_v1().is_ok());
    ///
    /// let invalid = ResponseV1 {
    ///     ok: false,
    ///     error: None,
    ///     ..response.clone()
    /// };
    ///
    /// assert_eq!(
    ///     invalid.validate_v1().unwrap_err(),
    ///     "ok=false requires error payload"
    /// );
    /// ```
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
    /// Converts a `clap` command into a protocol v1 command description.
    pub fn from_clap(command: clap::Command) -> Self {
        Self::from(&CommandDef::from_clap(command))
    }
}

impl From<&CommandDef> for DescribeCommandV1 {
    fn from(command: &CommandDef) -> Self {
        Self {
            name: command.name.clone(),
            about: command.about.clone().unwrap_or_default(),
            auth: (!command.policy.is_empty()).then(|| DescribeCommandAuthV1 {
                visibility: match command.policy.visibility {
                    VisibilityMode::Public => None,
                    VisibilityMode::Authenticated => Some(DescribeVisibilityModeV1::Authenticated),
                    VisibilityMode::CapabilityGated => {
                        Some(DescribeVisibilityModeV1::CapabilityGated)
                    }
                    VisibilityMode::Hidden => Some(DescribeVisibilityModeV1::Hidden),
                },
                required_capabilities: command.policy.required_capabilities.clone(),
                feature_flags: command.policy.feature_flags.clone(),
            }),
            args: command.args.iter().map(DescribeArgV1::from).collect(),
            flags: command
                .flags
                .iter()
                .flat_map(describe_flag_entries)
                .collect(),
            subcommands: command
                .subcommands
                .iter()
                .map(DescribeCommandV1::from)
                .collect(),
        }
    }
}

impl From<&DescribeCommandV1> for CommandDef {
    fn from(command: &DescribeCommandV1) -> Self {
        Self {
            name: command.name.clone(),
            about: (!command.about.trim().is_empty()).then(|| command.about.clone()),
            long_about: None,
            usage: None,
            before_help: None,
            after_help: None,
            aliases: Vec::new(),
            hidden: matches!(
                command.auth.as_ref().and_then(|auth| auth.visibility),
                Some(DescribeVisibilityModeV1::Hidden)
            ),
            sort_key: None,
            policy: command
                .auth
                .as_ref()
                .map(command_policy_from_describe)
                .unwrap_or_default(),
            args: command.args.iter().map(ArgDef::from).collect(),
            flags: collect_describe_flags(&command.flags),
            subcommands: command.subcommands.iter().map(CommandDef::from).collect(),
        }
    }
}

impl From<&ArgDef> for DescribeArgV1 {
    fn from(arg: &ArgDef) -> Self {
        Self {
            name: arg.value_name.clone().or_else(|| Some(arg.id.clone())),
            about: arg.help.clone(),
            multi: arg.multi,
            value_type: describe_value_type(arg.value_kind),
            suggestions: arg.choices.iter().map(DescribeSuggestionV1::from).collect(),
        }
    }
}

impl From<&FlagDef> for DescribeFlagV1 {
    fn from(flag: &FlagDef) -> Self {
        Self {
            about: flag.help.clone(),
            flag_only: !flag.takes_value,
            multi: flag.multi,
            value_type: describe_value_type(flag.value_kind),
            suggestions: flag
                .choices
                .iter()
                .map(DescribeSuggestionV1::from)
                .collect(),
        }
    }
}

impl From<&DescribeArgV1> for ArgDef {
    fn from(arg: &DescribeArgV1) -> Self {
        let mut def = ArgDef::new(arg.name.clone().unwrap_or_else(|| "value".to_string()));
        if let Some(value_name) = &arg.name {
            def = def.value_name(value_name.clone());
        }
        if let Some(help) = &arg.about {
            def = def.help(help.clone());
        }
        if arg.multi {
            def = def.multi();
        }
        if let Some(value_kind) = command_value_kind(arg.value_type) {
            def = def.value_kind(value_kind);
        }
        def.choices(arg.suggestions.iter().map(ValueChoice::from))
    }
}

impl From<&DescribeFlagV1> for FlagDef {
    fn from(flag: &DescribeFlagV1) -> Self {
        let mut def = FlagDef::new("flag");
        if let Some(help) = &flag.about {
            def = def.help(help.clone());
        }
        if !flag.flag_only {
            def = def.takes_value("value");
        }
        if flag.multi {
            def = def.multi();
        }
        if let Some(value_kind) = command_value_kind(flag.value_type) {
            def = def.value_kind(value_kind);
        }
        def.choices(flag.suggestions.iter().map(ValueChoice::from))
    }
}

impl From<&ValueChoice> for DescribeSuggestionV1 {
    fn from(choice: &ValueChoice) -> Self {
        Self {
            value: choice.value.clone(),
            meta: choice.help.clone(),
            display: choice.display.clone(),
            sort: choice.sort_key.clone(),
        }
    }
}

impl From<&DescribeSuggestionV1> for ValueChoice {
    fn from(entry: &DescribeSuggestionV1) -> Self {
        Self {
            value: entry.value.clone(),
            help: entry.meta.clone(),
            display: entry.display.clone(),
            sort_key: entry.sort.clone(),
        }
    }
}

fn validate_command(command: &DescribeCommandV1) -> Result<(), String> {
    if command.name.trim().is_empty() {
        return Err("command name must not be empty".to_string());
    }
    if let Some(auth) = &command.auth {
        validate_command_auth(auth)?;
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

fn validate_command_auth(auth: &DescribeCommandAuthV1) -> Result<(), String> {
    if auth
        .required_capabilities
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err("required_capabilities must not contain empty values".to_string());
    }
    if auth
        .feature_flags
        .iter()
        .any(|value| value.trim().is_empty())
    {
        return Err("feature_flags must not contain empty values".to_string());
    }
    Ok(())
}

fn describe_flag_entries(flag: &FlagDef) -> Vec<(String, DescribeFlagV1)> {
    let value = DescribeFlagV1::from(flag);
    let mut names = Vec::new();
    if let Some(long) = flag.long.as_deref() {
        names.push(format!("--{long}"));
    }
    if let Some(short) = flag.short {
        names.push(format!("-{short}"));
    }
    names.extend(flag.aliases.iter().cloned());
    names
        .into_iter()
        .map(|name| (name, value.clone()))
        .collect()
}

fn group_describe_flag((name, flag): (&String, &DescribeFlagV1)) -> Option<FlagDef> {
    if !name.starts_with('-') {
        return None;
    }

    let mut def = FlagDef::from(flag);
    if let Some(long) = name.strip_prefix("--") {
        def.long = Some(long.to_string());
        def.id = long.to_string();
    } else if let Some(short) = name.strip_prefix('-') {
        def.short = short.chars().next();
        def.id = short.to_string();
    }
    Some(def)
}

fn collect_describe_flags(flags: &BTreeMap<String, DescribeFlagV1>) -> Vec<FlagDef> {
    let mut grouped: BTreeMap<String, Vec<(&String, &DescribeFlagV1)>> = BTreeMap::new();
    for entry in flags.iter() {
        let signature = serde_json::to_string(entry.1).unwrap_or_default();
        grouped.entry(signature).or_default().push(entry);
    }

    grouped
        .into_values()
        .filter_map(|group| {
            let mut iter = group.into_iter();
            let first = iter.next()?;
            let mut def = group_describe_flag(first)?;
            for (name, _) in iter {
                if let Some(long) = name.strip_prefix("--") {
                    if def.long.is_none() {
                        def.long = Some(long.to_string());
                        if def.id == "flag" {
                            def.id = long.to_string();
                        }
                    } else if Some(long) != def.long.as_deref() {
                        def.aliases.push(format!("--{long}"));
                    }
                } else if let Some(short) = name.strip_prefix('-') {
                    let short_char = short.chars().next();
                    if def.short.is_none() {
                        def.short = short_char;
                        if def.id == "flag" {
                            def.id = short.to_string();
                        }
                    } else if short_char != def.short {
                        def.aliases.push(format!("-{short}"));
                    }
                }
            }
            Some(def)
        })
        .collect()
}

fn command_policy_from_describe(auth: &DescribeCommandAuthV1) -> CommandPolicyDef {
    CommandPolicyDef {
        visibility: match auth.visibility {
            Some(DescribeVisibilityModeV1::Authenticated) => VisibilityMode::Authenticated,
            Some(DescribeVisibilityModeV1::CapabilityGated) => VisibilityMode::CapabilityGated,
            Some(DescribeVisibilityModeV1::Hidden) => VisibilityMode::Hidden,
            Some(DescribeVisibilityModeV1::Public) | None => VisibilityMode::Public,
        },
        required_capabilities: auth.required_capabilities.clone(),
        feature_flags: auth.feature_flags.clone(),
    }
}

fn describe_value_type(value_kind: Option<ValueKind>) -> Option<DescribeValueTypeV1> {
    match value_kind {
        Some(ValueKind::Path) => Some(DescribeValueTypeV1::Path),
        Some(ValueKind::Enum | ValueKind::FreeText) | None => None,
    }
}

fn command_value_kind(value_type: Option<DescribeValueTypeV1>) -> Option<ValueKind> {
    value_type.map(|_| ValueKind::Path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1, validate_command_auth,
    };
    use crate::core::command_policy::{CommandPath, VisibilityMode};

    #[test]
    fn command_auth_converts_to_generic_command_policy_unit() {
        let command = DescribeCommandV1 {
            name: "orch".to_string(),
            about: String::new(),
            auth: Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
                required_capabilities: vec!["orch.approval.decide".to_string()],
                feature_flags: vec!["orch".to_string()],
            }),
            args: Vec::new(),
            flags: BTreeMap::new(),
            subcommands: Vec::new(),
        };

        let policy = command
            .command_policy(CommandPath::new(["orch", "approval", "decide"]))
            .expect("auth metadata should build a policy");
        assert_eq!(policy.visibility, VisibilityMode::CapabilityGated);
        assert!(
            policy
                .required_capabilities
                .contains("orch.approval.decide")
        );
        assert!(policy.feature_flags.contains("orch"));
    }

    #[test]
    fn command_auth_validation_rejects_blank_entries_unit() {
        let err = validate_command_auth(&DescribeCommandAuthV1 {
            visibility: None,
            required_capabilities: vec![" ".to_string()],
            feature_flags: Vec::new(),
        })
        .expect_err("blank capabilities should be rejected");
        assert!(err.contains("required_capabilities"));
    }

    #[test]
    fn command_auth_hint_stays_compact_and_stable_unit() {
        let auth = DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
            required_capabilities: vec!["orch.approval.decide".to_string()],
            feature_flags: vec!["orch".to_string()],
        };
        assert_eq!(
            auth.hint().as_deref(),
            Some("cap: orch.approval.decide; feature: orch")
        );
        assert_eq!(
            DescribeVisibilityModeV1::Authenticated.as_label(),
            "authenticated"
        );
    }
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
