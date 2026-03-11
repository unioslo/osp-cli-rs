use crate::core::output::OutputFormat;
use std::ffi::{OsStr, OsString};
use std::ops::Deref;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

pub(crate) fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Owns a test temp directory and removes it when dropped.
///
/// Tests should keep this alive for as long as any child paths are in use
/// instead of discarding ownership and leaking a raw `PathBuf`.
#[must_use = "keep the temp dir alive for as long as child paths are used"]
pub(crate) struct TestTempDir(tempfile::TempDir);

impl TestTempDir {
    pub(crate) fn new(prefix: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("temp dir should be created");
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<Path> for TestTempDir {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl AsRef<OsStr> for TestTempDir {
    fn as_ref(&self) -> &OsStr {
        self.path().as_os_str()
    }
}

impl Deref for TestTempDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path()
    }
}

pub(crate) fn make_temp_dir(prefix: &str) -> TestTempDir {
    TestTempDir::new(prefix)
}

/// Owns a process-env sandbox for tests that exercise the full app host path.
///
/// These tests still bootstrap against process state such as `HOME`, `XDG_*`,
/// `PATH`, `TERM`, and `OSP_*` runtime hints. Keep them in one scoped sandbox
/// so command-surface tests do not inherit whatever previous tests left behind.
#[must_use = "keep the env sandbox alive for the duration of the test invocation"]
pub(crate) struct TestProcessEnv {
    _guard: MutexGuard<'static, ()>,
    _root: TestTempDir,
    saved: Vec<(OsString, Option<OsString>)>,
}

impl TestProcessEnv {
    pub(crate) fn new(prefix: &str) -> Self {
        let guard = env_lock().lock().expect("env lock should not be poisoned");
        let root = make_temp_dir(prefix);
        let mut sandbox = Self {
            _guard: guard,
            _root: root,
            saved: Vec::new(),
        };
        sandbox.apply_baseline();
        sandbox
    }

    fn apply_baseline(&mut self) {
        let home = self._root.path().to_path_buf();
        let xdg_config = home.join(".config");
        let xdg_cache = home.join(".cache");
        let xdg_state = home.join(".local").join("state");
        std::fs::create_dir_all(&xdg_config).expect("xdg config dir should be created");
        std::fs::create_dir_all(&xdg_cache).expect("xdg cache dir should be created");
        std::fs::create_dir_all(&xdg_state).expect("xdg state dir should be created");

        self.set("HOME", home.as_os_str());
        self.set("XDG_CONFIG_HOME", xdg_config.as_os_str());
        self.set("XDG_CACHE_HOME", xdg_cache.as_os_str());
        self.set("XDG_STATE_HOME", xdg_state.as_os_str());
        self.set("PATH", OsStr::new("/usr/bin:/bin"));
        self.set("TERM", OsStr::new("xterm-256color"));
        self.remove("NO_COLOR");
        self.remove("COLUMNS");

        let osp_keys = std::env::vars_os()
            .map(|(key, _)| key)
            .filter(|key| key.to_string_lossy().starts_with("OSP_"))
            .collect::<Vec<_>>();
        for key in osp_keys {
            self.remove_os(&key);
        }
    }

    fn remember_os(&mut self, key: &OsStr) {
        if self.saved.iter().any(|(saved_key, _)| saved_key == key) {
            return;
        }
        self.saved.push((key.to_os_string(), std::env::var_os(key)));
    }

    fn set(&mut self, key: &str, value: &OsStr) {
        self.remember_os(OsStr::new(key));
        unsafe { std::env::set_var(key, value) };
    }

    fn remove(&mut self, key: &str) {
        self.remember_os(OsStr::new(key));
        unsafe { std::env::remove_var(key) };
    }

    fn remove_os(&mut self, key: &OsStr) {
        self.remember_os(key);
        unsafe { std::env::remove_var(key) };
    }
}

impl Drop for TestProcessEnv {
    fn drop(&mut self) {
        for (key, value) in self.saved.iter().rev() {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

#[test]
fn stable_top_level_surface_exposes_primary_entrypoints_and_types_unit() {
    let _run_from = |args: Vec<&str>| crate::app::run_from::<Vec<&str>, &str>(args);
    let _run_process = |args: Vec<&str>| crate::app::run_process::<Vec<&str>, &str>(args);
    let mut sink = crate::app::BufferedUiSink::default();
    let _builder = crate::app::AppBuilder::new().build();
    let _runner = crate::app::AppBuilder::new().build_with_sink(&mut sink);
    let _cli_type: Option<crate::cli::Cli> = None;
    let _row: crate::core::row::Row = Default::default();
    let _resolver: Option<crate::config::ConfigResolver> = None;
    let _completion: Option<crate::completion::CompletionEngine> = None;
    let _prompt: Option<crate::repl::ReplPrompt> = None;
    let _plugins: Option<crate::plugin::PluginManager> = None;
    let _ldap: Option<crate::api::MockLdapClient> = None;
    let _app_runtime: Option<crate::app::AppRuntime> = None;
    let _format = OutputFormat::Json;
    let _settings = crate::ui::RenderSettings::test_plain(OutputFormat::Table);
}
