use osp_cli::config::RuntimeConfig;
use osp_cli::ports::mock::MockLdapClient;
use osp_cli::services::{ServiceContext, execute_line};

fn rows(output: &osp_cli::core::output_model::OutputResult) -> &[osp_cli::core::row::Row] {
    output.as_rows().expect("expected row output")
}

#[test]
fn service_execute_line_uses_context_user_and_dsl_projection() {
    let ctx = ServiceContext::new(
        Some("oistes".to_string()),
        MockLdapClient::default(),
        RuntimeConfig::default(),
    );

    let output = execute_line(&ctx, "ldap user --filter uid=oistes | P uid,netgroups")
        .expect("service command should execute");
    let rows = rows(&output);

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("uid").and_then(|value| value.as_str()),
        Some("oistes")
    );
    assert!(rows[0].contains_key("netgroups"));
    assert!(!rows[0].contains_key("cn"));
    assert!(!rows[0].contains_key("homeDirectory"));
}

#[test]
fn service_execute_line_combines_port_attribute_selection_with_dsl_pipeline() {
    let ctx = ServiceContext::new(None, MockLdapClient::default(), RuntimeConfig::default());

    let output = execute_line(
        &ctx,
        "ldap netgroup ucore --filter members=oistes --attributes cn,members | P members",
    )
    .expect("service command should execute");
    let rows = rows(&output);

    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains_key("members"));
    assert!(!rows[0].contains_key("cn"));
    let members = rows[0]
        .get("members")
        .and_then(|value| value.as_array())
        .expect("members should stay an array");
    assert!(members.iter().any(|value| value.as_str() == Some("oistes")));
}
