use crate::assert_snapshot_text;
use osp_cli::core::OutputResult;
use osp_cli::core::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_cli::dsl::apply_pipeline;
use osp_cli::ui::{RenderSettings, render};
use serde_json::json;

#[test]
fn startup_is_a_compact_console_overview_not_a_pipe_manual() {
    let output = crate::test_env::command(assert_cmd::cargo::cargo_bin!("osp"))
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
        lines.contains("Welcome anonymous!\nUser: Not signed in\nTheme: Dracula"),
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
    assert!(plain.lines().count() <= 40, "{plain}");
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
    settings.runtime.stdout_is_tty = true;
    settings.style_overrides.ipv4 = Some("red".into());
    settings.style_overrides.ipv6 = Some("green".into());
    let rendered = render(&output, &osp_cli::ui::RenderRequest::new(&settings));
    assert!(rendered.contains("\x1b[31m192.0.2.1\x1b[0m"));
    assert!(rendered.contains("\x1b[32m2001:db8::1\x1b[0m"));
    assert!(!rendered.contains("\x1b[31m999.0.0.1"));
    settings.color = ColorMode::Never;
    let plain = render(&output, &osp_cli::ui::RenderRequest::new(&settings));
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
    settings.runtime.stdout_is_tty = true;

    let rendered = render(&output, &osp_cli::ui::RenderRequest::new(&settings));

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

    let rendered = render(&output, &osp_cli::ui::RenderRequest::new(&settings));

    assert_snapshot_text!("single_row_key_value_block_rich", rendered);
}

#[test]
fn quick_filtered_single_row_key_value_block_plain_snapshot_contract() {
    let output = quick_filtered_single_row_output();
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    settings.width = Some(60);

    let rendered = render(&output, &osp_cli::ui::RenderRequest::new(&settings));

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

    let rendered = render(&output, &osp_cli::ui::RenderRequest::new(&settings));

    assert_snapshot_text!("quick_filtered_single_row_key_value_block_rich", rendered);
}

#[cfg(unix)]
#[test]
fn projected_cli_results_export_to_the_terminal_clipboard_contract() {
    let home = crate::temp_support::make_temp_dir("osp-cli-terminal-copy");
    let bin = assert_cmd::cargo::cargo_bin!("osp");
    let args = [
        bin.to_str().expect("CLI executable path should be UTF-8"),
        "--defaults-only",
        "--json",
        "theme",
        "list",
        "|",
        "F name=dracula",
        "|",
        "P name",
        "|",
        "Y",
    ];
    let command = args
        .iter()
        .map(|arg| shell_words::quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let output = crate::test_env::command("script")
        .env_clear()
        .envs(crate::test_env::isolated_env(home.path()))
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("OSC52", "yes")
        .args(["-qfec", &command, "/dev/null"])
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let captured = String::from_utf8(output).expect("terminal output should be UTF-8");
    let prefix = "\x1b]52;c;";
    let start = captured
        .find(prefix)
        .expect("terminal should receive an OSC52 clipboard write");
    let payload_start = start + prefix.len();
    let end = payload_start
        + captured[payload_start..]
            .find('\x07')
            .expect("clipboard write should terminate");
    let decoded = crate::test_env::command("python3")
        .env_clear()
        .envs(crate::test_env::isolated_env(home.path()))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "-c",
            "import base64,sys; sys.stdout.buffer.write(base64.b64decode(sys.argv[1], validate=True))",
            &captured[payload_start..end],
        ])
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let copied: serde_json::Value =
        serde_json::from_slice(&decoded).expect("copied result should remain typed JSON");
    let visible = format!("{}{}", &captured[..start], &captured[end + 1..]);
    let visible: serde_json::Value =
        serde_json::from_str(&visible).expect("visible result should remain typed JSON");
    assert_eq!(copied, json!([{"name": "Dracula"}]));
    assert_eq!(visible, copied);
}
