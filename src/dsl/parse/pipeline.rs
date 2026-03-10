use crate::dsl::model::{ParsedPipeline, ParsedStage, ParsedStageKind};
use crate::dsl::verbs::is_registered_explicit_verb;

use super::lexer::{split_pipeline, tokenize_stage, LexerError, StageSegment};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Parsed command line split into a command segment and trailing DSL stages.
pub struct Pipeline {
    /// Raw command segment before pipe stages.
    pub command: String,
    /// Raw DSL stages in left-to-right execution order.
    pub stages: Vec<String>,
}

/// Split a full command line into its command portion and raw pipe stages.
pub fn parse_pipeline(line: &str) -> Result<Pipeline, LexerError> {
    let segments = split_pipeline(line)?;

    let command = segments
        .first()
        .map(|segment| segment.raw.clone())
        .unwrap_or_default();

    let stages = if segments.len() > 1 {
        segments[1..]
            .iter()
            .map(|segment| segment.raw.clone())
            .collect()
    } else {
        Vec::new()
    };

    Ok(Pipeline { command, stages })
}

/// Parse a raw stage string into the structured form the evaluator consumes.
///
/// This is intentionally conservative:
/// - registered verbs become explicit stages
/// - unknown one-letter tokens are treated as likely typos and fail later
/// - everything else becomes quick-search text
pub fn parse_stage(raw_stage: &str) -> Result<ParsedStage, LexerError> {
    let segment = stage_segment_from_raw(raw_stage);

    if segment.raw.is_empty() {
        return Ok(empty_quick_stage(raw_stage));
    }

    let tokens = tokenize_stage(&segment)?;

    let Some(first) = tokens.first() else {
        return Ok(empty_quick_stage(raw_stage));
    };

    let verb = first.text.to_ascii_uppercase();
    let spec = stage_spec_after_first_token(&segment, first.span.end);

    Ok(ParsedStage::new(
        classify_stage_kind(&verb),
        verb,
        spec,
        segment.raw,
    ))
}

/// Parse an already-split list of stage strings.
pub fn parse_stage_list(stages: &[String]) -> Result<ParsedPipeline, LexerError> {
    Ok(ParsedPipeline {
        raw: stages.join(" | "),
        stages: stages
            .iter()
            .map(|stage| parse_stage(stage))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn stage_segment_from_raw(raw_stage: &str) -> StageSegment {
    let trimmed = raw_stage.trim();
    StageSegment {
        raw: trimmed.to_string(),
        span: super::lexer::Span {
            start: 0,
            end: trimmed.len(),
        },
    }
}

fn empty_quick_stage(raw_stage: &str) -> ParsedStage {
    ParsedStage::new(ParsedStageKind::Quick, "", "", raw_stage)
}

fn stage_spec_after_first_token(segment: &StageSegment, token_end: usize) -> String {
    if token_end > segment.raw.len() {
        return String::new();
    }
    segment.raw[token_end..].trim().to_string()
}

fn classify_stage_kind(verb: &str) -> ParsedStageKind {
    if is_registered_explicit_verb(verb) {
        return ParsedStageKind::Explicit;
    }

    if verb.len() == 1 && verb.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return ParsedStageKind::UnknownExplicit;
    }

    ParsedStageKind::Quick
}

#[cfg(test)]
mod tests {
    use crate::dsl::model::ParsedStageKind;

    use super::{parse_pipeline, parse_stage, parse_stage_list, LexerError};

    #[test]
    fn parse_pipeline_extracts_command_and_stages() {
        let parsed =
            parse_pipeline("ldap user oistes | P uid,cn | F uid=oistes").expect("valid pipeline");
        assert_eq!(parsed.command, "ldap user oistes");
        assert_eq!(parsed.stages, vec!["P uid,cn", "F uid=oistes"]);
    }

    #[test]
    fn parse_pipeline_ignores_empty_segments_like_python() {
        let parsed =
            parse_pipeline("ldap user oistes || P uid |  | F uid=oistes").expect("valid pipeline");
        assert_eq!(parsed.command, "ldap user oistes");
        assert_eq!(parsed.stages, vec!["P uid", "F uid=oistes"]);
    }

    #[test]
    fn parse_pipeline_rejects_invalid_quotes() {
        let err =
            parse_pipeline("ldap user 'oops | P uid").expect_err("invalid quotes should fail");
        assert!(matches!(err, LexerError::UnterminatedSingleQuote { .. }));
    }

    #[test]
    fn parse_pipeline_rejects_trailing_escape() {
        let err = parse_pipeline("ldap user foo\\").expect_err("trailing escape should fail");
        assert!(matches!(err, LexerError::TrailingEscape { .. }));
    }

    #[test]
    fn parse_stage_extracts_verb_and_spec() {
        let parsed = parse_stage("F uid=oistes").expect("stage should parse");
        assert_eq!(parsed.kind, ParsedStageKind::Explicit);
        assert_eq!(parsed.verb, "F");
        assert_eq!(parsed.spec, "uid=oistes");
    }

    #[test]
    fn parse_stage_with_only_term_becomes_quick_candidate() {
        let parsed = parse_stage("uid").expect("stage should parse");
        assert_eq!(parsed.kind, ParsedStageKind::Quick);
        assert_eq!(parsed.verb, "UID");
        assert_eq!(parsed.spec, "");
    }

    #[test]
    fn parse_stage_marks_unknown_single_letter_verb_as_explicit() {
        let parsed = parse_stage("R oist").expect("stage should parse");
        assert_eq!(parsed.kind, ParsedStageKind::UnknownExplicit);
        assert_eq!(parsed.verb, "R");
        assert_eq!(parsed.spec, "oist");
    }

    #[test]
    fn parse_stage_list_rejects_invalid_quoted_stage() {
        let err = parse_stage_list(&[r#"F note="oops"#.to_string()])
            .expect_err("invalid quotes should fail");
        assert!(matches!(err, LexerError::UnterminatedDoubleQuote { .. }));
    }

    #[test]
    fn parse_stage_list_rejects_trailing_escape() {
        let err = parse_stage_list(&["F path=C:\\Temp\\".to_string()])
            .expect_err("trailing escape should fail");
        assert!(matches!(err, LexerError::TrailingEscape { .. }));
    }
}
