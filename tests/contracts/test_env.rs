use std::ffi::OsString;
use std::path::Path;

pub(crate) fn isolated_env(home: &Path) -> Vec<(&'static str, OsString)> {
    let mut env = vec![
        ("HOME", home.as_os_str().to_owned()),
        ("XDG_CONFIG_HOME", home.join(".config").into_os_string()),
        ("XDG_CACHE_HOME", home.join(".cache").into_os_string()),
        (
            "XDG_STATE_HOME",
            home.join(".local").join("state").into_os_string(),
        ),
    ];
    // env_clear callers must keep the instrumented child process's report
    // destination, otherwise their asserted CLI behavior is absent from coverage.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        env.push(("LLVM_PROFILE_FILE", profile));
    }
    env
}

#[allow(dead_code)]
pub(crate) fn isolated_env_with_config_home(
    home: &Path,
    xdg_config_home: &Path,
) -> Vec<(&'static str, OsString)> {
    let mut env = isolated_env(home);
    env[1].1 = xdg_config_home.into();
    env
}
