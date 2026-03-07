use osp_dsl::parse::{
    lexer::{LexerError, Span, StageSegment, split_pipeline, tokenize_stage},
    pipeline::parse_pipeline,
};
use proptest::prelude::*;

fn word() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-z0-9]{1,8}").expect("word regex should compile")
}

fn single_quoted_fragment() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[^'\\r\\n]{0,16}")
        .expect("single-quoted fragment regex should compile")
}

fn double_quoted_fragment() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[^\"\\\\\\r\\n]{0,16}")
        .expect("double-quoted fragment regex should compile")
}

fn quoted_payload_fragment() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[A-Za-z0-9 !<>=|:/._-]{0,16}")
        .expect("quoted payload regex should compile")
}

fn escaped_plain_fragment() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[A-Za-z0-9:/._-]{1,8}")
        .expect("escaped plain fragment regex should compile")
}

proptest! {
    #[test]
    fn split_pipeline_preserves_pipe_inside_single_quotes(
        lhs in word(),
        left in single_quoted_fragment(),
        right in single_quoted_fragment(),
        stage in word(),
    ) {
        let payload = format!("{left}|{right}");
        let input = format!("ldap user {lhs} '{payload}' | P {stage}");

        let segments = split_pipeline(&input).expect("single-quoted payload should parse");
        prop_assert_eq!(segments.len(), 2);
        prop_assert_eq!(&segments[0].raw, &format!("ldap user {lhs} '{payload}'"));
        prop_assert_eq!(&segments[1].raw, &format!("P {stage}"));
    }

    #[test]
    fn split_pipeline_preserves_pipe_inside_double_quotes(
        lhs in word(),
        left in double_quoted_fragment(),
        right in double_quoted_fragment(),
        stage in word(),
    ) {
        let payload = format!("{left}|{right}");
        let input = format!("ldap user {lhs} \"{payload}\" | P {stage}");

        let segments = split_pipeline(&input).expect("double-quoted payload should parse");
        prop_assert_eq!(segments.len(), 2);
        prop_assert_eq!(&segments[0].raw, &format!("ldap user {lhs} \"{payload}\""));
        prop_assert_eq!(&segments[1].raw, &format!("P {stage}"));
    }

    #[test]
    fn tokenize_stage_preserves_double_quoted_payload_text(payload in double_quoted_fragment()) {
        let raw = format!("F note=\"{payload}\"");
        let stage = StageSegment {
            raw: raw.clone(),
            span: Span {
                start: 0,
                end: raw.len(),
            },
        };

        let tokens = tokenize_stage(&stage).expect("double-quoted stage should tokenize");
        prop_assert_eq!(tokens.len(), 4);
        prop_assert_eq!(&tokens[0].text, "F");
        prop_assert_eq!(&tokens[1].text, "note");
        prop_assert_eq!(&tokens[3].text, &payload);
    }

    #[test]
    fn tokenize_stage_preserves_single_quoted_payload_text(payload in quoted_payload_fragment()) {
        let raw = format!("F note='{payload}'");
        let stage = StageSegment {
            raw: raw.clone(),
            span: Span {
                start: 0,
                end: raw.len(),
            },
        };

        let tokens = tokenize_stage(&stage).expect("single-quoted stage should tokenize");
        prop_assert_eq!(tokens.len(), 4);
        prop_assert_eq!(&tokens[0].text, "F");
        prop_assert_eq!(&tokens[1].text, "note");
        prop_assert_eq!(&tokens[3].text, &payload);
    }

    #[test]
    fn tokenize_stage_preserves_escaped_plain_spaces(
        left in escaped_plain_fragment(),
        right in escaped_plain_fragment(),
    ) {
        let raw = format!("F note={left}\\ {right}");
        let stage = StageSegment {
            raw: raw.clone(),
            span: Span {
                start: 0,
                end: raw.len(),
            },
        };

        let tokens = tokenize_stage(&stage).expect("escaped plain value should tokenize");
        prop_assert_eq!(tokens.len(), 4);
        prop_assert_eq!(&tokens[0].text, "F");
        prop_assert_eq!(&tokens[1].text, "note");
        prop_assert_eq!(&tokens[3].text, &format!("{left} {right}"));
    }

    #[test]
    fn split_pipeline_trims_separator_whitespace(lhs in word(), stage in word()) {
        let input = format!("  ldap user {lhs}   |   P {stage}  ");

        let segments = split_pipeline(&input).expect("pipeline should parse");
        prop_assert_eq!(segments.len(), 2);
        prop_assert_eq!(&segments[0].raw, &format!("ldap user {lhs}"));
        prop_assert_eq!(&segments[1].raw, &format!("P {stage}"));
    }

    #[test]
    fn parse_pipeline_rejects_trailing_escape_for_plain_segments(prefix in word()) {
        let input = format!("ldap user {prefix}\\");
        let err = parse_pipeline(&input).expect_err("trailing escape should fail");
        prop_assert!(
            matches!(err, LexerError::TrailingEscape { .. }),
            "unexpected error: {:?}",
            err
        );
    }
}
