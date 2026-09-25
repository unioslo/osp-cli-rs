/// Static metadata for one registered DSL verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbInfo {
    /// Canonical verb spelling accepted by the parser.
    pub verb: &'static str,
    /// Short human-readable summary of the verb.
    pub summary: &'static str,
    /// Accepted syntax and a copyable pipeline example.
    pub help: &'static str,
}

const VERBS: &[VerbInfo] = &[
    VerbInfo {
        verb: "F",
        summary: "Filter rows",
        help: "F field OP value; OP: = == != > >= < <= ~. F field text contains text. Example: F status=running. Quote regex alternation: F name ~ 'foo|bar'. Naive dates/times use local timezone; use RFC3339 offsets for ambiguous DST times.",
    },
    VerbInfo {
        verb: "P",
        summary: "Project columns",
        help: "P field[,field...] [!field...]. Example: P id,status,created_at. Headings are canonical filter paths; use --json to inspect all fields. Nested paths and [] fanout are supported.",
    },
    VerbInfo {
        verb: "S",
        summary: "Sort rows",
        help: "S field [asc|desc] [AS num|str|ip] ...; -field and !field mean descending. Example: S -created_at id. Unknown fields in nonempty results are errors; missing values sort last.",
    },
    VerbInfo {
        verb: "G",
        summary: "Group rows",
        help: "G field [AS alias] [field ...]. Example: G provider. Group keys and aggregates are metadata; member rows remain canonical. F tests matching header fields first, otherwise members; S and L act on groups.",
    },
    VerbInfo {
        verb: "A",
        summary: "Aggregate rows/groups",
        help: "A count|sum|avg|min|max [field] [AS alias]. Example: A sum memory AS total. Computes one result on rows, or an aggregate per group. Empty input count is zero.",
    },
    VerbInfo {
        verb: "L",
        summary: "Limit rows",
        help: "L count [offset]. Example: L 10 20; L -5 takes the last five. On grouped output, limits groups.",
    },
    VerbInfo {
        verb: "Z",
        summary: "Collapse grouped output",
        help: "Z. Collapse group headers and aggregates into ordinary summary rows. Example: G provider | A count | Z.",
    },
    VerbInfo {
        verb: "C",
        summary: "Count rows",
        help: "C. Count current rows (including zero); groups become one count summary per group; no surviving groups yields count: 0. Example: F status=running | C. Service totals/cursors are discarded by a pipeline.",
    },
    VerbInfo {
        verb: "Y",
        summary: "Mark output for copy",
        help: "Y. Copy the current output. Example: P id | Y.",
    },
    VerbInfo {
        verb: "H",
        summary: "Show DSL help",
        help: "H [verb]. Example: H F. Help is shown before running the command.",
    },
    VerbInfo {
        verb: "V",
        summary: "Value-only quick search",
        help: "V [!|=|==|!=|%]text. Match values, retaining whole rows. Example: V running. != is negated case-insensitive equality.",
    },
    VerbInfo {
        verb: "K",
        summary: "Key-only quick search",
        help: "K [!|=|==|!=|%]field. Match keys, retaining whole rows. Example: K requester.",
    },
    VerbInfo {
        verb: "?",
        summary: "Clean rows / exists filter",
        help: "? removes empty fields. ?field checks truthy presence; !?field checks absence. Example: ?requester.",
    },
    VerbInfo {
        verb: "U",
        summary: "Unroll list field",
        help: "U field. Expand a list field into rows. Example: U contacts | VALUE contacts.",
    },
    VerbInfo {
        verb: "JQ",
        summary: "Run jq-like expression",
        help: "JQ 'expression'. The input is an array of canonical rows; quote jq pipes. Example: JQ '.[] | .id'. For each group, input is {groups, aggregates, rows}; use .rows to access members.",
    },
    VerbInfo {
        verb: "VAL",
        summary: "Extract values",
        help: "VAL [field...]. Extract matching values as value rows. Alias of VALUE. Example: VAL status.name.",
    },
    VerbInfo {
        verb: "VALUE",
        summary: "Extract values",
        help: "VALUE [field...]. Extract matching values as value rows. Example: VALUE status.name. Without fields, extracts all row values.",
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
