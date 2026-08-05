use crate::assert_snapshot_text;
use osp_cli::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_cli::core::output_model::OutputResult;
use osp_cli::dsl::apply_pipeline;
use osp_cli::ui::{RenderSettings, render_output};
use serde_json::json;

fn representative_single_row_output() -> OutputResult {
    let mut output = OutputResult::from_rows(vec![osp_cli::row! {
        "cn" => "Oistein Sovik",
        "uid" => "oistes",
        "uidNumber" => 361000,
        "uioPrimaryAffiliation" => "ANSATT@373034",
        "netgroups" => json!([
            "ansatt-373034",
            "ansatt-tekadm-373034",
            "dia-drs-vaktsjefer",
            "it-uio-azure-users",
            "it-uio-ms365-ansatt",
            "it-uio-ms365-ansatt-publisert",
            "it-uio-ms365-eapp-acos-akademiet",
            "los-alle",
            "mattermost-uio",
            "mattermost-uio-it",
            "mattermost-usit",
            "meta-ansatt-360000",
            "meta-ansatt-370000",
            "meta-ansatt-373000",
            "meta-ansatt-373034",
            "meta-ansatt-900000",
            "meta-ansatt-tekadm-360000",
            "meta-ansatt-tekadm-370000",
            "meta-ansatt-tekadm-373000",
            "meta-ansatt-tekadm-373034",
            "meta-ansatt-tekadm-900000",
            "postmaster-eo-migrerte",
            "rt-it-uu-kontakt",
            "rt-saksbehandler",
            "rt-usit-intark-drift",
            "rt-usit-lifeportal-utv-kunder",
            "rt-usit-ops",
            "rt-usit-respons",
            "ucore",
            "uio-ans",
            "uio-tils",
            "usit",
            "vcs-cfengine",
            "vcs-dhcp",
            "vcs-it-org",
            "vcs-it-osprov",
            "vcs-iti",
            "vcs-ops",
            "vcs-radius",
            "vcs-ssd",
            "vcs-usit",
            "vcs-virtprov-admins",
            "vortex-opptak",
            "zabbix-iti-ops",
        ]),
        "filegroups" => json!(["oistes", "ucore", "usit", "vortex-opptak"]),
        "dn" => "uid=oistes,cn=users,cn=system,dc=uio,dc=no",
        "eduPersonAffiliation" => json!(["employee", "member", "staff"]),
        "gidNumber" => 346297,
        "uioAffiliation" => "ANSATT@373034",
        "objectClass" => json!([
            "uioMembership",
            "top",
            "account",
            "posixAccount",
            "uioAccountObject",
        ]),
        "loginShell" => "/local/gnu/bin/bash",
        "homeDirectory" => "/uio/kant/usit-gsd-u1/oistes",
        "gecos" => "\\istein S|vik",
    }]);
    output.meta.key_index = vec![
        "cn".to_string(),
        "uid".to_string(),
        "uidNumber".to_string(),
        "uioPrimaryAffiliation".to_string(),
        "netgroups".to_string(),
        "filegroups".to_string(),
        "dn".to_string(),
        "eduPersonAffiliation".to_string(),
        "gidNumber".to_string(),
        "uioAffiliation".to_string(),
        "objectClass".to_string(),
        "loginShell".to_string(),
        "homeDirectory".to_string(),
        "gecos".to_string(),
    ];
    output
}

fn quick_filtered_single_row_output() -> OutputResult {
    apply_pipeline(
        vec![osp_cli::row! {
            "cn" => "Oistein Sovik",
            "uid" => "oistes",
            "uidNumber" => 361000,
            "uioPrimaryAffiliation" => "ANSATT@373034",
            "netgroups" => json!([
                "ansatt-373034",
                "ansatt-tekadm-373034",
                "dia-drs-vaktsjefer",
                "it-uio-azure-users",
                "it-uio-ms365-ansatt",
                "it-uio-ms365-ansatt-publisert",
                "it-uio-ms365-eapp-acos-akademiet",
                "los-alle",
                "mattermost-uio",
                "mattermost-uio-it",
                "mattermost-usit",
                "meta-ansatt-360000",
                "meta-ansatt-370000",
                "meta-ansatt-373000",
                "meta-ansatt-373034",
                "meta-ansatt-900000",
                "meta-ansatt-tekadm-360000",
                "meta-ansatt-tekadm-370000",
                "meta-ansatt-tekadm-373000",
                "meta-ansatt-tekadm-373034",
                "meta-ansatt-tekadm-900000",
                "postmaster-eo-migrerte",
                "rt-it-uu-kontakt",
                "rt-saksbehandler",
                "rt-usit-intark-drift",
                "rt-usit-lifeportal-utv-kunder",
                "rt-usit-ops",
                "rt-usit-respons",
                "ucore",
                "uio-ans",
                "uio-tils",
                "usit",
                "vcs-cfengine",
                "vcs-dhcp",
                "vcs-it-org",
                "vcs-it-osprov",
                "vcs-iti",
                "vcs-ops",
                "vcs-radius",
                "vcs-ssd",
                "vcs-usit",
                "vcs-virtprov-admins",
                "vortex-opptak",
                "zabbix-iti-ops",
            ]),
            "filegroups" => json!(["oistes", "ucore", "usit", "vortex-opptak"]),
            "dn" => "uid=oistes,cn=users,cn=system,dc=uio,dc=no",
            "eduPersonAffiliation" => json!(["employee", "member", "staff"]),
            "gidNumber" => 346297,
            "uioAffiliation" => "ANSATT@373034",
            "objectClass" => json!([
                "uioMembership",
                "top",
                "account",
                "posixAccount",
                "uioAccountObject",
            ]),
            "loginShell" => "/local/gnu/bin/bash",
            "homeDirectory" => "/uio/kant/usit-gsd-u1/oistes",
            "gecos" => "\\istein S|vik",
        }],
        &["vcs".to_string()],
    )
    .expect("quick-filtered output should render")
}

#[test]
fn single_row_key_value_block_plain_snapshot_contract() {
    let output = representative_single_row_output();
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    settings.width = Some(120);

    let rendered = render_output(&output, &settings);

    assert_snapshot_text!("single_row_key_value_block_plain", rendered);
}

#[test]
fn single_row_key_value_block_rich_snapshot_contract() {
    let output = representative_single_row_output();
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    settings.width = Some(120);
    settings.mode = RenderMode::Rich;
    settings.color = ColorMode::Always;
    settings.unicode = UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.theme_name = "dracula".to_string();

    let rendered = render_output(&output, &settings);

    assert_snapshot_text!("single_row_key_value_block_rich", rendered);
}

#[test]
fn quick_filtered_single_row_key_value_block_plain_snapshot_contract() {
    let output = quick_filtered_single_row_output();
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    settings.width = Some(60);

    let rendered = render_output(&output, &settings);

    assert_snapshot_text!("quick_filtered_single_row_key_value_block_plain", rendered);
}

#[test]
fn quick_filtered_single_row_key_value_block_rich_snapshot_contract() {
    let output = quick_filtered_single_row_output();
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    settings.width = Some(60);
    settings.mode = RenderMode::Rich;
    settings.color = ColorMode::Always;
    settings.unicode = UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.theme_name = "dracula".to_string();

    let rendered = render_output(&output, &settings);

    assert_snapshot_text!("quick_filtered_single_row_key_value_block_rich", rendered);
}
