use osp_api::MockLdapClient;
use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_dsl::{apply_pipeline, parse_pipeline};
use osp_ports::LdapDirectory;
use osp_ui::theme::DEFAULT_THEME_NAME;
use osp_ui::{RenderSettings, StyleOverrides, render_rows};

#[test]
fn dsl_pipeline_project_works_on_ldap_user_data() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .user("oistes", None, None)
        .expect("query should succeed");

    let parsed = parse_pipeline("ldap user oistes | P uid,cn");
    let transformed = apply_pipeline(rows, &parsed.stages).expect("pipeline should succeed");

    let settings = RenderSettings {
        format: OutputFormat::Table,
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
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        style_overrides: StyleOverrides::default(),
    };
    let output = render_rows(&transformed, &settings);

    assert!(output.contains("uid"));
    assert!(output.contains("cn"));
    assert!(!output.contains("homeDirectory"));
}

#[test]
fn dsl_pipeline_values_works_on_netgroup_members() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .netgroup("ucore", None, None)
        .expect("query should succeed");

    let parsed = parse_pipeline("ldap netgroup ucore | P members | VAL members");
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
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        style_overrides: StyleOverrides::default(),
    };
    let output = render_rows(&transformed, &settings);

    assert!(output.contains("oistes"));
    assert!(!output.contains("description"));
}

#[test]
fn dsl_pipeline_filter_works() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .netgroup("ucore", None, None)
        .expect("query should succeed");

    let parsed = parse_pipeline("ldap netgroup ucore | F cn=ucore | P cn");
    let transformed = apply_pipeline(rows, &parsed.stages).expect("pipeline should succeed");

    let settings = RenderSettings {
        format: OutputFormat::Mreg,
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
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        style_overrides: StyleOverrides::default(),
    };
    let output = render_rows(&transformed, &settings);

    assert!(output.contains("ucore"));
}

#[test]
fn dsl_pipeline_markdown_table_format_works() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .user("oistes", None, None)
        .expect("query should succeed");

    let parsed = parse_pipeline("ldap user oistes | P uid,cn");
    let transformed = apply_pipeline(rows, &parsed.stages).expect("pipeline should succeed");

    let settings = RenderSettings {
        format: OutputFormat::Markdown,
        mode: RenderMode::Plain,
        color: ColorMode::Never,
        unicode: UnicodeMode::Never,
        width: Some(200),
        margin: 0,
        indent_size: 2,
        short_list_max: 1,
        medium_list_max: 5,
        grid_padding: 4,
        grid_columns: None,
        column_weight: 3,
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        style_overrides: StyleOverrides::default(),
    };
    let output = render_rows(&transformed, &settings);

    let mut lines = output.lines();
    let header = lines.next().unwrap_or_default();
    let separator = lines.next().unwrap_or_default();
    assert!(header.contains("uid") && header.contains("cn"));
    assert!(separator.contains("---"));
    assert!(output.contains("oistes"));
}
