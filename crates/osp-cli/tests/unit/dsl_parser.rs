use osp_dsl::parse_pipeline;

#[test]
fn parse_pipeline_preserves_quoted_pipe_in_command() {
    let parsed = parse_pipeline("ldap user 'foo|bar' | P uid");
    assert_eq!(parsed.command, "ldap user 'foo|bar'");
    assert_eq!(parsed.stages, vec!["P uid"]);
}
