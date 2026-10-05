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

fn assert_job_toolchain(source: &str, job: &str, channel: &str) {
    let job_header = format!("  {job}:");
    let lines = source
        .lines()
        .skip_while(|line| *line != job_header)
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with("    "))
        .collect::<Vec<_>>();
    assert!(!lines.is_empty(), "workflow job {job} should exist");
    assert!(
        !lines.contains(&"    if: false"),
        "toolchain verification job {job} must be enabled",
    );
    let workflow_ref = format!("dtolnay/rust-toolchain@{channel}");
    assert!(
        lines.iter().any(|line| {
            line.trim().strip_prefix("uses:").is_some_and(|reference| {
                reference.split('#').next().unwrap().trim() == workflow_ref
            })
        }),
        "job {job} should install pinned Rust toolchain {channel}",
    );
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

    for workflow in [
        ".github/workflows/verify.yml",
        ".github/workflows/release.yml",
    ] {
        let path = workspace_root().join(workflow);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        assert_job_toolchain(&source, "verify", &toolchain_channel);
    }

    let confidence = fs::read_to_string(workspace_root().join("scripts/confidence.py"))
        .expect("confidence lane definitions should be readable");
    let miri_channel = confidence
        .lines()
        .find_map(|line| line.strip_prefix("MIRI_TOOLCHAIN = "))
        .expect("confidence lanes should pin Miri")
        .trim_matches('"');
    assert!(
        miri_channel.starts_with("nightly-"),
        "Miri must use a dated nightly"
    );
    let verify = fs::read_to_string(workspace_root().join(".github/workflows/verify.yml"))
        .expect("verification workflow should be readable");
    assert_job_toolchain(&verify, "miri", miri_channel);
}
