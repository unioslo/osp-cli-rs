use std::ffi::OsStr;
use std::ops::Deref;
use std::path::Path;

/// Owns a test temp directory and removes it when dropped.
///
/// Integration tests should keep this alive for as long as any child paths are
/// in use instead of returning bare `PathBuf`s rooted in `/tmp`.
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
