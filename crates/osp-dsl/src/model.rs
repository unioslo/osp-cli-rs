use osp_core::output_model::{Group, OutputMeta, OutputResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageCapability {
    StreamingSafe,
    NeedsAllRows,
    NeedsGroups,
    ExternalProcess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStage {
    pub verb: String,
    pub spec: String,
    pub raw: String,
}

impl ParsedStage {
    pub fn new(verb: impl Into<String>, spec: impl Into<String>, raw: impl Into<String>) -> Self {
        Self {
            verb: verb.into(),
            spec: spec.into(),
            raw: raw.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedPipeline {
    pub raw: String,
    pub stages: Vec<ParsedStage>,
}

pub type DslGroup = Group;
pub type DslMeta = OutputMeta;
pub type DslOutput = OutputResult;
