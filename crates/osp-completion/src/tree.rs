use std::collections::BTreeMap;

use crate::model::{ArgNode, CompletionNode, CompletionTree, FlagNode, SuggestionEntry};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: String,
    pub tooltip: Option<String>,
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
    pub fn build_from_specs(
        &self,
        specs: &[CommandSpec],
        pipe_verbs: impl IntoIterator<Item = (String, String)>,
    ) -> CompletionTree {
        let mut root = CompletionNode::default();
        for spec in specs {
            root.children
                .insert(spec.name.clone(), Self::node_from_spec(spec));
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
            set_node.children.insert(key.key, node);
        }
    }

    fn node_from_spec(spec: &CommandSpec) -> CompletionNode {
        let mut node = CompletionNode {
            tooltip: spec.tooltip.clone(),
            args: spec.args.clone(),
            flags: spec.flags.clone(),
            ..CompletionNode::default()
        };

        for subcommand in &spec.subcommands {
            node.children
                .insert(subcommand.name.clone(), Self::node_from_spec(subcommand));
        }

        node
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
    use crate::model::CompletionTree;

    use super::{CommandSpec, CompletionTreeBuilder, ConfigKeySpec};

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
}
