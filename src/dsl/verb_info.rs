/// Static metadata for one registered DSL verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbInfo {
    /// Canonical verb spelling accepted by the parser.
    pub verb: &'static str,
    /// Short human-readable summary of the verb.
    pub summary: &'static str,
}

const VERBS: &[VerbInfo] = &[
    VerbInfo {
        verb: "F",
        summary: "Filter rows",
    },
    VerbInfo {
        verb: "P",
        summary: "Project columns",
    },
    VerbInfo {
        verb: "S",
        summary: "Sort rows",
    },
    VerbInfo {
        verb: "G",
        summary: "Group rows",
    },
    VerbInfo {
        verb: "A",
        summary: "Aggregate rows/groups",
    },
    VerbInfo {
        verb: "L",
        summary: "Limit rows",
    },
    VerbInfo {
        verb: "Z",
        summary: "Collapse grouped output",
    },
    VerbInfo {
        verb: "C",
        summary: "Count rows",
    },
    VerbInfo {
        verb: "Y",
        summary: "Mark output for copy",
    },
    VerbInfo {
        verb: "H",
        summary: "Show DSL help",
    },
    VerbInfo {
        verb: "V",
        summary: "Value-only quick search",
    },
    VerbInfo {
        verb: "K",
        summary: "Key-only quick search",
    },
    VerbInfo {
        verb: "?",
        summary: "Clean rows / exists filter",
    },
    VerbInfo {
        verb: "U",
        summary: "Unroll list field",
    },
    VerbInfo {
        verb: "JQ",
        summary: "Run jq-like expression",
    },
    VerbInfo {
        verb: "VAL",
        summary: "Extract values",
    },
    VerbInfo {
        verb: "VALUE",
        summary: "Extract values",
    },
];

/// Returns metadata for all registered DSL verbs, including meta-only verbs.
pub fn registered_verbs() -> &'static [VerbInfo] {
    VERBS
}

#[cfg(test)]
/// Returns the registered non-meta DSL verb names used by tests.
pub fn registered_explicit_verbs() -> Vec<&'static str> {
    VERBS
        .iter()
        .filter(|info| info.verb != "H")
        .map(|info| info.verb)
        .collect()
}

/// Returns verb metadata for `verb`, matched case-insensitively.
pub fn verb_info(verb: &str) -> Option<&'static VerbInfo> {
    VERBS
        .iter()
        .find(|info| info.verb.eq_ignore_ascii_case(verb))
}

/// Returns whether `verb` is a registered non-meta verb.
pub fn is_registered_explicit_verb(verb: &str) -> bool {
    VERBS
        .iter()
        .filter(|info| info.verb != "H")
        .any(|info| info.verb.eq_ignore_ascii_case(verb))
}

#[cfg(test)]
mod tests {
    use crate::dsl::verb_info::{is_registered_explicit_verb, registered_explicit_verbs};

    #[test]
    fn explicit_verb_registration_is_derived_from_metadata_unit() {
        let verbs = registered_explicit_verbs();
        assert!(verbs.contains(&"F"));
        assert!(verbs.contains(&"JQ"));
        assert!(!verbs.contains(&"H"));
        assert!(is_registered_explicit_verb("val"));
        assert!(!is_registered_explicit_verb("h"));
    }
}
