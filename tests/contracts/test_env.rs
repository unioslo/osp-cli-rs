use std::ffi::OsString;
use std::path::Path;

pub(crate) fn isolated_env(home: &Path) -> [(&'static str, OsString); 4] {
    [
        ("HOME", home.as_os_str().to_owned()),
        ("XDG_CONFIG_HOME", home.join(".config").into_os_string()),
        ("XDG_CACHE_HOME", home.join(".cache").into_os_string()),
        (
            "XDG_STATE_HOME",
            home.join(".local").join("state").into_os_string(),
        ),
    ]
}

#[allow(dead_code)]
pub(crate) fn isolated_env_with_config_home(
    home: &Path,
    xdg_config_home: &Path,
) -> [(&'static str, OsString); 4] {
    [
        ("HOME", home.as_os_str().to_owned()),
        ("XDG_CONFIG_HOME", xdg_config_home.into()),
        ("XDG_CACHE_HOME", home.join(".cache").into_os_string()),
        (
            "XDG_STATE_HOME",
            home.join(".local").join("state").into_os_string(),
        ),
    ]
}
