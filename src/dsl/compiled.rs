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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StageBehavior {
    pub(crate) can_stream: bool,
    pub(crate) preserves_render_recommendation: bool,
    pub(crate) semantic_effect: SemanticEffect,
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

    pub(crate) fn behavior(&self) -> StageBehavior {
        match self {
            Self::Quick(_)
            | Self::Filter(_)
            | Self::Copy
            | Self::Clean
            | Self::Question(_)
            | Self::ValueQuick(_)
            | Self::KeyQuick(_) => StageBehavior {
                can_stream: true,
                preserves_render_recommendation: true,
                semantic_effect: SemanticEffect::Preserve,
            },
            Self::Project(_) | Self::Unroll(_) | Self::Values(_) => StageBehavior {
                can_stream: true,
                preserves_render_recommendation: false,
                semantic_effect: SemanticEffect::Transform,
            },
            Self::Limit(spec) => StageBehavior {
                can_stream: spec.is_head_only(),
                preserves_render_recommendation: true,
                semantic_effect: SemanticEffect::Preserve,
            },
            Self::Sort(_) => StageBehavior {
                can_stream: false,
                preserves_render_recommendation: true,
                semantic_effect: SemanticEffect::Preserve,
            },
            Self::Group(_) | Self::Aggregate(_) => StageBehavior {
                can_stream: false,
                preserves_render_recommendation: false,
                semantic_effect: SemanticEffect::Transform,
            },
            Self::Collapse | Self::CountMacro | Self::Jq(_) => StageBehavior {
                can_stream: false,
                preserves_render_recommendation: false,
                semantic_effect: SemanticEffect::Degrade,
            },
        }
    }

    pub(crate) fn quick_plan(&self) -> Option<&quick::QuickPlan> {
        match self {
            Self::Quick(plan)
            | Self::Question(plan)
            | Self::ValueQuick(plan)
            | Self::KeyQuick(plan) => Some(plan),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CompiledStage, SemanticEffect};
    use crate::dsl::verbs::{filter, limit, project};

    #[test]
    fn stage_behavior_centralizes_stream_render_and_semantic_rules_unit() {
        let filter = CompiledStage::Filter(filter::compile("uid=alice").expect("filter plan"));
        let behavior = filter.behavior();
        assert!(behavior.can_stream);
        assert!(behavior.preserves_render_recommendation);
        assert_eq!(behavior.semantic_effect, SemanticEffect::Preserve);

        let project = CompiledStage::Project(project::compile("uid").expect("project plan"));
        let behavior = project.behavior();
        assert!(behavior.can_stream);
        assert!(!behavior.preserves_render_recommendation);
        assert_eq!(behavior.semantic_effect, SemanticEffect::Transform);

        let head = CompiledStage::Limit(limit::parse_limit_spec("2").expect("head limit"));
        let behavior = head.behavior();
        assert!(behavior.can_stream);
        assert!(behavior.preserves_render_recommendation);
        assert_eq!(behavior.semantic_effect, SemanticEffect::Preserve);

        let tail = CompiledStage::Limit(limit::parse_limit_spec("-2").expect("tail limit"));
        let behavior = tail.behavior();
        assert!(!behavior.can_stream);
        assert!(behavior.preserves_render_recommendation);
        assert_eq!(behavior.semantic_effect, SemanticEffect::Preserve);
    }

    #[test]
    fn quick_plan_helper_covers_all_quick_family_variants_unit() {
        let quick = CompiledStage::Quick(crate::dsl::verbs::quick::compile("alice").expect("plan"));
        assert!(quick.quick_plan().is_some());

        let question =
            CompiledStage::Question(crate::dsl::verbs::quick::compile("?uid").expect("plan"));
        assert!(question.quick_plan().is_some());

        let value_quick =
            CompiledStage::ValueQuick(crate::dsl::verbs::quick::compile("V uid").expect("plan"));
        assert!(value_quick.quick_plan().is_some());

        let key_quick =
            CompiledStage::KeyQuick(crate::dsl::verbs::quick::compile("K uid").expect("plan"));
        assert!(key_quick.quick_plan().is_some());

        let filter = CompiledStage::Filter(filter::compile("uid=alice").expect("filter plan"));
        assert!(filter.quick_plan().is_none());
    }
}
