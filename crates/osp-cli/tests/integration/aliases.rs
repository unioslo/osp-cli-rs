use osp_api::MockLdapClient;
use osp_cli::pipeline::parse_command_text_with_aliases;
use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
use osp_core::output::OutputFormat;
use osp_dsl::apply_pipeline;
use osp_ports::LdapDirectory;
use osp_ui::{render_output, RenderSettings};

fn make_config(entries: &[(&str, &str)]) -> osp_config::ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("profile.active", "default");
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
fn alias_expands_internal_and_user_pipes() {
    let config = make_config(&[("alias.lsng", "ldap netgroup ${1} --value | P members")]);
    let parsed = parse_command_text_with_aliases("lsng ucore | VAL members", &config)
        .expect("alias expansion should succeed");

    assert_eq!(parsed.tokens, vec!["ldap", "netgroup", "ucore", "--value"]);
    assert_eq!(parsed.stages, vec!["P members", "VAL members"]);

    let ldap = MockLdapClient::default();
    let rows = ldap
        .netgroup("ucore", None, None)
        .expect("query should succeed");
    let transformed = apply_pipeline(rows, &parsed.stages).expect("pipeline should succeed");

    let settings = RenderSettings::test_plain(OutputFormat::Value);
    let output = render_output(&transformed, &settings);
    assert!(output.contains("oistes"));
}

#[test]
fn alias_resolves_config_placeholders_and_splat() {
    let config = make_config(&[
        ("user.name", "testuser"),
        ("alias.me", "ldap user ${user.name}"),
        ("alias.splat", "ldap user ${@}"),
        ("alias.with_default", "ldap user ${1:guest}"),
    ]);

    let parsed =
        parse_command_text_with_aliases("me", &config).expect("alias expansion should succeed");
    assert_eq!(parsed.tokens, vec!["ldap", "user", "testuser"]);

    let parsed = parse_command_text_with_aliases("splat \"foo bar\" baz", &config)
        .expect("alias expansion should succeed");
    assert_eq!(parsed.tokens, vec!["ldap", "user", "foo bar", "baz"]);

    let parsed = parse_command_text_with_aliases("with_default", &config)
        .expect("alias expansion should succeed");
    assert_eq!(parsed.tokens, vec!["ldap", "user", "guest"]);
}
