use osp_core::output_model::{Group, OutputMeta, OutputResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageCapability {
    StreamingSafe,
    NeedsAllRows,
    NeedsGroups,
    ExternalProcess,
}

/// High-level parser classification for a stage token.
///
/// The parser deliberately separates "known explicit verb", "unknown
/// verb-shaped token", and "quick-search text" so the evaluator does not have
/// to guess later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedStageKind {
    Explicit,
    UnknownExplicit,
    Quick,
}

/// One stage after the parser has decided how the evaluator should treat it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStage {
    pub kind: ParsedStageKind,
    pub verb: String,
    pub spec: String,
    pub raw: String,
}

impl ParsedStage {
    pub fn new(
        kind: ParsedStageKind,
        verb: impl Into<String>,
        spec: impl Into<String>,
        raw: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            verb: verb.into(),
            spec: spec.into(),
            raw: raw.into(),
        }
    }
}

/// Full parsed pipeline used by the evaluator.
///
/// `raw` is preserved for trace/debug output. `stages` carries the structured
/// stage classification that drives execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedPipeline {
    pub raw: String,
    pub stages: Vec<ParsedStage>,
}

pub type DslGroup = Group;
pub type DslMeta = OutputMeta;
pub type DslOutput = OutputResult;
