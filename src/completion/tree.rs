//! Declarative builders for the completion engine's immutable tree model.
//!
//! Public API shape:
//!
//! - describe command surfaces with [`CommandSpec`]
//! - lower them into a cached [`crate::completion::CompletionTree`] with
//!   [`CompletionTreeBuilder`]
//! - keep the resulting tree as plain data so the engine and embedders can
//!   reuse it without retaining builder state

use std::collections::BTreeMap;

use crate::completion::model::{
    ArgNode, CompletionNode, CompletionTree, FlagNode, SuggestionEntry,
};
use crate::core::command_def::{ArgDef, CommandDef, FlagDef, ValueChoice, ValueKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Declarative command description used to build a completion tree.
pub struct CommandSpec {
    /// Command or subcommand name.
    pub name: String,
    /// Optional description shown alongside the command.
    pub tooltip: Option<String>,
    /// Optional hidden sort key for display ordering.
    pub sort: Option<String>,
    /// Positional arguments accepted by this command.
    pub args: Vec<ArgNode>,
    /// Flags accepted by this command.
    pub flags: BTreeMap<String, FlagNode>,
    /// Nested subcommands below this command.
    pub subcommands: Vec<CommandSpec>,
}

impl CommandSpec {
    /// Starts a declarative command spec with the given command name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Attaches the description shown alongside this command in completion UIs.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Attaches a hidden sort key used to stabilize menu ordering.
    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.sort = Some(sort.into());
        self
    }

    /// Appends one positional argument definition.
    pub fn arg(mut self, arg: ArgNode) -> Self {
        self.args.push(arg);
        self
    }

    /// Extends the command with positional argument definitions.
    pub fn args(mut self, args: impl IntoIterator<Item = ArgNode>) -> Self {
        self.args.extend(args);
        self
    }

    /// Adds one flag definition keyed by its spelling.
    pub fn flag(mut self, name: impl Into<String>, flag: FlagNode) -> Self {
        self.flags.insert(name.into(), flag);
        self
    }

    /// Extends the command with multiple flag definitions.
    pub fn flags(mut self, flags: impl IntoIterator<Item = (String, FlagNode)>) -> Self {
        self.flags.extend(flags);
        self
    }

    /// Appends one nested subcommand.
    pub fn subcommand(mut self, subcommand: CommandSpec) -> Self {
        self.subcommands.push(subcommand);
        self
    }

    /// Extends the command with nested subcommands.
    pub fn subcommands(mut self, subcommands: impl IntoIterator<Item = CommandSpec>) -> Self {
        self.subcommands.extend(subcommands);
        self
    }
}

#[derive(Debug, Clone, Default)]
/// Builds immutable completion trees from command and config metadata.
///
/// This is the canonical builder surface for completion-tree construction.
pub struct CompletionTreeBuilder;

impl CompletionTreeBuilder {
    /// Builds an immutable completion tree from declarative command specs.
    ///
    /// The resulting structure is intentionally plain data so callers can cache
    /// it, augment it with plugin/provider hints, and pass it into the engine
    /// without keeping builder state alive.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{CommandSpec, CompletionTreeBuilder};
    ///
    /// let tree = CompletionTreeBuilder.build_from_specs(
    ///     &[CommandSpec::new("config")
    ///         .tooltip("Runtime configuration")
    ///         .subcommand(CommandSpec::new("set"))],
    ///     [("P".to_string(), "Project fields".to_string())],
    /// );
    ///
    /// assert!(tree.root.children.contains_key("config"));
    /// assert!(tree.root.children["config"].children.contains_key("set"));
    /// assert_eq!(tree.pipe_verbs.get("P").map(String::as_str), Some("Project fields"));
    /// ```
    pub fn build_from_specs(
        &self,
        specs: &[CommandSpec],
        pipe_verbs: impl IntoIterator<Item = (String, String)>,
    ) -> CompletionTree {
        let mut root = CompletionNode::default();
        for spec in specs {
            let name = spec.name.clone();
            assert!(
                root.children
                    .insert(name.clone(), Self::node_from_spec(spec))
                    .is_none(),
                "duplicate root command spec: {name}"
            );
        }

        CompletionTree {
            root,
            pipe_verbs: pipe_verbs.into_iter().collect(),
        }
    }

    /// Injects `config set` key completions into an existing tree.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{CommandSpec, CompletionTreeBuilder, ConfigKeySpec};
    ///
    /// let mut tree = CompletionTreeBuilder.build_from_specs(
    ///     &[CommandSpec::new("config").subcommand(CommandSpec::new("set"))],
    ///     [],
    /// );
    /// CompletionTreeBuilder.apply_config_set_keys(
    ///     &mut tree,
    ///     [
    ///         ConfigKeySpec::new("ui.format"),
    ///         ConfigKeySpec::new("log.level"),
    ///     ],
    /// );
    ///
    /// let set_node = &tree.root.children["config"].children["set"];
    /// assert!(set_node.children.contains_key("ui.format"));
    /// assert!(set_node.children.contains_key("log.level"));
    /// assert!(set_node.children["ui.format"].value_key);
    /// ```
    pub fn apply_config_set_keys(
        &self,
        tree: &mut CompletionTree,
        keys: impl IntoIterator<Item = ConfigKeySpec>,
    ) {
        let Some(config_node) = tree.root.children.get_mut("config") else {
            return;
        };
        let Some(set_node) = config_node.children.get_mut("set") else {
            return;
        };

        for key in keys {
            let key_name = key.key.clone();
            let mut node = CompletionNode {
                tooltip: key.tooltip,
                value_key: true,
                ..CompletionNode::default()
            };
            for suggestion in key.value_suggestions {
                node.children.insert(
                    suggestion.value.clone(),
                    CompletionNode {
                        value_leaf: true,
                        tooltip: suggestion.meta.clone(),
                        ..CompletionNode::default()
                    },
                );
            }
            assert!(
                set_node.children.insert(key_name.clone(), node).is_none(),
                "duplicate config set key: {key_name}"
            );
        }
    }

    fn node_from_spec(spec: &CommandSpec) -> CompletionNode {
        let mut node = CompletionNode {
            tooltip: spec.tooltip.clone(),
            sort: spec.sort.clone(),
            args: spec.args.clone(),
            flags: spec.flags.clone(),
            ..CompletionNode::default()
        };

        for subcommand in &spec.subcommands {
            let name = subcommand.name.clone();
            assert!(
                node.children
                    .insert(name.clone(), Self::node_from_spec(subcommand))
                    .is_none(),
                "duplicate subcommand spec: {name}"
            );
        }

        node
    }
}

pub(crate) fn command_spec_from_command_def(def: &CommandDef) -> CommandSpec {
    let mut spec = CommandSpec::new(def.name.clone())
        .args(def.args.iter().map(arg_node_from_def))
        .flags(
            def.flags
                .iter()
                .flat_map(flag_entries_from_def)
                .collect::<Vec<_>>(),
        )
        .subcommands(def.subcommands.iter().map(command_spec_from_command_def));

    if let Some(about) = def.about.as_deref() {
        spec = spec.tooltip(about);
    }
    if let Some(sort_key) = def.sort_key.as_deref() {
        spec = spec.sort(sort_key);
    }
    spec
}

fn arg_node_from_def(arg: &ArgDef) -> ArgNode {
    let mut node = ArgNode::named(arg.value_name.as_deref().unwrap_or(&arg.id))
        .suggestions(arg.choices.iter().map(suggestion_from_choice));
    if let Some(help) = arg.help.as_deref() {
        node = node.tooltip(help);
    }
    if arg.multi {
        node = node.multi();
    }
    if let Some(value_type) = to_completion_value_type(arg.value_kind) {
        node = node.value_type(value_type);
    }
    node
}

fn flag_entries_from_def(flag: &FlagDef) -> Vec<(String, FlagNode)> {
    let mut node = FlagNode::new().suggestions(flag.choices.iter().map(suggestion_from_choice));
    if let Some(help) = flag.help.as_deref() {
        node = node.tooltip(help);
    }
    if !flag.takes_value {
        node = node.flag_only();
    }
    if flag.multi {
        node = node.multi();
    }
    if let Some(value_type) = to_completion_value_type(flag.value_kind) {
        node = node.value_type(value_type);
    }

    flag_spellings(flag)
        .into_iter()
        .map(|name| (name, node.clone()))
        .collect()
}

fn flag_spellings(flag: &FlagDef) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(long) = flag.long.as_deref() {
        names.push(format!("--{long}"));
    }
    if let Some(short) = flag.short {
        names.push(format!("-{short}"));
    }
    names.extend(flag.aliases.iter().cloned());
    names
}

fn suggestion_from_choice(choice: &ValueChoice) -> SuggestionEntry {
    let mut entry = SuggestionEntry::value(choice.value.clone());
    if let Some(meta) = choice.help.as_deref() {
        entry = entry.meta(meta);
    }
    if let Some(display) = choice.display.as_deref() {
        entry = entry.display(display);
    }
    if let Some(sort_key) = choice.sort_key.as_deref() {
        entry = entry.sort(sort_key);
    }
    entry
}

fn to_completion_value_type(value_kind: Option<ValueKind>) -> Option<crate::completion::ValueType> {
    match value_kind {
        Some(ValueKind::Path) => Some(crate::completion::ValueType::Path),
        Some(ValueKind::Enum | ValueKind::FreeText) | None => None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Declarative `config set` key metadata used for completion nodes.
pub struct ConfigKeySpec {
    /// Config key name completed below `config set`.
    pub key: String,
    /// Optional description shown for the key.
    pub tooltip: Option<String>,
    /// Suggested values for the key.
    pub value_suggestions: Vec<SuggestionEntry>,
}

impl ConfigKeySpec {
    /// Creates a config key spec with the given key name.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ..Self::default()
        }
    }

    /// Sets the display tooltip for this config key.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Replaces the suggested values for this config key.
    pub fn value_suggestions(
        mut self,
        suggestions: impl IntoIterator<Item = SuggestionEntry>,
    ) -> Self {
        self.value_suggestions = suggestions.into_iter().collect();
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::completion::model::CompletionTree;
    use crate::core::command_def::{ArgDef, CommandDef, FlagDef, ValueChoice, ValueKind};

    use super::{CommandSpec, CompletionTreeBuilder, ConfigKeySpec, command_spec_from_command_def};

    fn build_tree() -> CompletionTree {
        CompletionTreeBuilder.build_from_specs(
            &[CommandSpec::new("config").subcommand(CommandSpec::new("set"))],
            [("F".to_string(), "Filter".to_string())],
        )
    }

    #[test]
    #[should_panic(expected = "duplicate root command spec")]
    fn duplicate_root_specs_fail_fast() {
        let _ = CompletionTreeBuilder.build_from_specs(
            &[CommandSpec::new("config"), CommandSpec::new("config")],
            [],
        );
    }

    #[test]
    #[should_panic(expected = "duplicate config set key")]
    fn duplicate_config_keys_fail_fast() {
        let mut tree = build_tree();
        CompletionTreeBuilder.apply_config_set_keys(
            &mut tree,
            [
                ConfigKeySpec::new("ui.format"),
                ConfigKeySpec::new("ui.format"),
            ],
        );
    }

    #[test]
    fn command_spec_conversion_preserves_flag_spellings_and_choices_unit() {
        let def = CommandDef::new("theme")
            .about("Inspect themes")
            .sort("10")
            .arg(
                ArgDef::new("name")
                    .help("Theme name")
                    .value_kind(ValueKind::Path)
                    .choices([ValueChoice::new("nord").help("Builtin theme")]),
            )
            .flag(
                FlagDef::new("raw")
                    .long("raw")
                    .short('r')
                    .alias("--plain")
                    .help("Show raw values"),
            );

        let spec = command_spec_from_command_def(&def);

        assert_eq!(spec.tooltip.as_deref(), Some("Inspect themes"));
        assert_eq!(spec.sort.as_deref(), Some("10"));
        assert!(spec.flags.contains_key("--raw"));
        assert!(spec.flags.contains_key("-r"));
        assert!(spec.flags.contains_key("--plain"));
        assert_eq!(spec.args[0].tooltip.as_deref(), Some("Theme name"));
        assert_eq!(spec.args[0].suggestions[0].value, "nord");
        assert_eq!(
            spec.args[0].value_type,
            Some(crate::completion::ValueType::Path)
        );
    }
}
