//! Shared field selection over canonical rows.
//!
//! Structural selectors resolve addressed paths; bare selectors search
//! descendants. Projection uses original addresses before compaction. Filters
//! retain complete rows; they never switch to a document-pruning executor.

use crate::core::row::Row;
use serde_json::Value;
use std::collections::HashSet;

use crate::dsl::{
    eval::matchers::match_row_keys,
    eval::resolve::{
        AddressStep, AddressedValue, resolve_descendant_matches, resolve_path_matches,
    },
    parse::{
        key_spec::{ExactMode, KeySpec},
        path::{PathExpression, expression_to_flat_key, is_structural_path_token, parse_path},
    },
};

/// Compile-time split between structural path semantics and permissive
/// descendant matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectorMode {
    StructuralPath,
    PermissiveDescendant,
}

/// Parsed selector token plus the compile-time mode it should use.
///
/// Selector verbs should carry this instead of threading raw `KeySpec` and
/// `SelectorMode` separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledSelector {
    pub(crate) key_spec: KeySpec,
    pub(crate) mode: SelectorMode,
    path: Option<PathExpression>,
}

/// Classifies `token` into the selector mode it should use.
///
/// This decision should happen during verb compilation so execution does not
/// have to keep re-guessing semantics from token shape.
pub(crate) fn classify_token(token: &str) -> SelectorMode {
    if token_uses_structural_path(token) {
        SelectorMode::StructuralPath
    } else {
        SelectorMode::PermissiveDescendant
    }
}

/// Classifies a parsed [`KeySpec`].
///
/// Callers should prefer this over classifying raw stage text so operator
/// prefixes like `!path` and `?path` do not accidentally leak into selector
/// semantics.
pub(crate) fn classify_key_spec(spec: &KeySpec) -> SelectorMode {
    classify_token(&spec.token)
}

impl CompiledSelector {
    pub(crate) fn parse(raw: &str) -> Self {
        Self::from_key_spec(KeySpec::parse(raw))
    }

    pub(crate) fn from_token(token: String, exact: ExactMode) -> Self {
        Self::from_key_spec(KeySpec {
            token,
            negated: false,
            existence: false,
            exact,
            strict_ambiguous: false,
        })
    }

    pub(crate) fn from_key_spec(key_spec: KeySpec) -> Self {
        let mode = classify_key_spec(&key_spec);
        let path = parse_path(&key_spec.token).ok();
        Self {
            key_spec,
            mode,
            path,
        }
    }

    pub(crate) fn is_structural(&self) -> bool {
        matches!(self.mode, SelectorMode::StructuralPath)
    }

    pub(crate) fn resolve_matches(&self, root: &Value) -> Vec<AddressedValue> {
        match self.mode {
            SelectorMode::StructuralPath => resolve_path_matches(root, self.token(), self.exact()),
            SelectorMode::PermissiveDescendant => {
                resolve_descendant_matches(root, self.token(), self.exact())
            }
        }
    }

    pub(crate) fn token(&self) -> &str {
        &self.key_spec.token
    }

    pub(crate) fn exact(&self) -> ExactMode {
        self.key_spec.exact
    }

    pub(crate) fn path(&self) -> Option<&PathExpression> {
        self.path.as_ref()
    }

    pub(crate) fn collect_dynamic_column(
        &self,
        nested_row: &Value,
    ) -> Option<(String, Vec<AddressedValue>)> {
        if !self.is_structural() {
            return None;
        }

        let matches = self.resolve_matches(nested_row);
        if !matches.iter().any(|entry| {
            entry
                .address
                .iter()
                .any(|step| matches!(step, AddressStep::Index(_)))
        }) {
            return None;
        }

        Some((self.label(), matches))
    }

    pub(crate) fn matched_flat_keys(&self, flat_row: &Row) -> Vec<String> {
        if self.is_structural() {
            let Some(path) = self.path() else {
                return Vec::new();
            };
            let Some(exact) = expression_to_flat_key(path) else {
                return Vec::new();
            };
            return flat_row
                .keys()
                .filter(|key| {
                    *key == &exact
                        || key.starts_with(&format!("{exact}."))
                        || key.starts_with(&format!("{exact}["))
                })
                .cloned()
                .collect();
        }

        match_row_keys(flat_row, self.token(), self.exact())
            .into_iter()
            .map(ToOwned::to_owned)
            .collect()
    }

    pub(crate) fn label(&self) -> String {
        if let Some(path) = self.path()
            && let Some(segment) = path.segments.last()
            && let Some(name) = &segment.name
        {
            return name.clone();
        }

        let token = self.token();
        let last = token.rsplit('.').next().unwrap_or(token);
        let head = last.split('[').next().unwrap_or(last);
        if head.is_empty() {
            "value".to_string()
        } else {
            head.to_string()
        }
    }
}

/// Returns whether `token` should use the structural selector engine rather
/// than permissive descendant matching.
///
/// Bare names like `name` intentionally stay on the permissive path for now.
/// Dotted, indexed, sliced, fanout, or absolute selectors are structural.
pub(crate) fn token_uses_structural_path(token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return false;
    }

    let Ok(path) = parse_path(trimmed) else {
        return false;
    };

    is_structural_path_token(trimmed, &path)
}

/// Collects and deduplicates addressed matches from compiled selectors.
pub(crate) fn collect_compiled_matches<'a, I>(root: &Value, selectors: I) -> Vec<AddressedValue>
where
    I: IntoIterator<Item = &'a CompiledSelector>,
{
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for selector in selectors {
        for entry in selector.resolve_matches(root) {
            if seen.insert(entry.flat_key.clone()) {
                out.push(entry);
            }
        }
    }

    out
}
