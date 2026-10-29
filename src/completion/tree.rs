//! Declarative builders for the completion engine's immutable tree model.
//!
//! Public API shape:
//!
//! - describe command surfaces with [`CompletionNode`]
//! - assemble them into a cached [`crate::completion::CompletionTree`] with
//!   [`CompletionTreeBuilder`]
//! - keep the resulting tree as plain data so the engine and embedders can
//!   reuse it without retaining builder state

use std::collections::BTreeMap;

use crate::completion::model::{
    ArgNode, CompletionNode, CompletionTree, FlagHints, FlagNode, SuggestionEntry,
};
use crate::core::clap_metadata as clap_meta;
use crate::core::plugin::{DescribeCommandV1, DescribeFlagV1, DescribeSuggestionV1};
use thiserror::Error;

/// Error returned when declarative completion metadata cannot be lowered into a
/// valid immutable tree.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompletionTreeBuildError {
    /// Two root command specs used the same command name.
    #[error("duplicate root command spec: {name}")]
    DuplicateRootCommand {
        /// Duplicate root command name.
        name: String,
    },
    /// Two injected `config set` key specs used the same key.
    #[error("duplicate config set key: {key}")]
    DuplicateConfigSetKey {
        /// Duplicate `config set` key name.
        key: String,
    },
}

impl CompletionNode {
    /// Creates a named command node.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Attaches the description shown alongside this command in completion UIs.
    ///
    /// If omitted, completion UIs show no description for this command.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Appends one positional argument definition.
    ///
    /// If omitted, the command contributes no positional completion metadata.
    pub fn arg(mut self, arg: ArgNode) -> Self {
        self.args.push(arg);
        self
    }

    /// Adds one flag definition keyed by its spelling.
    ///
    /// If omitted, the command contributes no flag completion metadata.
    pub fn flag(mut self, name: impl Into<String>, flag: FlagNode) -> Self {
        self.flags.insert(name.into(), flag);
        self
    }

    /// Extends the command with multiple flag definitions.
    ///
    /// If omitted, the command contributes no flag completion metadata.
    pub fn flags(mut self, flags: impl IntoIterator<Item = (String, FlagNode)>) -> Self {
        self.flags.extend(flags);
        self
    }

    /// Adds one named child command.
    ///
    /// # Panics
    ///
    /// Panics if this node already contains the child name.
    ///
    /// If omitted, the command contributes no nested subcommand metadata.
    pub fn subcommand(mut self, subcommand: CompletionNode) -> Self {
        let name = subcommand.name.clone();
        assert!(
            self.children.insert(name.clone(), subcommand).is_none(),
            "duplicate subcommand: {name}"
        );
        self
    }

    /// Extends the command with nested subcommands.
    ///
    /// If omitted, the command contributes no nested subcommand metadata.
    pub fn subcommands(mut self, subcommands: impl IntoIterator<Item = CompletionNode>) -> Self {
        for subcommand in subcommands {
            self = self.subcommand(subcommand);
        }
        self
    }
}

#[derive(Debug, Clone, Default)]
/// Builds immutable completion trees from command and config metadata.
///
/// This is the canonical builder surface for completion-tree construction.
pub struct CompletionTreeBuilder;

impl CompletionTreeBuilder {
    /// Assembles an immutable completion tree from named command nodes.
    ///
    /// The resulting structure is intentionally plain data so callers can cache
    /// it, augment it with plugin/provider hints, and pass it into the engine
    /// without keeping builder state alive.
    ///
    /// # Errors
    ///
    /// Returns [`CompletionTreeBuildError::DuplicateRootCommand`] when `specs`
    /// contains duplicate root command names.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{CompletionNode, CompletionTreeBuilder};
    ///
    /// let tree = CompletionTreeBuilder.build_from_specs(
    ///     &[CompletionNode::new("config")
    ///         .tooltip("Runtime configuration")
    ///         .subcommand(CompletionNode::new("set"))],
    ///     [("P".to_string(), "Project fields".to_string())],
    /// )
    /// .expect("tree should build");
    ///
    /// assert!(tree.root.children.contains_key("config"));
    /// assert!(tree.root.children["config"].children.contains_key("set"));
    /// assert_eq!(tree.pipe_verbs.get("P").map(String::as_str), Some("Project fields"));
    /// ```
    pub fn build_from_specs(
        &self,
        specs: &[CompletionNode],
        pipe_verbs: impl IntoIterator<Item = (String, String)>,
    ) -> Result<CompletionTree, CompletionTreeBuildError> {
        let mut root = CompletionNode::default();
        for spec in specs {
            let name = spec.name.clone();
            let node = spec.clone();
            if root.children.insert(name.clone(), node).is_some() {
                return Err(CompletionTreeBuildError::DuplicateRootCommand { name });
            }
        }

        Ok(CompletionTree {
            root,
            pipe_verbs: pipe_verbs.into_iter().collect(),
        })
    }

    /// Injects `config set` key completions into an existing tree.
    ///
    /// This is a no-op when the tree does not contain a `config set` path.
    ///
    /// # Errors
    ///
    /// Returns [`CompletionTreeBuildError::DuplicateConfigSetKey`] when `keys`
    /// contains duplicate config-set key names.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::completion::{CompletionNode, CompletionTreeBuilder, ConfigKeySpec};
    ///
    /// let mut tree = CompletionTreeBuilder.build_from_specs(
    ///     &[CompletionNode::new("config").subcommand(CompletionNode::new("set"))],
    ///     [],
    /// )
    /// .expect("tree should build");
    /// CompletionTreeBuilder.apply_config_set_keys(
    ///     &mut tree,
    ///     [
    ///         ConfigKeySpec::new("ui.format"),
    ///         ConfigKeySpec::new("log.level"),
    ///     ],
    /// )
    /// .expect("config keys should inject");
    ///
    /// let set_node = &tree.root.children["config"].children["set"];
    /// assert!(set_node.children.contains_key("ui.format"));
    /// assert!(set_node.children.contains_key("log.level"));
    /// assert!(set_node.children["ui.format"].value_key);
    /// ```
    pub(crate) fn apply_config_set_keys(
        &self,
        tree: &mut CompletionTree,
        keys: impl IntoIterator<Item = ConfigKeySpec>,
    ) -> Result<(), CompletionTreeBuildError> {
        let Some(config_node) = tree.root.children.get_mut("config") else {
            return Ok(());
        };
        let Some(set_node) = config_node.children.get_mut("set") else {
            return Ok(());
        };
        let set_flags = set_node.flags.clone();

        for key in keys {
            let key_name = key.key.clone();
            let mut node = CompletionNode {
                tooltip: key.tooltip,
                value_key: true,
                flags: set_flags.clone(),
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
            if set_node.children.insert(key_name.clone(), node).is_some() {
                return Err(CompletionTreeBuildError::DuplicateConfigSetKey { key: key_name });
            }
        }

        Ok(())
    }
}

impl CompletionNode {
    /// Projects built clap grammar into completion metadata.
    ///
    /// Automatic help flags are recognized without adding menu suggestions.
    pub fn from_clap(command: &clap::Command) -> Self {
        let mut grammar = command.clone();
        grammar.build();
        Self::from_built_clap(&grammar, command)
    }

    fn from_built_clap(command: &clap::Command, declared: &clap::Command) -> Self {
        let mut required_common = Vec::new();
        let flags = command
            .get_arguments()
            .filter(|arg| !arg.is_positional() && !arg.is_hide_set())
            .flat_map(|arg| {
                let spellings = clap_meta::flag_spellings(arg);
                let preferred = spellings.first().cloned();
                if arg.is_required_set() {
                    required_common.extend(preferred.clone());
                }
                spellings.into_iter().enumerate().map(move |(index, name)| {
                    (
                        name,
                        FlagNode {
                            hidden: matches!(
                                arg.get_action(),
                                clap::ArgAction::Help
                                    | clap::ArgAction::HelpShort
                                    | clap::ArgAction::HelpLong
                            ),
                            alias_of: (index > 0).then(|| preferred.clone()).flatten(),
                            tooltip: clap_meta::arg_help(arg),
                            flag_only: !arg.get_action().takes_values(),
                            multi: clap_meta::is_multi(arg),
                            value_type: clap_meta::is_path(arg)
                                .then_some(crate::completion::ValueType::Path),
                            suggestions: clap_suggestions(arg),
                            ..FlagNode::default()
                        },
                    )
                })
            })
            .collect();
        // Native metadata previously crossed the sorted wire flag map.
        required_common.sort();
        // Keep declared command paths: the REPL already owns help navigation.
        let names = declared
            .get_subcommands()
            .filter(|child| !child.is_hide_set())
            .map(|child| child.get_name().to_owned())
            .collect::<Vec<_>>();
        let word_resolver = (!names.is_empty()).then(|| {
            crate::completion::CommandWordResolver::new(move |word| {
                crate::command_words::resolve(word, names.iter().map(String::as_str))
                    .ok()
                    .flatten()
            })
        });
        Self {
            word_resolver,
            name: command.get_name().to_string(),
            tooltip: clap_meta::plain(command.get_about()),
            args: command
                .get_positionals()
                .filter(|arg| !arg.is_hide_set())
                .map(|arg| ArgNode {
                    name: Some(clap_meta::arg_name(arg)),
                    tooltip: clap_meta::arg_help(arg),
                    required: arg.is_required_set(),
                    multi: clap_meta::is_multi(arg),
                    value_type: clap_meta::is_path(arg)
                        .then_some(crate::completion::ValueType::Path),
                    suggestions: clap_suggestions(arg),
                    catalogs: Vec::new(),
                })
                .collect(),
            flags,
            flag_hints: required_hints(required_common),
            children: declared
                .get_subcommands()
                .filter(|child| !child.is_hide_set())
                .flat_map(|child| {
                    std::iter::once(child.get_name())
                        .chain(child.get_visible_aliases())
                        .map(move |name| {
                            let grammar = command
                                .find_subcommand(child.get_name())
                                .expect("built grammar retains declared subcommands");
                            let mut node = Self::from_built_clap(grammar, child);
                            node.name = name.to_owned();
                            (name.to_owned(), node)
                        })
                })
                .collect(),
            ..Self::default()
        }
    }

    /// Projects external plugin metadata at the describe boundary.
    pub fn from_describe(command: &DescribeCommandV1) -> Self {
        // Alias metadata belongs to the preferred spelling. Preserve that rule
        // even for plugins whose aliases carry different suggestions or help.
        let mut groups = BTreeMap::<String, DescribeFlagGroup<'_>>::new();
        for (name, flag) in &command.flags {
            if flag.alias_of.is_none() && name.starts_with('-') {
                groups.insert(
                    name.clone(),
                    DescribeFlagGroup {
                        flag,
                        spellings: normalized_flag_name(name).into_iter().collect(),
                    },
                );
            }
        }
        for (name, flag) in &command.flags {
            if let Some(preferred) = &flag.alias_of {
                if let Some(group) = groups.get_mut(preferred) {
                    group.spellings.push(name.clone());
                } else if name.starts_with('-') {
                    groups.insert(
                        name.clone(),
                        DescribeFlagGroup {
                            flag,
                            spellings: normalized_flag_name(name).into_iter().collect(),
                        },
                    );
                }
            }
        }
        let mut required_common = Vec::new();
        let mut flags = BTreeMap::new();
        for DescribeFlagGroup { flag, spellings } in groups.into_values() {
            let preferred = spellings.first().cloned();
            if flag.required {
                required_common.extend(preferred.clone());
            }
            for (index, name) in spellings.into_iter().enumerate() {
                flags.insert(
                    name,
                    FlagNode {
                        alias_of: (index > 0).then(|| preferred.clone()).flatten(),
                        tooltip: flag.about.clone(),
                        flag_only: flag.flag_only,
                        multi: flag.multi,
                        value_type: flag.value_type.map(|_| crate::completion::ValueType::Path),
                        suggestions: flag
                            .suggestions
                            .iter()
                            .map(suggestion_from_describe)
                            .collect(),
                        ..FlagNode::default()
                    },
                );
            }
        }
        Self {
            name: command.name.clone(),
            tooltip: (!command.about.trim().is_empty()).then(|| command.about.clone()),
            args: command
                .args
                .iter()
                .map(|arg| ArgNode {
                    name: Some(arg.name.clone().unwrap_or_else(|| "value".to_string())),
                    tooltip: arg.about.clone(),
                    required: arg.required,
                    multi: arg.multi,
                    value_type: arg.value_type.map(|_| crate::completion::ValueType::Path),
                    suggestions: arg
                        .suggestions
                        .iter()
                        .map(suggestion_from_describe)
                        .collect(),
                    catalogs: Vec::new(),
                })
                .collect(),
            flags,
            flag_hints: required_hints(required_common),
            children: command
                .subcommands
                .iter()
                .map(|child| (child.name.clone(), Self::from_describe(child)))
                .collect(),
            ..Self::default()
        }
    }
}

struct DescribeFlagGroup<'a> {
    flag: &'a DescribeFlagV1,
    spellings: Vec<String>,
}

fn normalized_flag_name(name: &str) -> Option<String> {
    if name.starts_with("--") {
        Some(name.to_string())
    } else {
        name.strip_prefix('-')?
            .chars()
            .next()
            .map(|short| format!("-{short}"))
    }
}

fn required_hints(required_common: Vec<String>) -> Option<FlagHints> {
    (!required_common.is_empty()).then_some(FlagHints {
        required_common,
        ..FlagHints::default()
    })
}

fn clap_suggestions(arg: &clap::Arg) -> Vec<SuggestionEntry> {
    arg.get_possible_values()
        .into_iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| SuggestionEntry {
            value: value.get_name().to_string(),
            meta: value.get_help().map(ToString::to_string),
            display: None,
            sort: None,
            aliases: Vec::new(),
        })
        .collect()
}

fn suggestion_from_describe(value: &DescribeSuggestionV1) -> SuggestionEntry {
    SuggestionEntry {
        value: value.value.clone(),
        meta: value.meta.clone(),
        display: value.display.clone(),
        sort: value.sort.clone(),
        aliases: Vec::new(),
    }
}

/// Declarative `config set` key metadata used for completion nodes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub(crate) struct ConfigKeySpec {
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
    ///
    /// If omitted, completion UIs show no description for the key.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Replaces the suggested values for this config key.
    ///
    /// If omitted, the key contributes no value suggestions.
    pub fn value_suggestions(
        mut self,
        suggestions: impl IntoIterator<Item = SuggestionEntry>,
    ) -> Self {
        self.value_suggestions = suggestions.into_iter().collect();
        self
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {

    #[test]
    fn internal_example_1() {
        use crate::completion::{CompletionNode, CompletionTreeBuilder, ConfigKeySpec};

        let mut tree = CompletionTreeBuilder
            .build_from_specs(
                &[CompletionNode::new("config").subcommand(CompletionNode::new("set"))],
                [],
            )
            .expect("tree should build");
        CompletionTreeBuilder
            .apply_config_set_keys(
                &mut tree,
                [
                    ConfigKeySpec::new("ui.format"),
                    ConfigKeySpec::new("log.level"),
                ],
            )
            .expect("config keys should inject");

        let set_node = &tree.root.children["config"].children["set"];
        assert!(set_node.children.contains_key("ui.format"));
        assert!(set_node.children.contains_key("log.level"));
        assert!(set_node.children["ui.format"].value_key);
    }

    use crate::completion::model::CompletionTree;

    use super::{CompletionNode, CompletionTreeBuildError, CompletionTreeBuilder, ConfigKeySpec};

    fn build_tree() -> CompletionTree {
        CompletionTreeBuilder
            .build_from_specs(
                &[CompletionNode::new("config").subcommand(CompletionNode::new("set"))],
                [("F".to_string(), "Filter".to_string())],
            )
            .expect("tree should build")
    }

    #[test]
    fn duplicate_root_specs_return_an_error() {
        let err = CompletionTreeBuilder
            .build_from_specs(
                &[CompletionNode::new("config"), CompletionNode::new("config")],
                [],
            )
            .expect_err("duplicate root command should fail");

        assert_eq!(
            err,
            CompletionTreeBuildError::DuplicateRootCommand {
                name: "config".to_string()
            }
        );
    }

    #[test]
    fn duplicate_config_keys_return_an_error() {
        let mut tree = build_tree();
        let err = CompletionTreeBuilder
            .apply_config_set_keys(
                &mut tree,
                [
                    ConfigKeySpec::new("ui.format"),
                    ConfigKeySpec::new("ui.format"),
                ],
            )
            .expect_err("duplicate config key should fail");

        assert_eq!(
            err,
            CompletionTreeBuildError::DuplicateConfigSetKey {
                key: "ui.format".to_string()
            }
        );
    }
}
