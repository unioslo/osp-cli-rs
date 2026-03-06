use osp_dsl::parse::{
    lexer::{Span, StageSegment, tokenize_stage},
    pipeline::parse_pipeline,
};

fn render_segments(input: &str) -> Vec<String> {
    let parsed = parse_pipeline(input).expect("golden pipeline should parse");
    let mut segments = Vec::with_capacity(parsed.stages.len() + 1);
    if !parsed.command.is_empty() {
        segments.push(parsed.command);
    }
    segments.extend(parsed.stages);
    segments
}

#[test]
fn parser_stage_split_matches_checked_in_golden_cases() {
    let cases = [
        (
            "ldap user oistes | P uid,cn | F uid=oistes",
            vec!["ldap user oistes", "P uid,cn", "F uid=oistes"],
        ),
        (
            "ldap user 'foo|bar' | P uid",
            vec!["ldap user 'foo|bar'", "P uid"],
        ),
        (
            "ldap user \"foo|bar\" | P uid",
            vec!["ldap user \"foo|bar\"", "P uid"],
        ),
        (
            "ldap user foo\\|bar | P uid",
            vec!["ldap user foo\\|bar", "P uid"],
        ),
        (
            "ldap user oistes || P uid",
            vec!["ldap user oistes", "P uid"],
        ),
        (
            "ldap user oistes |  | P uid",
            vec!["ldap user oistes", "P uid"],
        ),
    ];

    for (input, expected) in cases {
        assert_eq!(
            render_segments(input),
            expected.into_iter().map(str::to_string).collect::<Vec<_>>(),
            "pipeline split mismatch for: {input}"
        );
    }
}

#[test]
fn parser_stage_tokenization_matches_checked_in_golden_cases() {
    let cases = [
        ("uid=oistes", vec!["uid", "=", "oistes"]),
        ("vlan>=75", vec!["vlan", ">=", "75"]),
        ("status != active", vec!["status", "!=", "active"]),
        ("status == ==online", vec!["status", "==", "==online"]),
        ("==online", vec!["==online"]),
        ("!?interfaces", vec!["!?interfaces"]),
        ("name \"foo bar\"", vec!["name", "foo bar"]),
        ("name=\"foo bar\"", vec!["name", "=", "foo bar"]),
        (r#"note "say \"hi\"""#, vec!["note", r#"say "hi""#]),
        (r#"path 'C:\Temp\'"#, vec!["path", r#"C:\Temp\"#]),
    ];

    for (input, expected) in cases {
        let segment = StageSegment {
            raw: input.to_string(),
            span: Span {
                start: 0,
                end: input.len(),
            },
        };
        let tokens = tokenize_stage(&segment)
            .expect("golden tokenization case should parse")
            .into_iter()
            .map(|token| token.text)
            .collect::<Vec<_>>();

        assert_eq!(
            tokens,
            expected.into_iter().map(str::to_string).collect::<Vec<_>>(),
            "token mismatch for: {input}"
        );
    }
}
