#[cfg(unix)]
fn write_config(home: &std::path::Path, config: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    std::fs::write(config_dir.join("config.toml"), config).expect("config should be written");
}

#[cfg(unix)]
fn write_secrets(home: &std::path::Path, secrets: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    let secrets_path = config_dir.join("secrets.toml");
    std::fs::write(&secrets_path, secrets).expect("secrets should be written");
    let mut perms = std::fs::metadata(&secrets_path)
        .expect("secrets metadata")
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(&secrets_path, perms).expect("secrets permissions");
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

#[cfg(unix)]
fn parse_json_stdout(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be valid json: {err}\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}

#[cfg(unix)]
fn first_json_row<'a>(payload: &'a serde_json::Value, context: &str) -> &'a serde_json::Value {
    payload
        .as_array()
        .unwrap_or_else(|| panic!("{context} should render a JSON array"))
        .first()
        .unwrap_or_else(|| panic!("{context} should render at least one row"))
}
