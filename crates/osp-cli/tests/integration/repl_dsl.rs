use osp_api::MockLdapClient;
use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_dsl::{apply_pipeline, parse_pipeline};
use osp_ports::LdapDirectory;
use osp_ui::theme::DEFAULT_THEME_NAME;
use osp_ui::{RenderRuntime, RenderSettings, StyleOverrides, render_output};
use serde_json::json;

#[test]
fn dsl_pipeline_project_works_on_ldap_user_data() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .user("oistes", None, None)
        .expect("query should succeed");

    let parsed = parse_pipeline("ldap user oistes | P uid,cn").expect("valid pipeline");
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
        table_overflow: osp_ui::TableOverflow::Clip,
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        theme: None,
        style_overrides: StyleOverrides::default(),
        runtime: RenderRuntime::default(),
    };
    let output = render_output(&transformed, &settings);

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

    let parsed =
        parse_pipeline("ldap netgroup ucore | P members | VAL members").expect("valid pipeline");
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
        runtime: RenderRuntime::default(),
    };
    let output = render_output(&transformed, &settings);

    assert!(output.contains("oistes"));
    assert!(!output.contains("description"));
}

#[test]
fn dsl_pipeline_filter_works() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .netgroup("ucore", None, None)
        .expect("query should succeed");

    let parsed = parse_pipeline("ldap netgroup ucore | F cn=ucore | P cn").expect("valid pipeline");
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
        table_overflow: osp_ui::TableOverflow::Clip,
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        theme: None,
        style_overrides: StyleOverrides::default(),
        runtime: RenderRuntime::default(),
    };
    let output = render_output(&transformed, &settings);

    assert!(output.contains("ucore"));
}

#[test]
fn dsl_pipeline_markdown_table_format_works() {
    let ldap = MockLdapClient::default();
    let rows = ldap
        .user("oistes", None, None)
        .expect("query should succeed");

    let parsed = parse_pipeline("ldap user oistes | P uid,cn").expect("valid pipeline");
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
        table_overflow: osp_ui::TableOverflow::Clip,
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        theme: None,
        style_overrides: StyleOverrides::default(),
        runtime: RenderRuntime::default(),
    };
    let output = render_output(&transformed, &settings);

    let mut lines = output.lines();
    let header = lines.next().unwrap_or_default();
    let separator = lines.next().unwrap_or_default();
    assert!(header.contains("uid") && header.contains("cn"));
    assert!(separator.contains("---"));
    assert!(output.contains("oistes"));
}

#[test]
fn dsl_pipeline_grouped_output_renders_without_flattening() {
    let rows = vec![
        json!({"dept": "sales", "host": "alpha"})
            .as_object()
            .cloned()
            .expect("row fixture"),
        json!({"dept": "sales", "host": "beta"})
            .as_object()
            .cloned()
            .expect("row fixture"),
        json!({"dept": "eng", "host": "gamma"})
            .as_object()
            .cloned()
            .expect("row fixture"),
    ];

    let transformed = apply_pipeline(rows, &["G dept".to_string()]).expect("pipeline should succeed");

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
        table_overflow: osp_ui::TableOverflow::Clip,
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: DEFAULT_THEME_NAME.to_string(),
        theme: None,
        style_overrides: StyleOverrides::default(),
        runtime: RenderRuntime::default(),
    };
    let output = render_output(&transformed, &settings);

    assert!(output.contains("dept"));
    assert!(output.contains("sales"));
    assert!(output.contains("eng"));
    assert!(output.contains("alpha"));
    assert!(output.contains("gamma"));
}
