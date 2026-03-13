use super::render_document;
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::ui::RenderBackend;
use crate::ui::document::{
    Block, Document, JsonBlock, MregBlock, MregEntry, MregRow, MregValue, PanelBlock, TableAlign,
    TableBlock, TableStyle, ValueBlock, ValueLayout,
};
use crate::ui::format;
use crate::ui::{RenderRuntime, RenderSettings};
use crate::ui::{ResolvedRenderSettings, TableOverflow};
use serde_json::{Value, json};

fn settings(backend: RenderBackend, color: bool, unicode: bool) -> ResolvedRenderSettings {
    ResolvedRenderSettings {
        backend,
        color,
        unicode,
        width: None,
        margin: 0,
        indent_size: 2,
        short_list_max: 1,
        medium_list_max: 5,
        grid_padding: 4,
        grid_columns: None,
        column_weight: 3,
        table_overflow: TableOverflow::Clip,
        table_border: crate::ui::TableBorderStyle::Square,
        help_table_border: crate::ui::TableBorderStyle::None,
        theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
        theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
        style_overrides: crate::ui::style::StyleOverrides::default(),
        chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
    }
}

fn plain_settings_with_width(width: usize) -> ResolvedRenderSettings {
    let mut settings = settings(RenderBackend::Plain, false, false);
    settings.width = Some(width);
    settings
}

fn mreg_render_settings(width: usize) -> RenderSettings {
    RenderSettings {
        format: OutputFormat::Mreg,
        format_explicit: false,
        mode: RenderMode::Plain,
        color: ColorMode::Never,
        unicode: UnicodeMode::Never,
        width: Some(width),
        margin: 0,
        indent_size: 2,
        short_list_max: 1,
        medium_list_max: 5,
        grid_padding: 4,
        grid_columns: None,
        column_weight: 3,
        table_overflow: TableOverflow::Clip,
        table_border: crate::ui::TableBorderStyle::Square,
        help_chrome: crate::ui::HelpChromeSettings::default(),
        mreg_stack_min_col_width: 10,
        mreg_stack_overflow_ratio: 200,
        theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
        theme: None,
        style_overrides: crate::ui::style::StyleOverrides::default(),
        chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        ruled_section_policy: crate::ui::chrome::RuledSectionPolicy::PerSection,
        guide_default_format: crate::ui::GuideDefaultFormat::Guide,
        runtime: RenderRuntime::default(),
    }
}

fn trim_line_endings(value: &str) -> String {
    value
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn render_value_block_appends_trailing_newline() {
    let document = Document {
        blocks: vec![Block::Value(ValueBlock {
            values: vec!["one".to_string(), "two".to_string()],
            indent: 0,
            inline_markup: false,
            layout: ValueLayout::Vertical,
        })],
    };
    assert_eq!(
        render_document(&document, settings(RenderBackend::Plain, false, false)),
        "one\ntwo\n"
    );
}

#[test]
fn panel_rules_cover_plain_layout_and_margin_sizing_unit() {
    let plain_document = Document {
        blocks: vec![Block::Panel(PanelBlock {
            title: Some("Info".to_string()),
            body: Document {
                blocks: vec![Block::Value(ValueBlock {
                    values: vec!["alpha".to_string(), "beta".to_string()],
                    indent: 0,
                    inline_markup: false,
                    layout: ValueLayout::Vertical,
                })],
            },
            rules: crate::ui::document::PanelRules::Both,
            frame_style: None,
            kind: Some("info".to_string()),
            border_token: None,
            title_token: None,
        })],
    };

    assert_eq!(
        render_document(&plain_document, plain_settings_with_width(80)),
        concat!(
            "- Info -------------------------------------------------------------------------\n",
            "alpha\n",
            "beta\n",
            "--------------------------------------------------------------------------------\n"
        )
    );

    let document = Document {
        blocks: vec![Block::Panel(PanelBlock {
            title: Some("Commands".to_string()),
            body: Document {
                blocks: vec![Block::Value(ValueBlock {
                    values: vec!["alpha".to_string()],
                    indent: 2,
                    inline_markup: false,
                    layout: ValueLayout::Vertical,
                })],
            },
            rules: crate::ui::document::PanelRules::Top,
            frame_style: None,
            kind: None,
            border_token: None,
            title_token: None,
        })],
    };
    let mut settings = plain_settings_with_width(20);
    settings.margin = 4;

    let rendered = render_document(&document, settings);
    let mut lines = rendered.lines();
    let first_line = lines.next().expect("divider line");
    let body_line = lines.next().expect("body line");

    assert!(!first_line.starts_with(' '));
    assert!(first_line.starts_with("--- Commands "));
    assert_eq!(first_line.chars().count(), 20);
    assert!(body_line.starts_with("      alpha"));
}

#[test]
fn render_mreg_respects_color_toggle() {
    let block = MregBlock {
        block_id: 1,
        rows: vec![MregRow {
            entries: vec![MregEntry {
                key: "uid".to_string(),
                depth: 0,
                value: MregValue::Scalar(json!("oistes")),
            }],
        }],
    };
    let plain = render_document(
        &Document {
            blocks: vec![Block::Mreg(block.clone())],
        },
        settings(RenderBackend::Plain, false, false),
    );
    let colored = render_document(
        &Document {
            blocks: vec![Block::Mreg(block)],
        },
        settings(RenderBackend::Rich, true, false),
    );

    assert_eq!(plain, "uid: oistes\n");
    assert!(colored.contains("uid"));
    assert!(colored.contains("\x1b["));
}

#[test]
fn mreg_scalar_entries_render_inline() {
    let document = Document {
        blocks: vec![Block::Mreg(MregBlock {
            block_id: 1,
            rows: vec![MregRow {
                entries: vec![MregEntry {
                    key: "members".to_string(),
                    depth: 0,
                    value: MregValue::Scalar(json!("alice")),
                }],
            }],
        })],
    };

    let rendered = render_document(
        &document,
        ResolvedRenderSettings {
            backend: RenderBackend::Plain,
            color: false,
            unicode: false,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 3,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::ui::style::StyleOverrides::default(),
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );
    assert_eq!(rendered, "members: alice\n");
}

#[test]
fn mreg_large_lists_use_grid_layout() {
    let values = (1..=12)
        .map(|index| json!(format!("item-{index}")))
        .collect::<Vec<Value>>();
    let document = Document {
        blocks: vec![Block::Mreg(MregBlock {
            block_id: 1,
            rows: vec![MregRow {
                entries: vec![MregEntry {
                    key: "members".to_string(),
                    depth: 0,
                    value: MregValue::Grid(values),
                }],
            }],
        })],
    };

    let rendered = render_document(
        &document,
        ResolvedRenderSettings {
            backend: RenderBackend::Plain,
            color: false,
            unicode: false,
            width: Some(48),
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::ui::style::StyleOverrides::default(),
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );

    assert!(rendered.contains("members:"));
    assert!(
        rendered
            .lines()
            .any(|line| line.matches("item-").count() >= 2)
    );
}

#[test]
fn json_block_rendering_covers_pretty_plain_exact_plain_and_rich_colorized_output_unit() {
    let pretty_document = Document {
        blocks: vec![Block::Json(JsonBlock {
            payload: json!([{"uid": "oistes"}]),
        })],
    };
    let pretty = render_document(
        &pretty_document,
        settings(RenderBackend::Plain, false, false),
    );
    assert!(pretty.contains('\n'));
    assert!(pretty.contains("\"uid\""));

    let exact_document = Document {
        blocks: vec![Block::Json(JsonBlock {
            payload: json!({"uid":"oistes"}),
        })],
    };
    assert_eq!(
        render_document(
            &exact_document,
            settings(RenderBackend::Plain, false, false)
        ),
        "{\n  \"uid\": \"oistes\"\n}\n"
    );

    let rich_document = Document {
        blocks: vec![Block::Json(JsonBlock {
            payload: json!({"uid":"oistes","enabled":true,"count":2}),
        })],
    };
    let rich = render_document(&rich_document, settings(RenderBackend::Rich, true, true));
    assert!(rich.contains("\x1b["));
    assert!(rich.contains("\"uid\""));
    assert!(rich.contains("true"));
}

#[test]
fn grid_table_rendering_covers_border_style_color_unicode_and_ascii_fallbacks_unit() {
    let document = Document {
        blocks: vec![Block::Table(TableBlock {
            block_id: 1,
            style: TableStyle::Grid,
            border_override: None,
            headers: vec!["uid".to_string()],
            rows: vec![vec![json!("oistes")]],
            header_pairs: Vec::new(),
            align: None,
            shrink_to_fit: true,
            depth: 0,
        })],
    };

    let mut round = settings(RenderBackend::Rich, false, true);
    round.table_border = crate::ui::TableBorderStyle::Round;
    let mut square = settings(RenderBackend::Plain, false, false);
    square.table_border = crate::ui::TableBorderStyle::Square;
    let mut none = settings(RenderBackend::Rich, false, true);
    none.table_border = crate::ui::TableBorderStyle::None;

    let rounded = render_document(&document, round);
    let ascii = render_document(&document, square);
    let borderless = render_document(&document, none);

    assert!(rounded.contains('╭'));
    assert!(rounded.contains('│'));
    assert!(ascii.contains('+'));
    assert!(!borderless.contains('│'));
    assert!(!borderless.contains('┏'));

    let no_color = render_document(&document, settings(RenderBackend::Rich, false, true));
    assert!(!no_color.contains("\x1b["));

    let no_unicode = render_document(&document, settings(RenderBackend::Rich, false, false));
    for ch in ['┌', '┐', '└', '┘', '│', '─', '┬', '┴', '┼'] {
        assert!(!no_unicode.contains(ch));
    }

    assert!(
        ascii
            .lines()
            .any(|line| line.starts_with("+-") && line.contains('-'))
    );
    assert!(!ascii.lines().any(|line| line.starts_with("+=")));
}

#[test]
fn guide_tables_ignore_box_chrome_and_remain_borderless_unit() {
    let document = Document {
        blocks: vec![Block::Table(TableBlock {
            block_id: 1,
            style: TableStyle::Guide,
            border_override: None,
            headers: vec!["uid".to_string(), "role".to_string()],
            rows: vec![vec![json!("oistes"), json!("ops")]],
            header_pairs: Vec::new(),
            align: None,
            shrink_to_fit: true,
            depth: 0,
        })],
    };

    let mut settings = settings(RenderBackend::Plain, false, false);
    settings.help_table_border = crate::ui::TableBorderStyle::Square;

    let rendered = render_document(&document, settings);

    assert!(rendered.contains("uid"));
    assert!(rendered.contains("oistes"));
    assert!(!rendered.contains('+'));
    assert!(!rendered.contains('|'));
}

#[test]
fn ldap_user_sample_renders_as_python_style_mreg() {
    let Value::Object(row) = json!({
        "cn": "_istein S_vik",
        "eduPersonAffiliation": ["employee", "member", "staff"],
        "gecos": "\\istein S|vik",
        "gidNumber": "346297",
        "homeDirectory": "/uio/kant/usit-gsd-u1/oistes",
        "loginShell": "/local/gnu/bin/bash",
        "objectClass": [
            "uioMembership",
            "top",
            "account",
            "posixAccount",
            "uioAccountObject",
            "sambaSamAccount"
        ],
        "uid": "oistes",
        "uidNumber": "361000",
        "uioAffiliation": "ANSATT@373034",
        "uioPrimaryAffiliation": "ANSATT@373034",
        "netgroups": [
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
            "zabbix-iti-ops"
        ],
        "filegroups": ["oistes", "ucore", "usit", "vortex-opptak"]
    }) else {
        panic!("expected ldap user object");
    };

    let rows = vec![row];
    let settings = mreg_render_settings(80);
    let document = format::build_document(&rows, &settings);
    let rendered = render_document(&document, settings.resolve_render_settings());

    assert_eq!(
        trim_line_endings(&rendered),
        trim_line_endings(concat!(
            "cn:                    _istein S_vik\n",
            "eduPersonAffiliation (3): employee\n",
            "                          member\n",
            "                          staff\n",
            "gecos:                 \\istein S|vik\n",
            "gidNumber:             346297\n",
            "homeDirectory:         /uio/kant/usit-gsd-u1/oistes\n",
            "loginShell:            /local/gnu/bin/bash\n",
            "objectClass (6):\n",
            "  account            top             \n",
            "  posixAccount       uioAccountObject\n",
            "  sambaSamAccount    uioMembership   \n",
            "uid:                   oistes\n",
            "uidNumber:             361000\n",
            "uioAffiliation:        ANSATT@373034\n",
            "uioPrimaryAffiliation: ANSATT@373034\n",
            "netgroups (44):\n",
            "  ansatt-373034                       rt-it-uu-kontakt             \n",
            "  ansatt-tekadm-373034                rt-saksbehandler             \n",
            "  dia-drs-vaktsjefer                  rt-usit-intark-drift         \n",
            "  it-uio-azure-users                  rt-usit-lifeportal-utv-kunder\n",
            "  it-uio-ms365-ansatt                 rt-usit-ops                  \n",
            "  it-uio-ms365-ansatt-publisert       rt-usit-respons              \n",
            "  it-uio-ms365-eapp-acos-akademiet    ucore                        \n",
            "  los-alle                            uio-ans                      \n",
            "  mattermost-uio                      uio-tils                     \n",
            "  mattermost-uio-it                   usit                         \n",
            "  mattermost-usit                     vcs-cfengine                 \n",
            "  meta-ansatt-360000                  vcs-dhcp                     \n",
            "  meta-ansatt-370000                  vcs-it-org                   \n",
            "  meta-ansatt-373000                  vcs-it-osprov                \n",
            "  meta-ansatt-373034                  vcs-iti                      \n",
            "  meta-ansatt-900000                  vcs-ops                      \n",
            "  meta-ansatt-tekadm-360000           vcs-radius                   \n",
            "  meta-ansatt-tekadm-370000           vcs-ssd                      \n",
            "  meta-ansatt-tekadm-373000           vcs-usit                     \n",
            "  meta-ansatt-tekadm-373034           vcs-virtprov-admins          \n",
            "  meta-ansatt-tekadm-900000           vortex-opptak                \n",
            "  postmaster-eo-migrerte              zabbix-iti-ops               \n",
            "filegroups (4):        oistes\n",
            "                       ucore\n",
            "                       usit\n",
            "                       vortex-opptak\n"
        ))
    );
}

#[test]
fn nested_object_lists_stack_when_table_would_hide_content() {
    let Value::Object(row) = json!({
        "id": 55753,
        "ipaddresses": [
            {
                "id": 57171,
                "macaddress": "10:62:e5:19:74:4a",
                "created_at": "2019-12-02T21:50:27.600379+01:00",
                "updated_at": "2022-06-20T09:51:40.448942+02:00",
                "ipaddress": "129.240.130.83",
                "host": 55753
            },
            {
                "id": 57172,
                "macaddress": "",
                "created_at": "2019-12-02T21:50:28.054904+01:00",
                "updated_at": "2023-01-20T16:06:24.373064+01:00",
                "ipaddress": "2001:700:100:4003::83",
                "host": 55753
            }
        ],
        "networks": [
            {
                "policy": null,
                "communities": [
                    {
                        "id": 3,
                        "name": "laptops",
                        "description": "Laptops",
                        "network": 1733,
                        "global_name": "community02"
                    },
                    {
                        "id": 2,
                        "name": "workstations",
                        "description": "Workstations",
                        "network": 1733,
                        "global_name": "community01"
                    }
                ],
                "network": "129.240.130.0/24",
                "description": "knh-klientnett-2 (statisk DHCP)",
                "vlan": 200,
                "dns_delegated": false,
                "category": "kn",
                "location": "usit",
                "frozen": false,
                "reserved": 3,
                "max_communities": null
            },
            {
                "policy": null,
                "network": "2001:700:100:4003::/64",
                "description": "usit-knh",
                "vlan": 200,
                "dns_delegated": false,
                "category": "",
                "location": "",
                "frozen": false,
                "reserved": 3,
                "max_communities": null
            }
        ]
    }) else {
        panic!("expected object");
    };

    let rows = vec![row];
    let settings = mreg_render_settings(100);
    let document = format::build_document(&rows, &settings);
    let rendered = render_document(&document, settings.resolve_render_settings());

    assert!(rendered.contains("ipaddresses (2):"));
    assert!(rendered.contains("| id"));
    assert!(rendered.contains("networks (2):"));
    assert!(rendered.contains("communities (2):"));
    assert!(rendered.contains("community02"));
    assert!(rendered.contains("| global_name"));
    assert!(!rendered.contains("{'id': 3"));
}

#[test]
fn mreg_alignment_accounts_for_nested_depth() {
    let document = Document {
        blocks: vec![Block::Mreg(MregBlock {
            block_id: 1,
            rows: vec![MregRow {
                entries: vec![
                    MregEntry {
                        key: "parent".to_string(),
                        depth: 0,
                        value: MregValue::Scalar(json!("root")),
                    },
                    MregEntry {
                        key: "nested".to_string(),
                        depth: 1,
                        value: MregValue::Scalar(json!("leaf")),
                    },
                ],
            }],
        })],
    };

    let rendered = render_document(&document, settings(RenderBackend::Plain, false, false));
    let mut lines = rendered.lines();
    let top = lines.next().unwrap_or_default();
    let nested = lines.next().unwrap_or_default();
    assert!(top.starts_with("parent:  "));
    assert!(nested.starts_with("  nested: "));
}

#[test]
fn markdown_and_header_pair_tables_render_expected_surface_contracts_unit() {
    let header_pairs_document = Document {
        blocks: vec![Block::Table(TableBlock {
            block_id: 1,
            style: TableStyle::Grid,
            border_override: None,
            headers: vec!["uid".to_string()],
            rows: vec![vec![json!("oistes")]],
            header_pairs: vec![("group".to_string(), json!("ops"))],
            align: None,
            shrink_to_fit: true,
            depth: 0,
        })],
    };
    assert_eq!(
        render_document(
            &header_pairs_document,
            settings(RenderBackend::Plain, false, false)
        ),
        concat!(
            "group: ops  |  count: 1\n",
            "+--------+\n",
            "| uid    |\n",
            "+--------+\n",
            "| oistes |\n",
            "+--------+\n"
        )
    );

    let markdown_document = Document {
        blocks: vec![Block::Table(TableBlock {
            block_id: 1,
            style: TableStyle::Markdown,
            border_override: None,
            headers: vec!["name".to_string(), "count".to_string()],
            rows: vec![vec![json!("alice"), json!(42)]],
            header_pairs: Vec::new(),
            align: Some(vec![TableAlign::Left, TableAlign::Right]),
            shrink_to_fit: true,
            depth: 0,
        })],
    };
    let rendered = render_document(
        &markdown_document,
        settings(RenderBackend::Plain, false, false),
    );
    assert!(rendered.contains("| name"));
    assert!(rendered.contains("| alice"));
    let separator = rendered.lines().nth(1).expect("markdown separator row");
    let cells = separator.split('|').collect::<Vec<_>>();
    assert!(cells[1].trim().starts_with(':'));
    assert!(cells[2].trim().ends_with(':'));
}

#[test]
fn table_overflow_policies_cover_clip_none_ellipsis_and_wrap_unit() {
    let cases = [
        (
            "this-is-a-very-long-cell-that-should-truncate",
            Some(40usize),
            TableOverflow::Clip,
            false,
            false,
            false,
        ),
        (
            "this-is-a-very-long-cell-that-should-not-truncate",
            Some(20usize),
            TableOverflow::None,
            true,
            false,
            false,
        ),
        (
            "this-is-a-very-long-cell-that-should-truncate",
            Some(20usize),
            TableOverflow::Ellipsis,
            false,
            true,
            false,
        ),
        (
            "abcdefghijklmno",
            Some(12usize),
            TableOverflow::Wrap,
            false,
            false,
            true,
        ),
    ];

    for (value, width, overflow, keeps_full, has_ellipsis, keeps_tail) in cases {
        let document = Document {
            blocks: vec![Block::Table(TableBlock {
                block_id: 1,
                style: TableStyle::Grid,
                border_override: None,
                headers: vec!["uid".to_string()],
                rows: vec![vec![json!(value)]],
                header_pairs: Vec::new(),
                align: None,
                shrink_to_fit: true,
                depth: 0,
            })],
        };

        let rendered = render_document(
            &document,
            ResolvedRenderSettings {
                backend: RenderBackend::Plain,
                color: false,
                unicode: false,
                width,
                margin: 0,
                indent_size: 2,
                short_list_max: 1,
                medium_list_max: 5,
                grid_padding: 4,
                grid_columns: None,
                column_weight: 3,
                table_overflow: overflow,
                table_border: crate::ui::TableBorderStyle::Square,
                help_table_border: crate::ui::TableBorderStyle::None,
                theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
                theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
                style_overrides: crate::ui::style::StyleOverrides::default(),
                chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
            },
        );

        assert_eq!(
            rendered.contains(value),
            keeps_full,
            "overflow={overflow:?}"
        );
        assert_eq!(
            rendered.contains("..."),
            has_ellipsis,
            "overflow={overflow:?}"
        );
        if keeps_tail {
            assert!(rendered.contains("mno"), "overflow={overflow:?}");
        }
    }
}

#[test]
fn auto_grid_value_block_preserves_order_and_splits_long_lists() {
    let document = Document {
        blocks: vec![Block::Value(ValueBlock {
            values: vec![
                "`F` key>3".to_string(),
                "`P` col1 col2".to_string(),
                "`S` sort_key".to_string(),
                "`G` group_by".to_string(),
                "`A` metric()".to_string(),
                "`L` limit offset".to_string(),
            ],
            indent: 2,
            inline_markup: true,
            layout: ValueLayout::AutoGrid,
        })],
    };

    let rendered = render_document(
        &document,
        ResolvedRenderSettings {
            backend: RenderBackend::Plain,
            color: false,
            unicode: true,
            width: Some(40),
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 3,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::ui::style::StyleOverrides::default(),
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );

    let lines = rendered.lines().collect::<Vec<_>>();
    assert!(lines[0].starts_with("  F key>3"));
    assert!(lines[0].contains("G group_by"));
    assert!(lines[1].starts_with("  P col1 col2"));
    assert!(lines[1].contains("A metric()"));
    assert!(lines[2].starts_with("  S sort_key"));
    assert!(lines[2].contains("L limit offset"));
}

#[test]
fn style_and_theme_overrides_cover_hex_code_text_and_value_rendering_unit() {
    let mreg_document = Document {
        blocks: vec![Block::Mreg(MregBlock {
            block_id: 1,
            rows: vec![MregRow {
                entries: vec![MregEntry {
                    key: "color".to_string(),
                    depth: 0,
                    value: MregValue::Scalar(json!("#ff00ff")),
                }],
            }],
        })],
    };
    let plain_theme_rendered = render_document(
        &mreg_document,
        ResolvedRenderSettings {
            backend: RenderBackend::Rich,
            color: true,
            unicode: false,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: "plain".to_string(),
            theme: crate::ui::theme::resolve_theme("plain"),
            style_overrides: crate::ui::style::StyleOverrides::default(),
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );
    assert!(!plain_theme_rendered.contains("\x1b["));

    let themed_hex = render_document(
        &mreg_document,
        ResolvedRenderSettings {
            backend: RenderBackend::Rich,
            color: true,
            unicode: true,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::ui::style::StyleOverrides::default(),
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );
    assert!(themed_hex.contains("\x1b[38;2;255;0;255m"));

    let code_document = Document {
        blocks: vec![Block::Code(crate::ui::document::CodeBlock {
            code: "let x = 1;".to_string(),
            language: Some("rust".to_string()),
        })],
    };
    let code_rendered = render_document(
        &code_document,
        ResolvedRenderSettings {
            backend: RenderBackend::Rich,
            color: true,
            unicode: true,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::ui::style::StyleOverrides {
                code: Some("#00ff00".to_string()),
                ..Default::default()
            },
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );
    assert!(code_rendered.contains("\x1b[38;2;0;255;0m"));
    assert!(code_rendered.contains("let x = 1;"));

    let value_document = Document {
        blocks: vec![Block::Value(ValueBlock {
            values: vec!["alpha".to_string()],
            indent: 0,
            inline_markup: false,
            layout: ValueLayout::Vertical,
        })],
    };
    let value_rendered = render_document(
        &value_document,
        ResolvedRenderSettings {
            backend: RenderBackend::Rich,
            color: true,
            unicode: true,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::ui::style::StyleOverrides {
                text: Some("#224466".to_string()),
                ..Default::default()
            },
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );
    assert!(value_rendered.contains("\x1b[38;2;34;68;102malpha\x1b[0m"));

    let table_document = Document {
        blocks: vec![Block::Table(TableBlock {
            block_id: 1,
            style: TableStyle::Grid,
            border_override: None,
            headers: vec!["name".to_string()],
            rows: vec![vec![serde_json::json!("alice")]],
            header_pairs: Vec::new(),
            align: None,
            shrink_to_fit: false,
            depth: 0,
        })],
    };
    let table_rendered = render_document(
        &table_document,
        ResolvedRenderSettings {
            backend: RenderBackend::Rich,
            color: true,
            unicode: false,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME),
            style_overrides: crate::ui::style::StyleOverrides {
                value: Some("#cc5500".to_string()),
                ..Default::default()
            },
            chrome_frame: crate::ui::chrome::SectionFrameStyle::Top,
        },
    );
    assert!(table_rendered.contains("\x1b[38;2;204;85;0malice\x1b[0m"));
}
