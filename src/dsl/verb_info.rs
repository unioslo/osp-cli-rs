#[cfg(test)]
use crate::dsl::model::{ParsedStage, ParsedStageKind};

/// Streaming behavior for a DSL verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbStreaming {
    /// The stage can process rows incrementally.
    Streamable,
    /// The stage can stream for some inputs but must materialize for others.
    Conditional,
    /// The stage requires full materialization of its input.
    Materializes,
    /// The stage is metadata-only and does not participate in data execution.
    Meta,
}

/// Static metadata for one registered DSL verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbInfo {
    /// Canonical verb spelling accepted by the parser.
    pub verb: &'static str,
    /// Short human-readable summary of the verb.
    pub summary: &'static str,
    /// Streaming behavior class for the verb.
    pub streaming: VerbStreaming,
    /// Short note explaining the streaming behavior.
    pub streaming_note: &'static str,
}

const VERBS: &[VerbInfo] = &[
    VerbInfo {
        verb: "F",
        summary: "Filter rows",
        streaming: VerbStreaming::Streamable,
        streaming_note: "row-by-row filter",
    },
    VerbInfo {
        verb: "P",
        summary: "Project columns",
        streaming: VerbStreaming::Streamable,
        streaming_note: "row-by-row projection/fanout",
    },
    VerbInfo {
        verb: "S",
        summary: "Sort rows",
        streaming: VerbStreaming::Materializes,
        streaming_note: "sorting needs the full input",
    },
    VerbInfo {
        verb: "G",
        summary: "Group rows",
        streaming: VerbStreaming::Materializes,
        streaming_note: "grouping needs the full input",
    },
    VerbInfo {
        verb: "A",
        summary: "Aggregate rows/groups",
        streaming: VerbStreaming::Materializes,
        streaming_note: "aggregation needs the full input or full groups",
    },
    VerbInfo {
        verb: "L",
        summary: "Limit rows",
        streaming: VerbStreaming::Conditional,
        streaming_note: "head limits stream; tail/negative forms materialize",
    },
    VerbInfo {
        verb: "Z",
        summary: "Collapse grouped output",
        streaming: VerbStreaming::Materializes,
        streaming_note: "collapse only runs after grouped output exists",
    },
    VerbInfo {
        verb: "C",
        summary: "Count rows",
        streaming: VerbStreaming::Materializes,
        streaming_note: "count needs the full input or full groups",
    },
    VerbInfo {
        verb: "Y",
        summary: "Mark output for copy",
        streaming: VerbStreaming::Streamable,
        streaming_note: "passthrough marker",
    },
    VerbInfo {
        verb: "H",
        summary: "Show DSL help",
        streaming: VerbStreaming::Meta,
        streaming_note: "help stage; not part of data execution",
    },
    VerbInfo {
        verb: "V",
        summary: "Value-only quick search",
        streaming: VerbStreaming::Conditional,
        streaming_note: "flat rows stream with a two-row lookahead; grouped input still materializes",
    },
    VerbInfo {
        verb: "K",
        summary: "Key-only quick search",
        streaming: VerbStreaming::Conditional,
        streaming_note: "flat rows stream with a two-row lookahead; grouped input still materializes",
    },
    VerbInfo {
        verb: "?",
        summary: "Clean rows / exists filter",
        streaming: VerbStreaming::Conditional,
        streaming_note: "flat rows stream; grouped input still materializes",
    },
    VerbInfo {
        verb: "U",
        summary: "Unroll list field",
        streaming: VerbStreaming::Streamable,
        streaming_note: "row-by-row selector fanout",
    },
    VerbInfo {
        verb: "JQ",
        summary: "Run jq-like expression",
        streaming: VerbStreaming::Materializes,
        streaming_note: "jq receives the full current payload",
    },
    VerbInfo {
        verb: "VAL",
        summary: "Extract values",
        streaming: VerbStreaming::Streamable,
        streaming_note: "row-by-row value extraction",
    },
    VerbInfo {
        verb: "VALUE",
        summary: "Extract values",
        streaming: VerbStreaming::Streamable,
        streaming_note: "row-by-row value extraction",
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
        .filter(|info| !matches!(info.streaming, VerbStreaming::Meta))
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
        .filter(|info| !matches!(info.streaming, VerbStreaming::Meta))
        .any(|info| info.verb.eq_ignore_ascii_case(verb))
}

/// Returns the display badge for a verb's streaming behavior, if any.
pub fn render_streaming_badge(streaming: VerbStreaming) -> Option<&'static str> {
    match streaming {
        VerbStreaming::Streamable | VerbStreaming::Meta => None,
        VerbStreaming::Conditional => Some("[conditional]"),
        VerbStreaming::Materializes => Some("[materializes]"),
    }
}

#[cfg(test)]
pub(crate) fn stage_can_stream_rows(stage: &ParsedStage) -> bool {
    if !matches!(
        stage.kind,
        ParsedStageKind::Quick | ParsedStageKind::Explicit
    ) {
        return false;
    }

    crate::dsl::compiled::CompiledStage::from_parsed(stage)
        .map(|stage| stage.behavior().can_stream)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use crate::dsl::{
        model::{ParsedStage, ParsedStageKind},
        verb_info::{
            VerbStreaming, is_registered_explicit_verb, registered_explicit_verbs,
            render_streaming_badge, stage_can_stream_rows, verb_info,
        },
    };

    #[test]
    fn stage_streamability_matches_real_barriers_unit() {
        assert!(stage_can_stream_rows(&ParsedStage::new(
            ParsedStageKind::Explicit,
            "F",
            "uid=alice",
            "F uid=alice",
        )));
        assert!(stage_can_stream_rows(&ParsedStage::new(
            ParsedStageKind::Explicit,
            "L",
            "10 0",
            "L 10 0",
        )));
        assert!(!stage_can_stream_rows(&ParsedStage::new(
            ParsedStageKind::Explicit,
            "L",
            "-2",
            "L -2",
        )));
        assert!(!stage_can_stream_rows(&ParsedStage::new(
            ParsedStageKind::Explicit,
            "A",
            "count",
            "A count",
        )));
        assert!(stage_can_stream_rows(&ParsedStage::new(
            ParsedStageKind::Quick,
            "UID",
            "",
            "uid",
        )));
    }

    #[test]
    fn verb_metadata_exposes_streaming_annotations_unit() {
        let aggregate = verb_info("A").expect("aggregate verb should exist");
        assert_eq!(aggregate.streaming, VerbStreaming::Materializes);
        assert_eq!(
            render_streaming_badge(aggregate.streaming),
            Some("[materializes]")
        );

        let filter = verb_info("F").expect("filter verb should exist");
        assert_eq!(filter.streaming, VerbStreaming::Streamable);
        assert_eq!(render_streaming_badge(filter.streaming), None);
    }

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
