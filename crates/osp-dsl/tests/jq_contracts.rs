use std::{
    env,
    process::Command,
    sync::{Mutex, OnceLock},
};

use osp_core::output_model::OutputResult;
use osp_dsl::apply_pipeline;
use serde_json::{Map, Value, json};

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("fixture must be object")
}

fn output_rows(output: &OutputResult) -> &[Map<String, Value>] {
    output.as_rows().expect("expected row output")
}

fn flat_rows() -> Vec<Map<String, Value>> {
    vec![
        obj(json!({
            "host": "alpha",
            "networks": [
                {"network": "129.240.130.0/24", "vlan": 200},
                {"network": "2001:700:100:4003::/64", "vlan": 303}
            ],
        })),
        obj(json!({
            "host": "beta",
            "networks": [{"network": "129.240.130.11/24", "vlan": 75}],
        })),
        obj(json!({
            "host": "gamma",
            "networks": [],
        })),
    ]
}

fn large_rows() -> Vec<Map<String, Value>> {
    let payload = "x".repeat(4096);
    (0..128)
        .map(|index| {
            obj(json!({
                "id": index,
                "payload": payload,
            }))
        })
        .collect()
}

fn jq_available() -> bool {
    Command::new("jq")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn path_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct PathGuard {
    original: Option<String>,
}

impl PathGuard {
    fn clear() -> Self {
        let original = env::var("PATH").ok();
        // Use an impossible directory instead of empty to avoid shell quirks.
        // Safety: this test file serializes PATH mutation with a process-local
        // mutex and restores the original value in Drop.
        unsafe {
            env::set_var("PATH", "/__osp_no_such_bin_dir__");
        }
        Self { original }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        match &self.original {
            // Safety: see PathGuard::clear. Mutation is serialized and scoped
            // to this test guard.
            Some(value) => unsafe { env::set_var("PATH", value) },
            // Safety: see PathGuard::clear.
            None => unsafe { env::remove_var("PATH") },
        }
    }
}

#[test]
fn jq_identity_matches_python_contract() {
    let _guard = path_lock().lock().expect("lock should not be poisoned");
    if !jq_available() {
        eprintln!("skipping jq contract test: jq executable not available");
        return;
    }

    let rows = flat_rows();
    let output = apply_pipeline(rows.clone(), &["JQ .".to_string()]).expect("jq should succeed");
    assert_eq!(output_rows(&output), rows.as_slice());
}

#[test]
fn jq_select_first_network_matches_python_contract() {
    let _guard = path_lock().lock().expect("lock should not be poisoned");
    if !jq_available() {
        eprintln!("skipping jq contract test: jq executable not available");
        return;
    }

    let output = apply_pipeline(flat_rows(), &["JQ .[0].networks[0]".to_string()])
        .expect("jq should succeed");
    let expected = vec![obj(json!({"network": "129.240.130.0/24", "vlan": 200}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn jq_map_hosts_matches_python_contract() {
    let _guard = path_lock().lock().expect("lock should not be poisoned");
    if !jq_available() {
        eprintln!("skipping jq contract test: jq executable not available");
        return;
    }

    let output = apply_pipeline(flat_rows(), &[r#"JQ "[.[0].host, .[1].host]""#.to_string()])
        .expect("jq should succeed");
    let expected = vec![
        obj(json!({"value": "alpha"})),
        obj(json!({"value": "beta"})),
    ];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn jq_leading_pipe_matches_python_contract() {
    let _guard = path_lock().lock().expect("lock should not be poisoned");
    if !jq_available() {
        eprintln!("skipping jq contract test: jq executable not available");
        return;
    }

    let output = apply_pipeline(flat_rows(), &[r#"JQ "| .[0].host""#.to_string()])
        .expect("jq should succeed");
    let expected = vec![obj(json!({"value": "alpha"}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn jq_length_matches_python_contract() {
    let _guard = path_lock().lock().expect("lock should not be poisoned");
    if !jq_available() {
        eprintln!("skipping jq contract test: jq executable not available");
        return;
    }

    let output = apply_pipeline(flat_rows(), &[r#"JQ ". | length""#.to_string()])
        .expect("jq should succeed");
    let expected = vec![obj(json!({"value": 3}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn jq_missing_binary_returns_clear_error() {
    let _guard = path_lock().lock().expect("lock should not be poisoned");
    let _path = PathGuard::clear();

    let error = apply_pipeline(flat_rows(), &["JQ .".to_string()])
        .expect_err("missing jq binary should return an error");

    assert!(
        error.to_string().contains("jq executable not found"),
        "unexpected error: {error}"
    );
}

#[test]
fn jq_large_identity_payload_roundtrips_contract() {
    let _guard = path_lock().lock().expect("lock should not be poisoned");
    if !jq_available() {
        eprintln!("skipping jq contract test: jq executable not available");
        return;
    }

    let rows = large_rows();
    let output = apply_pipeline(rows.clone(), &["JQ .".to_string()])
        .expect("jq should handle large payloads");

    assert_eq!(output_rows(&output), rows.as_slice());
}
