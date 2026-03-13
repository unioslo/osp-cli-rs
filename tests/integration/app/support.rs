#[cfg(unix)]
use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub(super) fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(crate) fn parse_json_output(
    label: &str,
    args: &[&str],
    stdout: &str,
    stderr: &str,
) -> serde_json::Value {
    serde_json::from_str(stdout).unwrap_or_else(|err| {
        panic!(
            "{label} stdout should be json: {err}\nargs: {:?}\nstdout:\n{}\nstderr:\n{}",
            args, stdout, stderr
        )
    })
}

#[cfg(unix)]
pub(super) fn with_path_prefix<T>(prefix: &Path, callback: impl FnOnce() -> T) -> T {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let previous = std::env::var_os("PATH");
    let joined = std::env::join_paths(
        std::iter::once(prefix.to_path_buf())
            .chain(previous.iter().flat_map(std::env::split_paths)),
    )
    .expect("PATH should join");
    unsafe {
        std::env::set_var("PATH", joined);
    }

    let result = callback();

    match previous {
        Some(value) => unsafe { std::env::set_var("PATH", value) },
        None => unsafe { std::env::remove_var("PATH") },
    }

    result
}

#[cfg(unix)]
pub(super) fn write_executable_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(path)
        .expect("plugin metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("plugin script should be executable");
}
