use crate::model::{ParsedPipeline, ParsedStage, ParsedStageKind};

use super::lexer::{LexerError, StageSegment, split_pipeline, tokenize_stage};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pipeline {
    pub command: String,
    pub stages: Vec<String>,
}

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

const EXPLICIT_STAGE_VERBS: &[&str] = &[
    "P", "V", "K", "VAL", "VALUE", "F", "G", "A", "S", "L", "Z", "C", "Y", "U", "?", "JQ",
];

pub fn parse_stage(raw_stage: &str) -> Result<ParsedStage, LexerError> {
    let segment = StageSegment {
        raw: raw_stage.trim().to_string(),
        span: super::lexer::Span {
            start: 0,
            end: raw_stage.trim().len(),
        },
    };

    if segment.raw.is_empty() {
        return Ok(ParsedStage::new(ParsedStageKind::Quick, "", "", raw_stage));
    }

    let tokens = tokenize_stage(&segment)?;

    let Some(first) = tokens.first() else {
        return Ok(ParsedStage::new(ParsedStageKind::Quick, "", "", raw_stage));
    };

    let verb = first.text.to_ascii_uppercase();
    let spec = if first.span.end <= segment.raw.len() {
        segment.raw[first.span.end..].trim().to_string()
    } else {
        String::new()
    };

    Ok(ParsedStage::new(
        classify_stage_kind(&verb),
        verb,
        spec,
        segment.raw,
    ))
}

pub fn parse_stage_list(stages: &[String]) -> Result<ParsedPipeline, LexerError> {
    Ok(ParsedPipeline {
        raw: stages.join(" | "),
        stages: stages
            .iter()
            .map(|stage| parse_stage(stage))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn classify_stage_kind(verb: &str) -> ParsedStageKind {
    if EXPLICIT_STAGE_VERBS.contains(&verb) {
        return ParsedStageKind::Explicit;
    }

    if verb.len() == 1 && verb.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return ParsedStageKind::UnknownExplicit;
    }

    ParsedStageKind::Quick
}

#[cfg(test)]
mod tests {
    use crate::model::ParsedStageKind;

    use super::{LexerError, parse_pipeline, parse_stage, parse_stage_list};

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
}
