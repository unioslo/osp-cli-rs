use crate::output_support::{first_json_row, parse_json_stdout};

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
use crate::temp_support::make_temp_dir;
