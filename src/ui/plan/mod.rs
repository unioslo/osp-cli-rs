use crate::core::output::OutputFormat;
use crate::core::output_model::OutputResult;
use crate::guide::GuideView;

use super::settings::{RenderProfile, RenderSettings, ResolvedRenderSettings, resolve_settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticKind {
    Guide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPlan {
    pub format: OutputFormat,
    pub profile: RenderProfile,
    pub settings: ResolvedRenderSettings,
    pub semantic_kind: Option<SemanticKind>,
}

pub fn plan_output(
    output: &OutputResult,
    settings: &RenderSettings,
    profile: RenderProfile,
) -> RenderPlan {
    let format = settings.resolve_output_format(output);
    let effective_profile = if matches!(format, OutputFormat::Json) {
        RenderProfile::CopySafe
    } else {
        profile
    };
    RenderPlan {
        format,
        profile: effective_profile,
        settings: resolve_settings(settings, effective_profile),
        semantic_kind: semantic_kind(output),
    }
}

fn semantic_kind(output: &OutputResult) -> Option<SemanticKind> {
    GuideView::try_from_output_result(output).map(|_| SemanticKind::Guide)
}
