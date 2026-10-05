use osp_cli::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_cli::core::output_model::{
    OutputDocument, OutputDocumentKind, OutputMeta, OutputResult, output_items_from_value,
};
use osp_cli::dsl::{apply_output_pipeline, parse_pipeline};
use osp_cli::guide::{GuideSection, GuideSectionKind, GuideView};
use osp_cli::ui::{RenderRuntime, RenderSettings, TableBorderStyle, TableOverflow, render_output};
use serde_json::{Value, json};

fn export_settings(format: OutputFormat) -> RenderSettings {
    RenderSettings::builder()
        .with_format(format)
        .with_format_explicit(true)
        .with_mode(RenderMode::Plain)
        .with_color(ColorMode::Never)
        .with_unicode(UnicodeMode::Never)
        .with_width(100)
        .with_width_max(100)
        .with_margin(2)
        .with_indent_size(2)
        .with_table_overflow(TableOverflow::None)
        .with_table_border(TableBorderStyle::None)
        .with_runtime(
            RenderRuntime::builder()
                .with_stdout_is_tty(false)
                .with_terminal("dumb")
                .with_no_color(true)
                .with_width(100)
                .with_locale_utf8(true)
                .build(),
        )
        .build()
}

#[test]
fn service_record_exports_preserve_nested_typed_data() {
    let record = json!({
        "uid": "alice",
        "enabled": true,
        "quota_gib": 42.5,
        "roles": ["operator", "reviewer", "dns", "directory", "vm", "audit", "backup"],
        "contacts": {
            "email": "alice@example.com",
            "routes": [
                {"kind": "primary", "address": "192.0.2.8"},
                {"kind": "backup", "address": "2001:db8::8"}
            ]
        },
        "approval": {
            "decision": "approved",
            "evidence": [
                {"reviewer": "bob", "sources": ["ticket", "review"]},
                ["deploy", "audit"],
                true
            ]
        },
        "labels": [["primary", "prod"], ["backup", "stage"]]
    });
    let output = OutputResult::from_rows(Vec::new()).with_document(OutputDocument::new(
        OutputDocumentKind::Json,
        record.clone(),
    ));

    let json_export = render_output(&output, &export_settings(OutputFormat::Json));
    assert_eq!(serde_json::from_str::<Value>(&json_export).unwrap(), record);

    let mreg = render_output(&output, &export_settings(OutputFormat::Mreg));
    for expected in [
        "uid:",
        "alice",
        "enabled:",
        "true",
        "42.5",
        "contacts:",
        "email:",
        "alice@example.com",
        "routes (2):",
        "192.0.2.8",
        "2001:db8::8",
        "approved",
        "bob",
        "ticket",
        "review",
        "deploy",
        "audit",
        "prod",
        "stage",
        "operator",
        "reviewer",
        "dns",
        "directory",
        "vm",
        "backup",
    ] {
        assert!(
            mreg.contains(expected),
            "missing {expected:?} in Mreg export:\n{mreg}"
        );
    }

    let markdown = render_output(&output, &export_settings(OutputFormat::Markdown));
    for expected in [
        "- uid: alice",
        "- enabled: true",
        "- quota_gib: 42.5",
        "- contacts:",
        "- email: alice@example.com",
        "- routes (2):",
        "- address: 192.0.2.8",
        "- address: 2001:db8::8",
        "- evidence (3):",
        "- reviewer: bob",
        "- ticket",
        "- review",
        "- deploy",
        "- audit",
        "- primary",
        "- prod",
        "- stage",
    ] {
        assert!(
            markdown.contains(expected),
            "missing {expected:?} in Markdown export:\n{markdown}"
        );
    }
    assert_eq!(output.document.unwrap().value, record);
}

#[test]
fn grouped_report_exports_restore_aggregates_and_member_rows() {
    let members = [
        json!({"region": "eu|west", "uid": "alice|operator", "quota": 1.5, "roles": ["dns", "vm"]}),
        json!({"region": "eu|west", "uid": "bob", "quota": 2.0, "roles": ["review"]}),
        json!({"region": "us", "uid": "carol", "quota": 4.0, "roles": ["audit"]}),
    ];
    let rows = members
        .iter()
        .map(|member| member.as_object().unwrap().clone())
        .collect();
    let pipeline = parse_pipeline("service-report | G region | A sum(quota) AS total | S region")
        .expect("report aggregation should parse");
    let grouped = apply_output_pipeline(OutputResult::from_rows(rows), &pipeline.stages)
        .expect("report aggregation should execute");
    let canonical = json!([
        {
            "groups": {"region": "eu|west"},
            "aggregates": {"total": 3.5},
            "rows": [members[0], members[1]]
        },
        {
            "groups": {"region": "us"},
            "aggregates": {"total": 4.0},
            "rows": [members[2]]
        }
    ]);
    let exported = render_output(&grouped, &export_settings(OutputFormat::Json));
    let restored_value: Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(restored_value, canonical);

    let restored = OutputResult {
        items: output_items_from_value(restored_value),
        document: None,
        meta: OutputMeta {
            grouped: true,
            ..Default::default()
        },
    };
    assert_eq!(restored.items, grouped.items);
    assert_eq!(
        serde_json::from_str::<Value>(&render_output(
            &restored,
            &export_settings(OutputFormat::Json)
        ),)
        .unwrap(),
        canonical
    );

    let markdown = render_output(&restored, &export_settings(OutputFormat::Markdown));
    for expected in [
        "- region: eu|west",
        "- total: 3.5",
        "- region: us",
        "- total: 4",
        "eu\\|west",
        "alice\\|operator",
        "bob",
        "carol",
        "dns, vm",
        "review",
        "audit",
    ] {
        assert!(
            markdown.contains(expected),
            "missing {expected:?} in grouped Markdown:\n{markdown}"
        );
    }
    let mreg = render_output(&restored, &export_settings(OutputFormat::Mreg));
    for expected in [
        "region:",
        "eu|west",
        "us",
        "total:",
        "3.5",
        "4",
        "alice|operator",
        "bob",
        "carol",
        "dns, vm",
    ] {
        assert!(
            mreg.contains(expected),
            "missing {expected:?} in grouped Mreg:\n{mreg}"
        );
    }
}

#[test]
fn operational_guide_exports_keep_authored_entries_and_nested_policy_data() {
    let guide = GuideView {
        sections: vec![
            GuideSection::new("Usage", GuideSectionKind::Usage)
                .paragraph("osp site host apply HOST [OPTIONS]"),
            GuideSection::new("Operations", GuideSectionKind::Custom)
                .paragraph("Use `site host show` after **approval**.")
                .entry("site host show", "Read the `canonical` host record")
                .entry("site host apply", "Apply an approved change")
                .entry("site whoami", "")
                .entry("site host show | P uid", "Copy the `uid` field"),
            GuideSection::new("Arguments", GuideSectionKind::Arguments)
                .entry("HOST", "Canonical host name"),
            GuideSection::new("Options", GuideSectionKind::Options)
                .entry("--dry-run", "Preview the approved change"),
            GuideSection::new(
                "Common Invocation Options",
                GuideSectionKind::CommonInvocationOptions,
            )
            .entry("--profile", "Select the service profile")
            .entry("--json", "Export typed results"),
            GuideSection::new("Policy", GuideSectionKind::Custom).data(json!({
                "owner": "alice",
                "enabled": true,
                "reviewers": ["bob", "carol"],
                "service": {
                    "name": "directory",
                    "checks": [
                        {"name": "identity", "required": true},
                        ["dns", "network"]
                    ]
                }
            })),
        ],
        ..Default::default()
    };
    let output = guide.to_output_result();
    let exported = render_output(&output, &export_settings(OutputFormat::Json));
    assert_eq!(
        serde_json::from_str::<Value>(&exported).unwrap(),
        json!([guide.to_json_value()])
    );

    // An offline consumer restores the exported row payload without a live sidecar document.
    let imported = OutputResult {
        items: output_items_from_value(serde_json::from_str(&exported).unwrap()),
        document: None,
        meta: OutputMeta::default(),
    };
    let restored = GuideView::try_from_output_result(&imported).unwrap();
    assert_eq!(restored.to_json_value(), guide.to_json_value());
    assert_eq!(restored.usage, ["osp site host apply HOST [OPTIONS]"]);
    assert_eq!(restored.arguments[0].name, "HOST");
    assert_eq!(restored.options[0].name, "--dry-run");
    assert_eq!(
        restored
            .common_invocation_options
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["--profile", "--json"]
    );
    assert_eq!(
        restored
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect::<Vec<_>>(),
        [
            "Usage",
            "Operations",
            "Arguments",
            "Options",
            "Common Invocation Options",
            "Policy"
        ]
    );

    let reference = parse_pipeline("runbook | F name ~ ^--").unwrap();
    let flags = apply_output_pipeline(restored.to_output_result(), &reference.stages).unwrap();
    let flag_guide = GuideView::try_from_output_result(&flags).unwrap();
    assert_eq!(flag_guide.options[0].name, "--dry-run");
    assert_eq!(
        flag_guide
            .common_invocation_options
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["--profile", "--json"]
    );
    let flag_markdown = render_output(&flags, &export_settings(OutputFormat::Markdown));
    for flag in ["--dry-run", "--profile", "--json"] {
        assert!(flag_markdown.contains(flag), "{flag_markdown}");
    }

    let terminal = render_output(&output, &export_settings(OutputFormat::Guide));
    let visible_words = terminal.split_whitespace().collect::<Vec<_>>().join(" ");
    for expected in [
        "Operations",
        "Use site host show after approval.",
        "site host show Read the canonical host record",
        "site host apply Apply an approved change",
        "site whoami",
        "site host show | P uid Copy the uid field",
        "Policy",
        "alice",
        "true",
        "bob",
        "carol",
        "directory",
        "identity",
        "dns",
        "network",
    ] {
        assert!(
            visible_words.contains(expected),
            "missing {expected:?} in terminal guide:\n{terminal}"
        );
    }
    let markdown = render_output(&output, &export_settings(OutputFormat::Markdown));
    for expected in [
        "Operations",
        "Use `site host show` after **approval**.",
        "- `site host show` Read the `canonical` host record",
        "- `site host apply` Apply an approved change",
        "- `site whoami`",
        "- `site host show | P uid` Copy the `uid` field",
        "- owner: alice",
        "- enabled: true",
        "- reviewers (2):",
        "- bob",
        "- carol",
        "- name: directory",
        "- name: identity",
        "- dns",
        "- network",
    ] {
        assert!(
            markdown.contains(expected),
            "missing {expected:?} in guide Markdown:\n{markdown}"
        );
    }
    let mreg = render_output(&output, &export_settings(OutputFormat::Mreg));
    for expected in [
        "Operations",
        "site host show",
        "site host apply",
        "Policy",
        "alice",
        "bob",
        "carol",
        "directory",
        "identity",
        "dns",
        "network",
    ] {
        assert!(
            mreg.contains(expected),
            "missing {expected:?} in guide Mreg:\n{mreg}"
        );
    }
    assert_eq!(
        GuideView::try_from_output_result(&output)
            .unwrap()
            .to_json_value(),
        guide.to_json_value()
    );
}
