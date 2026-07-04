#![allow(missing_docs)]

use osp_cli::core::fuzzy::{completion_fuzzy_matcher, search_fuzzy_matcher};
use osp_cli::core::output::OutputFormat;
use osp_cli::core::output_model::OutputResult;
use osp_cli::row;
use osp_cli::ui::{RenderSettings, render_output};
use serde_json::json;

#[test]
fn miri_fuzzy_fallback_keeps_completion_and_search_contracts() {
    assert!(
        completion_fuzzy_matcher()
            .fuzzy_match("ldap", "lap")
            .is_some()
    );
    assert!(
        search_fuzzy_matcher()
            .fuzzy_indices("doctor --mreg", "doctr mreg")
            .is_some()
    );
}

#[test]
fn miri_nested_key_value_rendering_stays_structured() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "oistes",
        "netgroups" => json!(["it-uio-alle-drift", "vcs-virtprov-admins"]),
        "profile" => json!({
            "owner": "oistes",
            "groups": ["usit", "vortex-opptak"],
        }),
    }]);
    output.meta.key_index = vec![
        "uid".to_string(),
        "netgroups".to_string(),
        "profile".to_string(),
    ];

    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("uid:"));
    assert!(rendered.contains("netgroups (2):"));
    assert!(rendered.contains("it-uio-alle-drift"));
    assert!(rendered.contains("profile:"));
    assert!(rendered.contains("groups (2): usit"));
    assert!(!rendered.contains("[\"it-uio-alle-drift\""));
}
