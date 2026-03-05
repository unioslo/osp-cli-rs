use osp_core::output_model::OutputItems;
use osp_dsl::{apply_pipeline, execute_pipeline};
use serde_json::{Map, Value, json};

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("fixture must be object")
}

fn flat_rows() -> Vec<Map<String, Value>> {
    vec![
        obj(json!({
            "value": "host-a",
            "dept": "USIT",
            "status": "active",
            "interfaces": [
                {"mac": "aa:bb", "ip": "129.240.130.10"},
                {"mac": "cc:dd", "ip": "2001:700:100::1"}
            ],
            "netgroups": ["ansatt-373034", "uio", "usit"],
            "networks": [
                {"network": "129.240.130.0/24", "vlan": 200},
                {"network": "2001:700:100:4003::/64", "vlan": 303}
            ],
        })),
        obj(json!({
            "value": "host-b",
            "dept": "USIT",
            "status": "inactive",
            "interfaces": [{"mac": "ee:ff", "ip": "129.240.130.11"}],
            "netgroups": ["ansatt-373034"],
            "networks": [],
        })),
        obj(json!({
            "value": "standalone",
            "dept": null,
            "status": null,
            "interfaces": [],
            "netgroups": [],
            "networks": [],
        })),
    ]
}

#[test]
fn project_dotted_path_and_absolute_scope_match_python_behavior() {
    let nested = vec![obj(json!({
        "id": 55753,
        "txts": {"id": 27994},
        "ipaddresses": [{"id": 57171}, {"id": 57172}],
        "metadata": {"asset": {"id": 42}}
    }))];

    let output =
        apply_pipeline(nested.clone(), &["P asset.id".to_string()]).expect("pipeline should pass");
    assert_eq!(
        output,
        vec![obj(json!({"metadata": {"asset": {"id": 42}}}))]
    );

    let output =
        apply_pipeline(nested, &["P .asset.id".to_string()]).expect("pipeline should pass");
    assert!(output.is_empty());
}

#[test]
fn project_selector_forms_match_python_contract() {
    let rows = flat_rows();

    let output =
        apply_pipeline(rows.clone(), &["P interfaces[].mac".to_string()]).expect("project");
    assert_eq!(
        output,
        vec![
            obj(json!({"mac": "aa:bb"})),
            obj(json!({"mac": "cc:dd"})),
            obj(json!({"mac": "ee:ff"})),
        ]
    );

    let output = apply_pipeline(rows.clone(), &["P netgroups[0]".to_string()]).expect("project");
    assert_eq!(
        output,
        vec![
            obj(json!({"netgroups": "ansatt-373034"})),
            obj(json!({"netgroups": "ansatt-373034"})),
        ]
    );

    let output = apply_pipeline(rows.clone(), &["P netgroups[2]".to_string()]).expect("project");
    assert_eq!(output, vec![obj(json!({"netgroups": "usit"}))]);

    let output = apply_pipeline(rows.clone(), &["P netgroups[::-1]".to_string()]).expect("project");
    assert_eq!(output[0].get("netgroups"), Some(&json!("usit")));

    let output = apply_pipeline(rows, &["P interfaces[1:]".to_string()]).expect("project");
    assert_eq!(
        output,
        vec![obj(json!({"mac": "cc:dd", "ip": "2001:700:100::1"}))]
    );
}

#[test]
fn filter_handles_dotted_paths_and_group_streams() {
    let rows = flat_rows();
    let output =
        apply_pipeline(rows.clone(), &["F interfaces[].mac=aa:bb".to_string()]).expect("filter");
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].get("value"), Some(&json!("host-a")));

    let grouped = execute_pipeline(
        vec![
            obj(json!({"dept": "sales", "active": true})),
            obj(json!({"dept": "sales", "active": false})),
            obj(json!({"dept": "eng", "active": true})),
        ],
        &["G dept".to_string(), "F ?active".to_string()],
    )
    .expect("group pipeline should pass");

    match grouped.items {
        OutputItems::Groups(groups) => {
            assert_eq!(groups.len(), 2);
            assert!(groups.iter().all(|group| !group.rows.is_empty()));
        }
        OutputItems::Rows(_) => panic!("expected grouped output"),
    }
}
