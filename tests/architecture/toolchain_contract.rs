use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn parse_toml(path: &str) -> toml::Value {
    let file = workspace_root().join(path);
    toml::from_str(
        &fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display())),
    )
    .unwrap_or_else(|err| panic!("failed to parse {}: {err}", file.display()))
}

fn cargo_rust_version() -> String {
    parse_toml("Cargo.toml")["package"]["rust-version"]
        .as_str()
        .expect("Cargo.toml package.rust-version should be a string")
        .to_string()
}

fn pinned_toolchain_channel() -> String {
    parse_toml("rust-toolchain.toml")["toolchain"]["channel"]
        .as_str()
        .expect("rust-toolchain.toml toolchain.channel should be a string")
        .to_string()
}

fn normalize_minor_version(version: &str) -> String {
    let mut parts = version.split('.');
    let major = parts.next().expect("version should have major component");
    let minor = parts.next().expect("version should have minor component");
    format!("{major}.{minor}")
}

#[test]
fn pinned_rust_toolchain_stays_aligned_across_metadata_and_ci() {
    let cargo_version = cargo_rust_version();
    let toolchain_channel = pinned_toolchain_channel();

    assert_eq!(
        cargo_version,
        normalize_minor_version(&toolchain_channel),
        "Cargo.toml rust-version and rust-toolchain.toml channel drifted",
    );

    let workflow_ref = format!("dtolnay/rust-toolchain@{toolchain_channel}");
    for workflow in [
        ".github/workflows/verify.yml",
        ".github/workflows/release.yml",
    ] {
        let path = workspace_root().join(workflow);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        assert!(
            source.contains(&workflow_ref),
            "{} should install the pinned Rust toolchain {}",
            path.display(),
            toolchain_channel
        );
    }
}
