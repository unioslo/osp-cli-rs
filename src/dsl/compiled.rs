use anyhow::{Result, anyhow};

use crate::dsl::{
    model::{ParsedPipeline, ParsedStage, ParsedStageKind},
    verbs::{aggregate, filter, group, jq, limit, project, quick, sort, unroll, values},
};

#[derive(Debug, Clone)]
pub(crate) struct CompiledPipeline {
    pub(crate) stages: Vec<CompiledStage>,
}

#[derive(Debug, Clone)]
pub(crate) enum CompiledStage {
    Quick(quick::QuickPlan),
    Filter(filter::FilterPlan),
    Project(project::ProjectPlan),
    Sort(sort::SortPlan),
    Group(group::GroupPlan),
    Aggregate(aggregate::AggregatePlan),
    Limit(limit::LimitSpec),
    Collapse,
    CountMacro,
    Copy,
    Unroll(unroll::UnrollPlan),
    Clean,
    Question(quick::QuickPlan),
    Jq(String),
    Values(values::ValuesPlan),
    ValueQuick(quick::QuickPlan),
    KeyQuick(quick::QuickPlan),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticEffect {
    Preserve,
    Transform,
    Degrade,
}

impl CompiledPipeline {
    pub(crate) fn from_parsed(parsed: ParsedPipeline) -> Result<Self> {
        let stages = parsed
            .stages
            .iter()
            .filter(|stage| !stage.verb.is_empty())
            .map(CompiledStage::from_parsed)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { stages })
    }
}

impl CompiledStage {
    pub(crate) fn from_parsed(stage: &ParsedStage) -> Result<Self> {
        match stage.kind {
            ParsedStageKind::Quick => Ok(Self::Quick(quick::compile(&stage.raw)?)),
            ParsedStageKind::UnknownExplicit => Err(anyhow!("unknown DSL verb: {}", stage.verb)),
            ParsedStageKind::Explicit => match stage.verb.as_str() {
                "F" => Ok(Self::Filter(filter::compile(&stage.spec)?)),
                "P" => Ok(Self::Project(project::compile(&stage.spec)?)),
                "S" => Ok(Self::Sort(sort::compile(&stage.spec)?)),
                "G" => Ok(Self::Group(group::compile(&stage.spec)?)),
                "A" => Ok(Self::Aggregate(aggregate::compile(&stage.spec)?)),
                "L" => Ok(Self::Limit(limit::parse_limit_spec(&stage.spec)?)),
                "Z" => Ok(Self::Collapse),
                "C" => {
                    if !stage.spec.trim().is_empty() {
                        return Err(anyhow!("C takes no arguments"));
                    }
                    Ok(Self::CountMacro)
                }
                "Y" => Ok(Self::Copy),
                "U" => Ok(Self::Unroll(unroll::compile(&stage.spec)?)),
                "?" => {
                    let trimmed = stage.spec.trim();
                    if trimmed.is_empty() {
                        Ok(Self::Clean)
                    } else {
                        Ok(Self::Question(quick::compile(&format!("?{trimmed}"))?))
                    }
                }
                "JQ" => Ok(Self::Jq(jq::compile(&stage.spec)?)),
                "VAL" | "VALUE" => Ok(Self::Values(values::compile(&stage.spec)?)),
                "V" => {
                    let raw = if stage.spec.trim().is_empty() {
                        "V".to_string()
                    } else {
                        format!("V {}", stage.spec.trim())
                    };
                    Ok(Self::ValueQuick(quick::compile(&raw)?))
                }
                "K" => {
                    let raw = if stage.spec.trim().is_empty() {
                        "K".to_string()
                    } else {
                        format!("K {}", stage.spec.trim())
                    };
                    Ok(Self::KeyQuick(quick::compile(&raw)?))
                }
                other => Err(anyhow!("unknown DSL verb: {other}")),
            },
        }
    }

    pub(crate) fn can_stream(&self) -> bool {
        match self {
            Self::Quick(_)
            | Self::Filter(_)
            | Self::Project(_)
            | Self::Values(_)
            | Self::Copy
            | Self::Unroll(_)
            | Self::Clean
            | Self::Question(_)
            | Self::ValueQuick(_)
            | Self::KeyQuick(_) => true,
            Self::Limit(spec) => spec.is_head_only(),
            Self::Sort(_)
            | Self::Group(_)
            | Self::Aggregate(_)
            | Self::Collapse
            | Self::CountMacro
            | Self::Jq(_) => false,
        }
    }

    pub(crate) fn preserves_render_recommendation(&self) -> bool {
        matches!(
            self,
            Self::Quick(_)
                | Self::Filter(_)
                | Self::Sort(_)
                | Self::Limit(_)
                | Self::Copy
                | Self::Clean
                | Self::Question(_)
        )
    }

    pub(crate) fn semantic_effect(&self) -> SemanticEffect {
        match self {
            Self::Quick(_)
            | Self::Filter(_)
            | Self::Sort(_)
            | Self::Limit(_)
            | Self::Copy
            | Self::Clean
            | Self::Question(_)
            | Self::ValueQuick(_)
            | Self::KeyQuick(_) => SemanticEffect::Preserve,
            Self::Project(_)
            | Self::Group(_)
            | Self::Aggregate(_)
            | Self::Unroll(_)
            | Self::Values(_) => SemanticEffect::Transform,
            Self::Collapse | Self::CountMacro | Self::Jq(_) => SemanticEffect::Degrade,
        }
    }
}
