//! Completion-tree path resolution and command-derived context hints.
//!
//! The parser tells completion "what tokens exist near the cursor". This
//! module answers the next question: which completion-tree node and flag scope
//! those tokens imply, and which provider hints should influence the later
//! suggestion pass.

use std::collections::BTreeSet;

use crate::completion::model::{
    CommandLine, CompletionContext, CompletionNode, CompletionTree, PlanningHints,
    PlanningNumericColumn, PlanningRow, PlanningTable, PlanningValue,
};
use crate::core::fuzzy::fold_case;

pub(crate) struct ResolvedNodes<'a> {
    pub(crate) context_node: &'a CompletionNode,
    pub(crate) flag_scope_node: &'a CompletionNode,
}

pub(crate) struct TreeResolver<'a> {
    tree: &'a CompletionTree,
}

impl<'a> TreeResolver<'a> {
    pub(crate) fn new(tree: &'a CompletionTree) -> Self {
        Self { tree }
    }

    pub(crate) fn matched_command_len_tokens(&self, tokens: &[String]) -> usize {
        let mut node = &self.tree.root;
        let mut matched = 0usize;

        for token in tokens {
            if token == "|" || token.starts_with('-') {
                break;
            }
            let Some(child) = node.children.get(token) else {
                break;
            };
            matched += 1;
            if child.value_key || child.value_leaf {
                break;
            }
            node = child;
        }

        matched
    }

    pub(crate) fn resolved_nodes(&self, context: &CompletionContext) -> ResolvedNodes<'a> {
        ResolvedNodes {
            context_node: self.resolve_exact_or_root(&context.matched_path),
            flag_scope_node: self.resolve_exact_or_root(&context.flag_scope_path),
        }
    }

    pub(crate) fn resolve_exact_or_root(&self, path: &[String]) -> &'a CompletionNode {
        self.resolve_exact(path).unwrap_or(&self.tree.root)
    }

    pub(crate) fn resolve_flag_scope_path(&self, matched_path: &[String]) -> Vec<String> {
        let floor = if matched_path.is_empty() { 0 } else { 1 };
        for i in (floor..=matched_path.len()).rev() {
            let prefix = &matched_path[..i];
            let Some(node) = self.resolve_exact(prefix) else {
                continue;
            };
            if !node.flags.is_empty() {
                return prefix.to_vec();
            }
        }
        if matched_path.is_empty() {
            Vec::new()
        } else {
            matched_path.to_vec()
        }
    }

    pub(crate) fn resolve_context(&self, path: &[String]) -> (&'a CompletionNode, Vec<String>) {
        let mut node = &self.tree.root;
        let mut matched = Vec::new();

        for segment in path {
            let Some(next) = node.children.get(segment) else {
                break;
            };
            node = next;
            matched.push(segment.clone());
            if node.value_leaf {
                break;
            }
        }

        (node, matched)
    }

    pub(crate) fn resolve_exact(&self, path: &[String]) -> Option<&'a CompletionNode> {
        let (node, matched) = self.resolve_context(path);
        (matched.len() == path.len()).then_some(node)
    }
}

/// Returns rows whose declared identity values are compatible with the command.
///
/// This separates runtime-lane scope from the row's other planning facts. A
/// table with no represented identity lane is therefore unknown for the
/// request instead of excluding a different provider or runtime.
pub(crate) fn planning_scope_rows<'a>(
    hints: &PlanningHints,
    table: &'a PlanningTable,
    cmd: &CommandLine,
) -> Vec<&'a PlanningRow> {
    table
        .rows
        .iter()
        .filter(|row| planning_row_in_scope(hints, row, cmd))
        .collect()
}

pub(crate) fn matching_scoped_planning_rows<'a>(
    hints: &PlanningHints,
    table: &'a PlanningTable,
    cmd: &CommandLine,
) -> Vec<&'a PlanningRow> {
    planning_scope_rows(hints, table, cmd)
        .into_iter()
        .filter(|row| planning_row_has_known_requested_identity(hints, row, cmd))
        .filter(|row| planning_row_fits(hints, table, row, cmd))
        .collect()
}

pub(crate) fn planning_scope_is_known(
    hints: &PlanningHints,
    table: &PlanningTable,
    cmd: &CommandLine,
) -> bool {
    planning_scope_rows(hints, table, cmd)
        .into_iter()
        .all(|row| planning_row_has_known_requested_identity(hints, row, cmd))
}

pub(crate) fn planning_column_for_flag<'a>(
    table: &'a PlanningTable,
    flag: &str,
) -> Option<&'a str> {
    planning_column(table, flag)
}

pub(crate) fn planning_row_value<'a>(
    row: &'a PlanningRow,
    column: &str,
) -> Option<&'a PlanningValue> {
    row.values.get(column).or_else(|| {
        row.values
            .iter()
            .find(|(candidate, _)| column_matches_flag(candidate, column))
            .map(|(_, value)| value)
    })
}

fn planning_row_fits(
    hints: &PlanningHints,
    table: &PlanningTable,
    row: &PlanningRow,
    cmd: &CommandLine,
) -> bool {
    cmd.flag_values_map().iter().all(|(flag, _values)| {
        let Some(column) = planning_column_for_flag(table, flag) else {
            return true;
        };
        let requested = planning_requested_values(hints, cmd, column);
        if requested.is_empty() {
            return true;
        }
        requested
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .all(|value| {
                let cell = planning_row_value(row, column);
                if let Some(numeric) = table.minimum_columns.get(column) {
                    planning_minimum_matches(cell, value, numeric)
                } else {
                    planning_exact_matches(cell, value)
                }
            })
    })
}

fn planning_column<'a>(table: &'a PlanningTable, flag: &str) -> Option<&'a str> {
    table
        .minimum_columns
        .keys()
        .chain(table.columns.iter())
        .find(|column| column_matches_flag(column, flag))
        .map(String::as_str)
}

fn column_matches_flag(column: &str, flag: &str) -> bool {
    column == flag || column.trim_start_matches('-') == flag.trim_start_matches('-')
}

fn planning_requested_values<'a>(
    hints: &PlanningHints,
    cmd: &'a CommandLine,
    column: &str,
) -> Vec<&'a str> {
    let provider_column = hints.provider_column.as_deref();
    let provider_flag = provider_column
        .filter(|provider| column_matches_flag(provider, column))
        .and_then(|_| {
            cmd.flag_values_map().keys().find(|flag| {
                provider_column.is_some_and(|provider| column_matches_flag(provider, flag))
            })
        });

    let mut requested = Vec::new();
    for (flag, values) in cmd.flag_values_map() {
        if !column_matches_flag(column, flag) {
            continue;
        }
        if provider_flag == Some(flag) {
            requested.extend(
                values
                    .iter()
                    .filter_map(|value| provider_selector_part(hints, value, column)),
            );
        } else {
            requested.extend(values.iter().map(String::as_str));
        }
    }

    if provider_column.is_some_and(|provider| !column_matches_flag(provider, column)) {
        let identity_index = hints
            .identity_columns
            .iter()
            .filter(|identity| {
                !provider_column.is_some_and(|provider| column_matches_flag(provider, identity))
            })
            .position(|identity| column_matches_flag(identity, column));
        if let Some(identity_index) = identity_index
            && let Some(provider_flag) = provider_column.and_then(|provider| {
                cmd.flag_values_map()
                    .keys()
                    .find(|flag| column_matches_flag(provider, flag))
            })
        {
            requested.extend(
                cmd.flag_values(provider_flag)
                    .into_iter()
                    .flatten()
                    .filter_map(|value| provider_selector_part_at(value, identity_index + 1)),
            );
        }
    }

    requested
}

fn provider_selector_part<'a>(
    hints: &PlanningHints,
    value: &'a str,
    column: &str,
) -> Option<&'a str> {
    let provider = hints.provider_column.as_deref()?;
    if column_matches_flag(provider, column) {
        return provider_selector_part_at(value, 0);
    }
    let identity_index = hints
        .identity_columns
        .iter()
        .filter(|identity| !column_matches_flag(provider, identity))
        .position(|identity| column_matches_flag(identity, column))?;
    provider_selector_part_at(value, identity_index + 1)
}

fn provider_selector_part_at(value: &str, index: usize) -> Option<&str> {
    let part = value.split(':').nth(index)?.trim();
    (!part.is_empty()).then_some(part)
}

fn planning_exact_matches(cell: Option<&PlanningValue>, typed: &str) -> bool {
    let Some(cell) = cell else {
        return true;
    };
    match cell {
        PlanningValue::Text(value) if value.trim().is_empty() => true,
        PlanningValue::Text(value) => fold_case(value) == fold_case(typed),
        PlanningValue::Number(value) => typed
            .trim()
            .parse::<i64>()
            .map_or(true, |typed| typed == *value),
        PlanningValue::Unknown => true,
    }
}

fn planning_minimum_matches(
    cell: Option<&PlanningValue>,
    typed: &str,
    numeric: &PlanningNumericColumn,
) -> bool {
    let Some(requested) = parse_scaled_integer(typed, numeric) else {
        return true;
    };
    let Some(cell) = cell else {
        return true;
    };
    let Some(available) = planning_integer(cell, numeric) else {
        return true;
    };
    available >= requested
}

fn planning_integer(value: &PlanningValue, numeric: &PlanningNumericColumn) -> Option<i64> {
    match value {
        PlanningValue::Number(value) => Some(*value),
        PlanningValue::Text(value) => parse_scaled_integer(value, numeric),
        PlanningValue::Unknown => None,
    }
}

fn parse_scaled_integer(raw: &str, numeric: &PlanningNumericColumn) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let bytes = raw.as_bytes();
    let mut number_end = 0;
    if matches!(bytes.first(), Some(b'-' | b'+')) {
        number_end = 1;
    }
    while bytes
        .get(number_end)
        .is_some_and(|byte| byte.is_ascii_digit())
    {
        number_end += 1;
    }
    if number_end == 0
        || (number_end == 1
            && bytes
                .first()
                .is_some_and(|byte| *byte == b'-' || *byte == b'+'))
    {
        return None;
    }

    let value = raw[..number_end].parse::<i64>().ok()?;
    let suffix = raw[number_end..].trim();
    let scale = if suffix.is_empty() {
        1
    } else {
        numeric
            .unit_scales
            .iter()
            .find(|(unit, _)| unit.eq_ignore_ascii_case(suffix))
            .map(|(_, scale)| *scale)?
    };
    if scale <= 0 {
        return None;
    }
    value.checked_mul(scale)
}

fn table_has_explicit_filter(table: &PlanningTable, cmd: &CommandLine) -> bool {
    cmd.flag_values_map()
        .keys()
        .any(|flag| planning_column_for_flag(table, flag).is_some())
}

fn planning_row_in_scope(hints: &PlanningHints, row: &PlanningRow, cmd: &CommandLine) -> bool {
    let identity_matches = |column: &str| {
        planning_requested_values(hints, cmd, column)
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .all(|value| planning_exact_matches(planning_row_value(row, column), value))
    };

    if !hints
        .identity_columns
        .iter()
        .all(|column| identity_matches(column))
    {
        return false;
    }

    hints.provider_column.as_deref().is_none_or(|column| {
        hints
            .identity_columns
            .iter()
            .any(|identity| column_matches_flag(identity, column))
            || identity_matches(column)
    })
}

fn planning_row_has_known_requested_identity(
    hints: &PlanningHints,
    row: &PlanningRow,
    cmd: &CommandLine,
) -> bool {
    hints
        .identity_columns
        .iter()
        .filter(|column| {
            !hints
                .provider_column
                .as_deref()
                .is_some_and(|provider| column_matches_flag(column, provider))
        })
        .all(|column| {
            planning_requested_values(hints, cmd, column)
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .all(|_| planning_value_is_known(planning_row_value(row, column)))
        })
}

fn planning_row_has_unknown_requested_identity(
    hints: &PlanningHints,
    row: &PlanningRow,
    cmd: &CommandLine,
) -> bool {
    hints
        .identity_columns
        .iter()
        .filter(|column| {
            !hints
                .provider_column
                .as_deref()
                .is_some_and(|provider| column_matches_flag(column, provider))
        })
        .any(|column| {
            planning_requested_values(hints, cmd, column)
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .any(|_| !planning_value_is_known(planning_row_value(row, column)))
        })
}

fn planning_value_is_known(value: Option<&PlanningValue>) -> bool {
    match value {
        Some(PlanningValue::Text(value)) => !value.trim().is_empty(),
        Some(PlanningValue::Number(_)) => true,
        Some(PlanningValue::Unknown) | None => false,
    }
}

fn planning_provider_value<'a>(hints: &'a PlanningHints, row: &'a PlanningRow) -> Option<&'a str> {
    let column = hints.provider_column.as_deref()?;
    match planning_row_value(row, column) {
        Some(PlanningValue::Text(value)) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

fn planning_provider_values(node: &CompletionNode) -> BTreeSet<&str> {
    let Some(hints) = node.planning.as_ref() else {
        return BTreeSet::new();
    };
    hints
        .tables
        .iter()
        .flat_map(|table| table.rows.iter())
        .filter_map(|row| planning_provider_value(hints, row))
        .collect()
}

fn planning_provider_candidates<'a>(
    node: &'a CompletionNode,
    cmd: &CommandLine,
    all: &BTreeSet<&'a str>,
) -> Option<BTreeSet<&'a str>> {
    let hints = node.planning.as_ref()?;
    if hints.provider_column.is_none() || all.is_empty() {
        return None;
    }

    let mut represented = BTreeSet::new();
    let mut matching = BTreeSet::new();
    for table in &hints.tables {
        if !table.exhaustive || !table_has_explicit_filter(table, cmd) {
            continue;
        }
        let scoped_rows = planning_scope_rows(hints, table, cmd);
        let known_scope = scoped_rows
            .iter()
            .filter_map(|row| planning_provider_value(hints, row))
            .filter_map(|provider| known_provider(all, provider));
        let scoped_providers = known_scope.collect::<BTreeSet<_>>();
        if scoped_providers.is_empty() {
            continue;
        }
        represented.extend(scoped_providers.iter().copied());

        for row in scoped_rows {
            let Some(provider) = planning_provider_value(hints, row) else {
                return Some(all.clone());
            };
            if let Some(known) = known_provider(all, provider)
                && (planning_row_has_unknown_requested_identity(hints, row, cmd)
                    || planning_row_fits(hints, table, row, cmd))
            {
                matching.insert(known);
            }
        }
    }

    if represented.is_empty() {
        return None;
    }

    let mut allowed = all
        .difference(&represented)
        .copied()
        .collect::<BTreeSet<_>>();
    allowed.extend(matching);
    Some(allowed)
}

fn known_provider<'a>(all: &BTreeSet<&'a str>, value: &str) -> Option<&'a str> {
    all.iter()
        .copied()
        .find(|known| fold_case(known) == fold_case(value))
}

pub(crate) struct ProviderSelection<'a> {
    explicit: Option<&'a str>,
    candidates: BTreeSet<&'a str>,
    all: BTreeSet<&'a str>,
}

impl<'a> ProviderSelection<'a> {
    pub(crate) fn from_command(cmd: &'a CommandLine, node: &'a CompletionNode) -> Self {
        let hints = node.planning.as_ref();
        let explicit = cmd
            .flag_values("--provider")
            .and_then(|values| values.first())
            .map(String::as_str)
            .map(|value| {
                hints
                    .filter(|hints| hints.provider_column.is_some())
                    .and_then(|_| provider_selector_part_at(value, 0))
                    .unwrap_or(value)
            })
            .filter(|value| !value.trim().is_empty());

        let flag_hints = node.flag_hints.as_ref();
        let planning_providers = planning_provider_values(node);
        let all = flag_hints
            .into_iter()
            .flat_map(|hints| {
                hints
                    .by_provider
                    .keys()
                    .chain(hints.required_by_provider.keys())
                    .map(String::as_str)
            })
            .chain(planning_providers)
            .collect::<BTreeSet<_>>();

        let mut candidates = explicit.map_or_else(
            || all.clone(),
            |provider| {
                if let Some(known) = known_provider(&all, provider) {
                    BTreeSet::from([known])
                } else {
                    all.clone()
                }
            },
        );

        let Some(hints) = flag_hints else {
            if let Some(planning) = planning_provider_candidates(node, cmd, &all) {
                candidates = candidates.intersection(&planning).copied().collect();
            }
            return Self {
                explicit,
                candidates,
                all,
            };
        };

        if explicit.is_some() {
            if let Some(planning) = planning_provider_candidates(node, cmd, &all) {
                candidates = candidates.intersection(&planning).copied().collect();
            }
            return Self {
                explicit,
                candidates,
                all,
            };
        }

        let common = hints
            .common
            .iter()
            .chain(&hints.required_common)
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut candidates = all.clone();
        let mut constrained = false;

        for (flag, values) in cmd.flag_values_map() {
            if flag == "--provider" {
                continue;
            }

            if !common.contains(flag.as_str()) {
                let compatible =
                    all.iter()
                        .copied()
                        .filter(|provider| {
                            hints.by_provider.get(*provider).is_some_and(|flags| {
                                flags.iter().any(|candidate| candidate == flag)
                            }) || hints
                                .required_by_provider
                                .get(*provider)
                                .is_some_and(|flags| {
                                    flags.iter().any(|candidate| candidate == flag)
                                })
                        })
                        .collect::<BTreeSet<_>>();
                if !compatible.is_empty() {
                    constrained = true;
                    candidates = candidates
                        .intersection(&compatible)
                        .copied()
                        .collect::<BTreeSet<_>>();
                }
            }

            let Some(flag_node) = node.flags.get(flag) else {
                continue;
            };
            for value in values.iter().filter(|value| !value.is_empty()) {
                let value = fold_case(value);
                let matching = all
                    .iter()
                    .copied()
                    .filter(|provider| {
                        flag_node
                            .exhaustive_suggestions_by_provider
                            .contains(*provider)
                            && flag_node
                                .suggestions_by_provider
                                .get(*provider)
                                .is_some_and(|entries| {
                                    entries
                                        .iter()
                                        .any(|entry| fold_case(&entry.value).starts_with(&value))
                                })
                    })
                    .collect::<BTreeSet<_>>();
                // An unknown value is still valid input for the backend; it
                // contributes no completion constraint.
                if matching.is_empty() {
                    continue;
                }
                let compatible = all
                    .iter()
                    .copied()
                    .filter(|provider| {
                        // Advisory or free-form choices are not a closed
                        // catalog, so completion cannot exclude that provider.
                        matching.contains(provider)
                            || !flag_node
                                .exhaustive_suggestions_by_provider
                                .contains(*provider)
                    })
                    .collect::<BTreeSet<_>>();
                constrained = true;
                candidates = candidates
                    .intersection(&compatible)
                    .copied()
                    .collect::<BTreeSet<_>>();
            }
        }

        if constrained && candidates.is_empty() {
            candidates = all.clone();
        }

        if let Some(planning) = planning_provider_candidates(node, cmd, &all) {
            candidates = candidates.intersection(&planning).copied().collect();
        }

        Self {
            explicit,
            candidates,
            all,
        }
    }

    pub(crate) fn name(&self) -> Option<&'a str> {
        if let Some(provider) = self.explicit {
            return (self.candidates.is_empty()
                || self
                    .candidates
                    .iter()
                    .any(|known| fold_case(known) == fold_case(provider)))
            .then_some(provider);
        }
        (self.candidates.len() == 1)
            .then(|| self.candidates.first().copied())
            .flatten()
    }

    pub(crate) fn candidates(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.candidates.iter().copied()
    }

    pub(crate) fn hides_selector(&self) -> bool {
        self.explicit.is_some() || self.candidates.len() == 1
    }

    pub(crate) fn planning_table_applies_to_selected_provider(
        &self,
        hints: &PlanningHints,
        table: &PlanningTable,
        cmd: &CommandLine,
    ) -> bool {
        let scoped_rows = planning_scope_rows(hints, table, cmd)
            .into_iter()
            .filter(|row| planning_row_has_known_requested_identity(hints, row, cmd))
            .collect::<Vec<_>>();
        if scoped_rows.is_empty() {
            return false;
        }
        if hints.provider_column.is_none() {
            return true;
        }
        let Some(selected) = self.name() else {
            return false;
        };
        scoped_rows
            .iter()
            .all(|row| planning_provider_value(hints, row).is_some())
            && scoped_rows.iter().any(|row| {
                planning_provider_value(hints, row)
                    .is_some_and(|provider| fold_case(provider) == fold_case(selected))
            })
    }

    pub(crate) fn allows_planning_row(&self, hints: &PlanningHints, row: &PlanningRow) -> bool {
        let Some(provider) = planning_provider_value(hints, row) else {
            return true;
        };
        if self.all.is_empty() {
            return true;
        }
        if let Some(explicit) = self.explicit {
            if !self
                .all
                .iter()
                .any(|known| fold_case(known) == fold_case(explicit))
            {
                return true;
            }
            return self
                .candidates
                .iter()
                .any(|known| fold_case(known) == fold_case(provider));
        }
        self.candidates
            .iter()
            .any(|known| fold_case(known) == fold_case(provider))
    }
}

/// Result of the shared provider-narrowing pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderNarrowing {
    explicit: Option<String>,
    candidates: BTreeSet<String>,
    all: BTreeSet<String>,
}

impl ProviderNarrowing {
    /// Returns the provider value explicitly present in the command, if any.
    pub fn explicit(&self) -> Option<&str> {
        self.explicit.as_deref()
    }

    /// Returns providers still compatible with the command.
    pub fn candidates(&self) -> impl Iterator<Item = &str> {
        self.candidates.iter().map(String::as_str)
    }

    /// Returns all provider values known to the completion metadata.
    pub fn all(&self) -> impl Iterator<Item = &str> {
        self.all.iter().map(String::as_str)
    }

    /// Whether known exhaustive facts ruled out every known provider.
    pub fn is_contradictory(&self) -> bool {
        !self.all.is_empty() && self.candidates.is_empty()
    }
}

/// Narrows provider candidates with the same advisory rules used for suggestions.
pub fn narrow_provider_candidates(cmd: &CommandLine, node: &CompletionNode) -> ProviderNarrowing {
    let selection = ProviderSelection::from_command(cmd, node);
    ProviderNarrowing {
        explicit: selection.explicit.map(ToOwned::to_owned),
        candidates: selection.candidates().map(ToOwned::to_owned).collect(),
        all: selection
            .all
            .iter()
            .map(|provider| (*provider).to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::ProviderSelection;
    use crate::completion::model::{
        CommandLine, CompletionNode, FlagHints, FlagNode, FlagOccurrence, SuggestionEntry,
    };

    fn hints() -> FlagHints {
        FlagHints {
            common: vec!["--comment".to_string(), "--provider".to_string()],
            by_provider: BTreeMap::from([
                (
                    "alpha".to_string(),
                    vec!["--cpu".to_string(), "--shared".to_string()],
                ),
                (
                    "beta".to_string(),
                    vec!["--instance".to_string(), "--shared".to_string()],
                ),
            ]),
            ..FlagHints::default()
        }
    }

    fn command(flags: &[&str]) -> CommandLine {
        let mut command = CommandLine::default();
        for flag in flags {
            command.push_flag_occurrence(FlagOccurrence {
                name: (*flag).to_string(),
                values: Vec::new(),
            });
        }
        command
    }

    fn node(hints: FlagHints) -> CompletionNode {
        CompletionNode {
            flag_hints: Some(hints),
            ..CompletionNode::default()
        }
    }

    #[test]
    fn provider_selection_infers_only_compatible_providers_from_flags_unit() {
        let hints = hints();
        let node = node(hints);

        let unique_command = command(&["--cpu"]);
        let unique = ProviderSelection::from_command(&unique_command, &node);
        assert_eq!(unique.name(), Some("alpha"));
        assert_eq!(unique.candidates().collect::<Vec<_>>(), vec!["alpha"]);
        assert!(unique.hides_selector());

        let ambiguous_command = command(&["--shared"]);
        let ambiguous = ProviderSelection::from_command(&ambiguous_command, &node);
        assert_eq!(ambiguous.name(), None);
        assert_eq!(
            ambiguous.candidates().collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert!(!ambiguous.hides_selector());
    }

    #[test]
    fn provider_selection_keeps_conflicting_and_unknown_flags_permissive_unit() {
        let hints = hints();
        let node = node(hints);

        for flags in [
            &["--cpu", "--instance"][..],
            &["--unknown"][..],
            &["--comment"][..],
        ] {
            let command = command(flags);
            let selection = ProviderSelection::from_command(&command, &node);
            assert_eq!(selection.name(), None);
            assert_eq!(
                selection.candidates().collect::<Vec<_>>(),
                vec!["alpha", "beta"]
            );
            assert!(!selection.hides_selector());
        }
    }

    #[test]
    fn provider_selection_never_replaces_an_explicit_unknown_provider_unit() {
        let hints = FlagHints {
            by_provider: BTreeMap::from([("alpha".to_string(), vec!["--cpu".to_string()])]),
            ..FlagHints::default()
        };
        let node = node(hints);
        let mut command = command(&["--cpu"]);
        command.push_flag_occurrence(FlagOccurrence {
            name: "--provider".to_string(),
            values: vec!["unknown".to_string()],
        });

        let selection = ProviderSelection::from_command(&command, &node);

        assert_eq!(selection.name(), None);
        assert_eq!(selection.candidates().collect::<Vec<_>>(), vec!["alpha"]);
        assert!(selection.hides_selector());
    }

    #[test]
    fn provider_selection_uses_shared_flag_value_catalogs_without_rejecting_input_unit() {
        let mut node = node(FlagHints {
            common: vec!["--os".to_string(), "--provider".to_string()],
            by_provider: BTreeMap::from([
                ("alpha".to_string(), Vec::new()),
                ("beta".to_string(), Vec::new()),
            ]),
            ..FlagHints::default()
        });
        node.flags.insert(
            "--os".to_string(),
            FlagNode {
                suggestions_by_provider: BTreeMap::from([
                    (
                        "alpha".to_string(),
                        vec![
                            SuggestionEntry::from("ubuntu"),
                            SuggestionEntry::from("shared"),
                        ],
                    ),
                    (
                        "beta".to_string(),
                        vec![
                            SuggestionEntry::from("rhel"),
                            SuggestionEntry::from("shared"),
                        ],
                    ),
                ]),
                exhaustive_suggestions_by_provider: BTreeSet::from([
                    "alpha".to_string(),
                    "beta".to_string(),
                ]),
                ..FlagNode::default()
            },
        );

        for (value, expected) in [("ubu", Some("alpha")), ("shared", None), ("unknown", None)] {
            let mut command = CommandLine::default();
            command.push_flag_occurrence(FlagOccurrence {
                name: "--os".to_string(),
                values: vec![value.to_string()],
            });
            let selection = ProviderSelection::from_command(&command, &node);
            assert_eq!(selection.name(), expected, "value: {value}");
        }

        let mut explicit = CommandLine::default();
        explicit.push_flag_occurrence(FlagOccurrence {
            name: "--os".to_string(),
            values: vec!["ubuntu".to_string()],
        });
        explicit.push_flag_occurrence(FlagOccurrence {
            name: "--provider".to_string(),
            values: vec!["beta".to_string()],
        });
        assert_eq!(
            ProviderSelection::from_command(&explicit, &node).name(),
            Some("beta")
        );
    }

    #[test]
    fn provider_selection_only_excludes_providers_with_exhaustive_value_catalogs_unit() {
        let mut node = node(FlagHints {
            common: vec!["--os".to_string()],
            by_provider: BTreeMap::from([
                ("alpha".to_string(), Vec::new()),
                ("beta".to_string(), Vec::new()),
            ]),
            ..FlagHints::default()
        });
        node.flags.insert(
            "--os".to_string(),
            FlagNode {
                suggestions_by_provider: BTreeMap::from([
                    ("alpha".to_string(), vec![SuggestionEntry::from("ubuntu")]),
                    ("beta".to_string(), Vec::new()),
                ]),
                ..FlagNode::default()
            },
        );
        let mut command = CommandLine::default();
        command.push_flag_occurrence(FlagOccurrence {
            name: "--os".to_string(),
            values: vec!["ubu".to_string()],
        });

        assert_eq!(
            ProviderSelection::from_command(&command, &node).name(),
            None
        );

        node.flags
            .get_mut("--os")
            .unwrap()
            .exhaustive_suggestions_by_provider
            .insert("alpha".to_string());
        assert_eq!(
            ProviderSelection::from_command(&command, &node).name(),
            None
        );

        node.flags
            .get_mut("--os")
            .unwrap()
            .exhaustive_suggestions_by_provider
            .insert("beta".to_string());
        assert_eq!(
            ProviderSelection::from_command(&command, &node).name(),
            Some("alpha")
        );
    }
}
