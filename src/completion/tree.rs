use std::collections::BTreeMap;

use crate::completion::model::{
    ArgNode, CompletionNode, CompletionTree, FlagNode, SuggestionEntry,
};
use crate::core::command_def::{ArgDef, CommandDef, FlagDef, ValueChoice, ValueKind};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandSpec {
    /// Declarative command description used to build the plain completion tree.
    pub name: String,
    pub tooltip: Option<String>,
    pub sort: Option<String>,
    pub args: Vec<ArgNode>,
    pub flags: BTreeMap<String, FlagNode>,
    pub subcommands: Vec<CommandSpec>,
}

impl CommandSpec {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.sort = Some(sort.into());
        self
    }

    pub fn arg(mut self, arg: ArgNode) -> Self {
        self.args.push(arg);
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = ArgNode>) -> Self {
        self.args.extend(args);
        self
    }

    pub fn flag(mut self, name: impl Into<String>, flag: FlagNode) -> Self {
        self.flags.insert(name.into(), flag);
        self
    }

    pub fn flags(mut self, flags: impl IntoIterator<Item = (String, FlagNode)>) -> Self {
        self.flags.extend(flags);
        self
    }

    pub fn subcommand(mut self, subcommand: CommandSpec) -> Self {
        self.subcommands.push(subcommand);
        self
    }

    pub fn subcommands(mut self, subcommands: impl IntoIterator<Item = CommandSpec>) -> Self {
        self.subcommands.extend(subcommands);
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct CompletionTreeBuilder;

impl CompletionTreeBuilder {
    /// Build the immutable completion tree from higher-level command specs.
    ///
    /// The resulting structure is intentionally plain data so callers can cache
    /// it, augment it with plugin/provider hints, and pass it into the engine
    /// without keeping builder state alive.
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
pub struct ConfigKeySpec {
    pub key: String,
    pub tooltip: Option<String>,
    pub value_suggestions: Vec<SuggestionEntry>,
}

impl ConfigKeySpec {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            ..Self::default()
        }
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

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
    fn builds_nested_tree_from_specs() {
        let tree = build_tree();
        assert!(tree.root.children.contains_key("config"));
        assert!(
            tree.root
                .children
                .get("config")
                .and_then(|node| node.children.get("set"))
                .is_some()
        );
    }

    #[test]
    fn injects_config_key_nodes() {
        let mut tree = build_tree();
        CompletionTreeBuilder.apply_config_set_keys(
            &mut tree,
            [
                ConfigKeySpec::new("ui.format"),
                ConfigKeySpec::new("log.level"),
            ],
        );

        let set_node = &tree.root.children["config"].children["set"];
        assert!(set_node.children.contains_key("ui.format"));
        assert!(set_node.children.contains_key("log.level"));
        assert!(set_node.children["ui.format"].value_key);
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
