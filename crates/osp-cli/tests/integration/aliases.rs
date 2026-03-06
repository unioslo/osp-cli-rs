use osp_api::MockLdapClient;
use osp_cli::pipeline::parse_command_text_with_aliases;
use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_dsl::apply_pipeline;
use osp_ports::LdapDirectory;
use osp_ui::theme::DEFAULT_THEME_NAME;
use osp_ui::{RenderSettings, StyleOverrides, render_rows};

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

    let settings = RenderSettings {
        format: OutputFormat::Value,
        mode: RenderMode::Plain,
        color: ColorMode::Never,
        unicode: UnicodeMode::Never,
        width: None,
        margin: 0,
        indent_size: 2,
        short_list_max: 1,
        medium_list_max: 5,
        grid_padding: 4,
        grid_columns: None,
        column_weight: 3,
        table_overflow: osp_ui::TableOverflow::Clip,
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        theme: None,
        style_overrides: StyleOverrides::default(),
    };
    let output = render_rows(&transformed, &settings);
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
