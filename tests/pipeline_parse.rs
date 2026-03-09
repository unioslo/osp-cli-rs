use osp_cli::cli::pipeline::{parse_command_text_with_aliases, parse_command_tokens_with_aliases};
use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions, ResolvedConfig};

fn make_config(entries: &[(&str, &str)]) -> ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    for (key, value) in entries {
        defaults.set(*key, *value);
    }

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve")
}

#[test]
fn parse_allows_quick_pipe_segments() {
    let config = make_config(&[]);
    let parsed = parse_command_text_with_aliases("status | last | K foo", &config)
        .expect("quick pipes should parse");

    assert_eq!(parsed.tokens, vec!["status"]);
    assert_eq!(parsed.stages, vec!["last", "K foo"]);
}

#[test]
fn parse_preserves_quoted_pipe_inside_command_segment() {
    let config = make_config(&[]);
    let parsed = parse_command_text_with_aliases("ldap user 'foo|bar' | P uid", &config)
        .expect("quoted pipe command should parse");

    assert_eq!(parsed.tokens, vec!["ldap", "user", "foo|bar"]);
    assert_eq!(parsed.stages, vec!["P uid"]);
}

#[test]
fn parse_merges_orch_os_tokens_from_text() {
    let config = make_config(&[]);
    let parsed = parse_command_text_with_aliases("orch provision --os alma 9", &config)
        .expect("orch provision command should parse");

    assert_eq!(parsed.tokens, vec!["orch", "provision", "--os", "alma9"]);
}

#[test]
fn parse_merges_orch_os_tokens_from_cli_tokens() {
    let config = make_config(&[]);
    let tokens = ["orch", "provision", "--os", "alma", "9"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let parsed = parse_command_tokens_with_aliases(&tokens, &config)
        .expect("orch provision token list should parse");

    assert_eq!(parsed.tokens, vec!["orch", "provision", "--os", "alma9"]);
}

#[test]
fn parse_does_not_merge_outside_orch_provision() {
    let config = make_config(&[]);
    let parsed = parse_command_text_with_aliases("orch task create --os alma 9", &config)
        .expect("non-provision orch command should parse");

    assert_eq!(
        parsed.tokens,
        vec!["orch", "task", "create", "--os", "alma", "9"]
    );
}

#[test]
fn parse_does_not_merge_when_version_is_dash() {
    let config = make_config(&[]);
    let parsed = parse_command_text_with_aliases("orch provision --os alma -", &config)
        .expect("skip-marker orch provision command should parse");

    assert_eq!(
        parsed.tokens,
        vec!["orch", "provision", "--os", "alma", "-"]
    );
}

#[test]
fn parse_rejects_unknown_explicit_stage_from_text() {
    let config = make_config(&[]);
    let result = parse_command_text_with_aliases("status | X foo", &config);

    assert!(
        result.is_err(),
        "explicit single-letter unknown DSL stages should be rejected"
    );
}

#[test]
fn parse_accepts_quick_pipe_segments_from_cli_tokens() {
    let config = make_config(&[]);
    let tokens = ["status", "|", "last", "|", "K", "foo"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let parsed = parse_command_tokens_with_aliases(&tokens, &config)
        .expect("quick pipe tokens should parse");

    assert_eq!(parsed.tokens, vec!["status"]);
    assert_eq!(parsed.stages, vec!["last", "K foo"]);
}

#[test]
fn parse_cli_tokens_preserves_single_stage_argument_with_spaces() {
    let config = make_config(&[]);
    let tokens = ["status", "|", "K", "foo bar"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let parsed = parse_command_tokens_with_aliases(&tokens, &config)
        .expect("quoted-like token stage should parse");

    assert_eq!(parsed.tokens, vec!["status"]);
    assert_eq!(parsed.stages, vec!["K 'foo bar'"]);
}

#[test]
fn parse_rejects_unknown_explicit_stage_from_alias_pipe() {
    let config = make_config(&[("alias.bad", "status | X foo")]);
    let result = parse_command_text_with_aliases("bad", &config);

    assert!(
        result.is_err(),
        "alias-expanded pipes should be validated the same as user pipes"
    );
}

#[test]
fn parse_accepts_cli_help_stage_from_text() {
    let config = make_config(&[]);
    let parsed =
        parse_command_text_with_aliases("status | H F", &config).expect("help pipe should parse");

    assert_eq!(parsed.tokens, vec!["status"]);
    assert_eq!(parsed.stages, vec!["H F"]);
}

#[test]
fn parse_accepts_cli_help_stage_from_alias_pipe() {
    let config = make_config(&[("alias.helpme", "status | H G")]);
    let parsed = parse_command_text_with_aliases("helpme", &config)
        .expect("alias-expanded help pipe should parse");

    assert_eq!(parsed.tokens, vec!["status"]);
    assert_eq!(parsed.stages, vec!["H G"]);
}
