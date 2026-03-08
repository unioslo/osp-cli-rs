use crate::completion::{ArgNode, CommandSpec, FlagNode, SuggestionEntry, ValueType};
use crate::core::plugin::{DescribeArgV1, DescribeCommandV1, DescribeFlagV1, DescribeSuggestionV1};

pub(super) fn to_command_spec(command: &DescribeCommandV1) -> CommandSpec {
    let spec = CommandSpec::new(&command.name)
        .args(command.args.iter().map(to_arg_node))
        .flags(
            command
                .flags
                .iter()
                .map(|(name, flag)| (name.clone(), to_flag_node(flag))),
        )
        .subcommands(command.subcommands.iter().map(to_command_spec));

    if command.about.trim().is_empty() {
        spec
    } else {
        spec.tooltip(&command.about)
    }
}

pub(super) fn to_arg_node(arg: &DescribeArgV1) -> ArgNode {
    let mut node = ArgNode::default().suggestions(arg.suggestions.iter().map(to_suggestion_entry));
    if let Some(name) = &arg.name {
        node.name = Some(name.clone());
    }
    if let Some(about) = &arg.about {
        node = node.tooltip(about);
    }
    if arg.multi {
        node = node.multi();
    }
    if let Some(value_type) = arg.value_type.and_then(to_value_type) {
        node = node.value_type(value_type);
    }
    node
}

pub(super) fn to_flag_node(flag: &DescribeFlagV1) -> FlagNode {
    let mut node = FlagNode::new().suggestions(flag.suggestions.iter().map(to_suggestion_entry));
    if let Some(about) = &flag.about {
        node = node.tooltip(about);
    }
    if flag.flag_only {
        node = node.flag_only();
    }
    if flag.multi {
        node = node.multi();
    }
    if let Some(value_type) = flag.value_type.and_then(to_value_type) {
        node = node.value_type(value_type);
    }
    node
}

pub(super) fn to_suggestion_entry(entry: &DescribeSuggestionV1) -> SuggestionEntry {
    SuggestionEntry {
        value: entry.value.clone(),
        meta: entry.meta.clone(),
        display: entry.display.clone(),
        sort: entry.sort.clone(),
    }
}

pub(super) fn to_value_type(
    value_type: crate::core::plugin::DescribeValueTypeV1,
) -> Option<ValueType> {
    match value_type {
        crate::core::plugin::DescribeValueTypeV1::Path => Some(ValueType::Path),
    }
}

pub(super) fn direct_subcommand_names(spec: &CommandSpec) -> Vec<String> {
    spec.subcommands
        .iter()
        .map(|subcommand| subcommand.name.clone())
        .collect()
}

pub(super) fn collect_completion_words(spec: &CommandSpec) -> Vec<String> {
    let mut words = vec![spec.name.clone()];
    for flag in spec.flags.keys() {
        words.push(flag.clone());
    }
    for subcommand in &spec.subcommands {
        words.extend(collect_completion_words(subcommand));
    }
    words
}
