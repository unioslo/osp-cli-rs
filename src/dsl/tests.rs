use serde_json::json;

use crate::core::output_model::{OutputResult, RenderRecommendation};

use super::{Dsl2Mode, apply_output_pipeline};

fn sample_output() -> OutputResult {
    let mut output = OutputResult::from_rows(vec![
        json!({"uid": "alice", "dept": "ops"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "dept": "eng"})
            .as_object()
            .cloned()
            .expect("object"),
    ]);
    output.meta.render_recommendation = Some(RenderRecommendation::Guide);
    output
}

#[test]
fn mode_parser_accepts_legacy_rollout_aliases_as_enabled_unit() {
    assert_eq!(Dsl2Mode::parse("legacy"), Some(Dsl2Mode::Enabled));
    assert_eq!(Dsl2Mode::parse("compare"), Some(Dsl2Mode::Enabled));
    assert_eq!(Dsl2Mode::parse("enabled"), Some(Dsl2Mode::Enabled));
    assert_eq!(Dsl2Mode::parse("bogus"), None);
}

#[test]
fn dsl_facade_delegates_to_canonical_engine_unit() {
    let stages = vec!["F dept=ops".to_string(), "P uid".to_string()];
    let facade = crate::dsl::apply_output_pipeline(sample_output().clone(), &stages).expect("dsl");
    let canonical = apply_output_pipeline(sample_output(), &stages).expect("canonical");
    assert_eq!(facade, canonical);
}
