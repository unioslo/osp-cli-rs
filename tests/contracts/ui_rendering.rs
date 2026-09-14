use crate::assert_snapshot_text;
use osp_cli::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_cli::core::output_model::OutputResult;
use osp_cli::dsl::apply_pipeline;
use osp_cli::ui::{RenderSettings, render_output};
use serde_json::json;

#[test]
fn contextual_messages_preserve_event_order_and_theme_controls() {
    use osp_cli::ui::messages::{MessageBuffer, MessageLayout, MessageLevel};
    let mut messages = MessageBuffer::new();
    messages.warning("Task still running.");
    messages.push_titled(
        MessageLevel::Error,
        "Host lookup failed",
        "No matching host found.",
    );
    let theme = osp_cli::ui::theme::resolve_theme("dracula");
    let rendered = messages.render_grouped_styled(
        MessageLevel::Warning,
        true,
        true,
        Some(72),
        &theme,
        MessageLayout::Grouped,
    );
    let plain = crate::output_support::strip_ansi(&rendered);
    assert!(plain.find("Warning").unwrap() < plain.find("Host lookup failed").unwrap());
    assert!(!plain.contains("Errors") && !plain.contains("Warnings"));
    assert!(rendered.contains("\x1b[38;2;241;250;140m"));
    assert!(rendered.contains("\x1b[1;38;2;255;85;85m"));
    assert!(rendered.contains("\x1b[38;2;248;248;242m  No matching host found."));
    let uncolored = messages.render_grouped_styled(
        MessageLevel::Warning,
        false,
        false,
        Some(72),
        &theme,
        MessageLayout::Grouped,
    );
    assert!(uncolored.contains("Host lookup failed") && !uncolored.contains('\x1b'));
}

#[test]
fn startup_is_a_compact_console_overview_not_a_pipe_manual() {
    let output = assert_cmd::Command::new(assert_cmd::cargo::cargo_bin!("osp"))
        .args([
            "--defaults-only",
            "--theme",
            "dracula",
            "--color",
            "always",
            "--unicode",
            "always",
            "intro",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rendered = String::from_utf8(output).unwrap();
    let plain = crate::output_support::strip_ansi(&rendered);
    let lines = plain.lines().map(str::trim).collect::<Vec<_>>().join("\n");
    assert!(
        lines.contains("Welcome anonymous!\nUser: Not authenticated\nTheme: Dracula"),
        "{plain}"
    );
    let pipes = plain
        .split("Pipes")
        .nth(1)
        .unwrap()
        .split("Usage:")
        .next()
        .unwrap();
    assert!(pipes.lines().count() <= 6, "{pipes}");
    assert!(pipes.contains("text search") && pipes.contains("| H <verb>"));
    assert!(plain.lines().count() <= 33, "{plain}");
    assert!(rendered.contains("\x1b[38;2;189;147;249mOSP"));
    assert!(rendered.contains("\x1b[38;2;104;121;173mShow this command overview."));
}

#[test]
fn address_colors_follow_theme_overrides_only_in_colored_output() {
    let output = OutputResult::from_rows(vec![osp_cli::row! {
        "ipv4" => "192.0.2.1", "ipv6" => "2001:db8::1", "text" => "999.0.0.1"
    }]);
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.color = ColorMode::Always;
    settings.mode = RenderMode::Rich;
    settings.style_overrides.ipv4 = Some("red".into());
    settings.style_overrides.ipv6 = Some("green".into());
    let rendered = render_output(&output, &settings);
    assert!(rendered.contains("\x1b[31m192.0.2.1\x1b[0m"));
    assert!(rendered.contains("\x1b[32m2001:db8::1\x1b[0m"));
    assert!(!rendered.contains("\x1b[31m999.0.0.1"));
    settings.color = ColorMode::Never;
    let plain = render_output(&output, &settings);
    assert!(plain.contains("192.0.2.1") && plain.contains("2001:db8::1"));
    assert!(!plain.contains('\x1b'));
}

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
