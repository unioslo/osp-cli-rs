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
