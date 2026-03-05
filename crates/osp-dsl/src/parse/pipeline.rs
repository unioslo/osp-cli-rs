use crate::model::{ParsedPipeline, ParsedStage};

use super::lexer::{StageSegment, split_pipeline, tokenize_stage};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pipeline {
    pub command: String,
    pub stages: Vec<String>,
}

pub fn parse_pipeline(line: &str) -> Pipeline {
    let segments = match split_pipeline(line) {
        Ok(segments) => segments,
        // Keep compatibility with osprov-cli: when quotes are malformed, fall
        // back to a naive split on pipes instead of failing hard.
        Err(_) => line
            .split('|')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .map(|raw| StageSegment {
                raw: raw.to_string(),
                span: super::lexer::Span { start: 0, end: 0 },
            })
            .collect(),
    };

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

    Pipeline { command, stages }
}

pub fn parse_stage(raw_stage: &str) -> ParsedStage {
    let segment = StageSegment {
        raw: raw_stage.trim().to_string(),
        span: super::lexer::Span {
            start: 0,
            end: raw_stage.trim().len(),
        },
    };

    if segment.raw.is_empty() {
        return ParsedStage::new("", "", raw_stage);
    }

    let tokens = match tokenize_stage(&segment) {
        Ok(tokens) => tokens,
        Err(_) => {
            let mut parts = segment.raw.splitn(2, char::is_whitespace);
            let verb = parts.next().unwrap_or_default().to_ascii_uppercase();
            let spec = parts.next().unwrap_or_default().trim().to_string();
            return ParsedStage::new(verb, spec, raw_stage);
        }
    };

    let Some(first) = tokens.first() else {
        return ParsedStage::new("", "", raw_stage);
    };

    let verb = first.text.to_ascii_uppercase();
    let spec = if first.span.end <= segment.raw.len() {
        segment.raw[first.span.end..].trim().to_string()
    } else {
        String::new()
    };

    ParsedStage::new(verb, spec, segment.raw)
}

pub fn parse_stage_list(stages: &[String]) -> ParsedPipeline {
    ParsedPipeline {
        raw: stages.join(" | "),
        stages: stages.iter().map(|stage| parse_stage(stage)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_pipeline, parse_stage};

    #[test]
    fn parse_pipeline_extracts_command_and_stages() {
        let parsed = parse_pipeline("ldap user oistes | P uid,cn | F uid=oistes");
        assert_eq!(parsed.command, "ldap user oistes");
        assert_eq!(parsed.stages, vec!["P uid,cn", "F uid=oistes"]);
    }

    #[test]
    fn parse_pipeline_ignores_empty_segments_like_python() {
        let parsed = parse_pipeline("ldap user oistes || P uid |  | F uid=oistes");
        assert_eq!(parsed.command, "ldap user oistes");
        assert_eq!(parsed.stages, vec!["P uid", "F uid=oistes"]);
    }

    #[test]
    fn parse_pipeline_is_tolerant_for_invalid_quotes() {
        let parsed = parse_pipeline("ldap user 'oops | P uid");
        assert_eq!(parsed.command, "ldap user 'oops");
        assert_eq!(parsed.stages, vec!["P uid"]);
    }

    #[test]
    fn parse_stage_extracts_verb_and_spec() {
        let parsed = parse_stage("F uid=oistes");
        assert_eq!(parsed.verb, "F");
        assert_eq!(parsed.spec, "uid=oistes");
    }

    #[test]
    fn parse_stage_with_only_term_becomes_quick_candidate() {
        let parsed = parse_stage("uid");
        assert_eq!(parsed.verb, "UID");
        assert_eq!(parsed.spec, "");
    }
}
