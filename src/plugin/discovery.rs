use super::conversion::to_command_spec;
use super::manager::{DiscoveredPlugin, PluginManager, PluginSource};
use crate::completion::CommandSpec;
use crate::config::{default_cache_root_dir, default_config_root_dir};
use crate::core::plugin::DescribeV1;
use anyhow::{Context, Result, anyhow};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as FmtWrite;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

const PLUGIN_EXECUTABLE_PREFIX: &str = "osp-";
const BUNDLED_MANIFEST_FILE: &str = "manifest.toml";

#[derive(Debug, Clone)]
pub(super) struct SearchRoot {
    pub(super) path: PathBuf,
    pub(super) source: PluginSource,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct BundledManifest {
    protocol_version: u32,
    #[serde(default)]
    plugin: Vec<ManifestPlugin>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ManifestPlugin {
    pub(super) id: String,
    pub(super) exe: String,
    pub(super) version: String,
    #[serde(default = "default_true")]
    pub(super) enabled_by_default: bool,
    pub(super) checksum_sha256: Option<String>,
    #[serde(default)]
    pub(super) commands: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ValidatedBundledManifest {
    pub(super) by_exe: HashMap<String, ManifestPlugin>,
}

pub(super) enum ManifestState {
    NotBundled,
    Missing,
    Invalid(String),
    Valid(ValidatedBundledManifest),
}

enum DescribeEligibility {
    Allowed,
    Skip,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct DescribeCacheFile {
    #[serde(default)]
    pub(super) entries: Vec<DescribeCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct DescribeCacheEntry {
    pub(super) path: String,
    pub(super) size: u64,
    pub(super) mtime_secs: u64,
    pub(super) mtime_nanos: u32,
    pub(super) describe: DescribeV1,
}

impl PluginManager {
    pub fn refresh(&self) {
        let mut guard = self
            .discovered_cache
            .write()
            .unwrap_or_else(|err| err.into_inner());
        *guard = None;
    }

    pub(super) fn discover(&self) -> Arc<[DiscoveredPlugin]> {
        if let Some(cached) = self
            .discovered_cache
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
        {
            return cached;
        }

        let mut guard = self
            .discovered_cache
            .write()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(cached) = guard.clone() {
            return cached;
        }
        let discovered = self.discover_uncached();
        let shared = Arc::<[DiscoveredPlugin]>::from(discovered);
        *guard = Some(shared.clone());
        shared
    }

    fn discover_uncached(&self) -> Vec<DiscoveredPlugin> {
        let roots = self.search_roots();
        let mut plugins: Vec<DiscoveredPlugin> = Vec::new();
        let mut seen_paths: HashSet<PathBuf> = HashSet::new();
        let mut describe_cache = self.load_describe_cache().unwrap_or_default();
        let mut seen_describe_paths: HashSet<String> = HashSet::new();
        let mut cache_dirty = false;

        for root in &roots {
            plugins.extend(discover_plugins_in_root(
                root,
                &mut seen_paths,
                &mut describe_cache,
                &mut seen_describe_paths,
                &mut cache_dirty,
                self.process_timeout,
            ));
        }

        cache_dirty |=
            prune_stale_describe_cache_entries(&mut describe_cache, &seen_describe_paths);
        if cache_dirty {
            let _ = self.save_describe_cache(&describe_cache);
        }

        tracing::debug!(
            discovered_plugins = plugins.len(),
            unhealthy_plugins = plugins
                .iter()
                .filter(|plugin| plugin.issue.is_some())
                .count(),
            search_roots = roots.len(),
            "completed plugin discovery"
        );

        plugins
    }

    fn search_roots(&self) -> Vec<SearchRoot> {
        let ordered = self.ordered_search_roots();
        let roots = existing_unique_search_roots(ordered);
        tracing::debug!(search_roots = roots.len(), "resolved plugin search roots");
        roots
    }

    fn ordered_search_roots(&self) -> Vec<SearchRoot> {
        let mut ordered = Vec::new();

        ordered.extend(self.explicit_dirs.iter().cloned().map(|path| SearchRoot {
            path,
            source: PluginSource::Explicit,
        }));

        if let Ok(raw) = std::env::var("OSP_PLUGIN_PATH") {
            ordered.extend(std::env::split_paths(&raw).map(|path| SearchRoot {
                path,
                source: PluginSource::Env,
            }));
        }

        ordered.extend(bundled_plugin_dirs().into_iter().map(|path| SearchRoot {
            path,
            source: PluginSource::Bundled,
        }));

        if let Some(user_dir) = self.user_plugin_dir() {
            ordered.push(SearchRoot {
                path: user_dir,
                source: PluginSource::UserConfig,
            });
        }

        if self.allow_path_discovery
            && let Ok(raw) = std::env::var("PATH")
        {
            ordered.extend(std::env::split_paths(&raw).map(|path| SearchRoot {
                path,
                source: PluginSource::Path,
            }));
        }

        tracing::trace!(
            search_roots = ordered.len(),
            "assembled ordered plugin search roots"
        );
        ordered
    }

    fn load_describe_cache(&self) -> Result<DescribeCacheFile> {
        let Some(path) = self.describe_cache_path() else {
            tracing::debug!("describe cache path unavailable; using empty cache");
            return Ok(DescribeCacheFile::default());
        };
        if !path.exists() {
            tracing::debug!(path = %path.display(), "describe cache missing; using empty cache");
            return Ok(DescribeCacheFile::default());
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read describe cache {}", path.display()))?;
        let cache = serde_json::from_str::<DescribeCacheFile>(&raw)
            .with_context(|| format!("failed to parse describe cache {}", path.display()))?;
        tracing::debug!(
            path = %path.display(),
            entries = cache.entries.len(),
            "loaded describe cache"
        );
        Ok(cache)
    }

    fn save_describe_cache(&self, cache: &DescribeCacheFile) -> Result<()> {
        let Some(path) = self.describe_cache_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create describe cache dir {}", parent.display())
            })?;
        }

        let payload = serde_json::to_string_pretty(cache)
            .context("failed to serialize describe cache to JSON")?;
        super::state::write_text_atomic(&path, &payload)
            .with_context(|| format!("failed to write describe cache {}", path.display()))
    }

    fn user_plugin_dir(&self) -> Option<PathBuf> {
        let mut path = self.config_root.clone().or_else(default_config_root_dir)?;
        path.push("plugins");
        Some(path)
    }

    fn describe_cache_path(&self) -> Option<PathBuf> {
        let mut path = self.cache_root.clone().or_else(default_cache_root_dir)?;
        path.push("describe-v1.json");
        Some(path)
    }
}

pub(super) fn bundled_manifest_path(root: &SearchRoot) -> Option<PathBuf> {
    (root.source == PluginSource::Bundled).then(|| root.path.join(BUNDLED_MANIFEST_FILE))
}

pub(super) fn load_manifest_state(root: &SearchRoot) -> ManifestState {
    let Some(path) = bundled_manifest_path(root) else {
        return ManifestState::NotBundled;
    };
    if !path.exists() {
        return ManifestState::Missing;
    }
    load_manifest_state_from_path(&path)
}

pub(super) fn load_manifest_state_from_path(path: &Path) -> ManifestState {
    match load_and_validate_manifest(path) {
        Ok(manifest) => ManifestState::Valid(manifest),
        Err(err) => ManifestState::Invalid(err.to_string()),
    }
}

pub(super) fn existing_unique_search_roots(ordered: Vec<SearchRoot>) -> Vec<SearchRoot> {
    let mut deduped_paths: HashSet<PathBuf> = HashSet::new();
    ordered
        .into_iter()
        .filter(|root| {
            if !root.path.is_dir() {
                return false;
            }
            let canonical = root
                .path
                .canonicalize()
                .unwrap_or_else(|_| root.path.clone());
            deduped_paths.insert(canonical)
        })
        .collect()
}

pub(super) fn discover_root_executables(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut executables = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| is_plugin_executable(path))
        .collect::<Vec<PathBuf>>();
    executables.sort();
    executables
}

fn discover_plugins_in_root(
    root: &SearchRoot,
    seen_paths: &mut HashSet<PathBuf>,
    describe_cache: &mut DescribeCacheFile,
    seen_describe_paths: &mut HashSet<String>,
    cache_dirty: &mut bool,
    process_timeout: Duration,
) -> Vec<DiscoveredPlugin> {
    let manifest_state = load_manifest_state(root);
    let plugins = discover_root_executables(&root.path)
        .into_iter()
        .filter(|path| seen_paths.insert(path.clone()))
        .map(|executable| {
            assemble_discovered_plugin(
                root.source,
                executable,
                &manifest_state,
                describe_cache,
                seen_describe_paths,
                cache_dirty,
                process_timeout,
            )
        })
        .collect::<Vec<_>>();

    tracing::debug!(
        root = %root.path.display(),
        source = %root.source,
        discovered_plugins = plugins.len(),
        unhealthy_plugins = plugins.iter().filter(|plugin| plugin.issue.is_some()).count(),
        "scanned plugin search root"
    );

    plugins
}

pub(super) fn assemble_discovered_plugin(
    source: PluginSource,
    executable: PathBuf,
    manifest_state: &ManifestState,
    describe_cache: &mut DescribeCacheFile,
    seen_describe_paths: &mut HashSet<String>,
    cache_dirty: &mut bool,
    process_timeout: Duration,
) -> DiscoveredPlugin {
    let file_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let manifest_entry = manifest_entry_for_executable(manifest_state, &file_name);
    let mut plugin =
        seeded_discovered_plugin(source, executable.clone(), &file_name, &manifest_entry);

    apply_manifest_discovery_issue(&mut plugin.issue, manifest_state, manifest_entry.as_ref());

    match describe_eligibility(source, manifest_state, manifest_entry.as_ref(), &executable) {
        Ok(DescribeEligibility::Allowed) => match describe_with_cache(
            &executable,
            describe_cache,
            seen_describe_paths,
            cache_dirty,
            process_timeout,
        ) {
            Ok(describe) => {
                apply_describe_metadata(&mut plugin, &describe, manifest_entry.as_ref())
            }
            Err(err) => super::state::merge_issue(&mut plugin.issue, err.to_string()),
        },
        Ok(DescribeEligibility::Skip) => {}
        Err(err) => super::state::merge_issue(&mut plugin.issue, err.to_string()),
    }

    tracing::debug!(
        plugin_id = %plugin.plugin_id,
        source = %plugin.source,
        executable = %plugin.executable.display(),
        healthy = plugin.issue.is_none(),
        issue = ?plugin.issue,
        command_count = plugin.commands.len(),
        "assembled discovered plugin"
    );

    plugin
}

fn manifest_entry_for_executable(
    manifest_state: &ManifestState,
    file_name: &str,
) -> Option<ManifestPlugin> {
    match manifest_state {
        ManifestState::Valid(manifest) => manifest.by_exe.get(file_name).cloned(),
        ManifestState::NotBundled | ManifestState::Missing | ManifestState::Invalid(_) => None,
    }
}

fn seeded_discovered_plugin(
    source: PluginSource,
    executable: PathBuf,
    file_name: &str,
    manifest_entry: &Option<ManifestPlugin>,
) -> DiscoveredPlugin {
    let fallback_id = file_name
        .strip_prefix(PLUGIN_EXECUTABLE_PREFIX)
        .unwrap_or("unknown")
        .to_string();
    let commands = manifest_entry
        .as_ref()
        .map(|entry| entry.commands.clone())
        .unwrap_or_default();

    DiscoveredPlugin {
        plugin_id: manifest_entry
            .as_ref()
            .map(|entry| entry.id.clone())
            .unwrap_or(fallback_id),
        plugin_version: manifest_entry.as_ref().map(|entry| entry.version.clone()),
        executable,
        source,
        command_specs: commands
            .iter()
            .map(|name| CommandSpec::new(name.clone()))
            .collect(),
        commands,
        issue: None,
        default_enabled: manifest_entry
            .as_ref()
            .map(|entry| entry.enabled_by_default)
            .unwrap_or(true),
    }
}

fn apply_manifest_discovery_issue(
    issue: &mut Option<String>,
    manifest_state: &ManifestState,
    manifest_entry: Option<&ManifestPlugin>,
) {
    if let Some(message) = manifest_discovery_issue(manifest_state, manifest_entry) {
        super::state::merge_issue(issue, message);
    }
}

fn describe_eligibility(
    source: PluginSource,
    manifest_state: &ManifestState,
    manifest_entry: Option<&ManifestPlugin>,
    executable: &Path,
) -> Result<DescribeEligibility> {
    if source != PluginSource::Bundled {
        return Ok(DescribeEligibility::Allowed);
    }

    match manifest_state {
        ManifestState::Missing | ManifestState::Invalid(_) => return Ok(DescribeEligibility::Skip),
        ManifestState::Valid(_) if manifest_entry.is_none() => {
            return Ok(DescribeEligibility::Skip);
        }
        ManifestState::NotBundled | ManifestState::Valid(_) => {}
    }

    if let Some(entry) = manifest_entry {
        validate_manifest_checksum(entry, executable)?;
    }

    Ok(DescribeEligibility::Allowed)
}

fn manifest_discovery_issue(
    manifest_state: &ManifestState,
    manifest_entry: Option<&ManifestPlugin>,
) -> Option<String> {
    match manifest_state {
        ManifestState::Missing => Some(format!("bundled {} not found", BUNDLED_MANIFEST_FILE)),
        ManifestState::Invalid(err) => Some(format!("bundled manifest invalid: {err}")),
        ManifestState::Valid(_) if manifest_entry.is_none() => {
            Some("plugin executable not present in bundled manifest".to_string())
        }
        ManifestState::NotBundled | ManifestState::Valid(_) => None,
    }
}

fn apply_describe_metadata(
    plugin: &mut DiscoveredPlugin,
    describe: &DescribeV1,
    manifest_entry: Option<&ManifestPlugin>,
) {
    if let Some(entry) = manifest_entry {
        plugin.default_enabled = entry.enabled_by_default;
        if let Err(err) = validate_manifest_describe(entry, describe) {
            super::state::merge_issue(&mut plugin.issue, err.to_string());
            return;
        }
    }

    plugin.plugin_id = describe.plugin_id.clone();
    plugin.plugin_version = Some(describe.plugin_version.clone());
    plugin.commands = describe
        .commands
        .iter()
        .map(|cmd| cmd.name.clone())
        .collect::<Vec<String>>();
    plugin.command_specs = describe
        .commands
        .iter()
        .map(to_command_spec)
        .collect::<Vec<CommandSpec>>();

    if let Some(issue) = min_osp_version_issue(describe) {
        super::state::merge_issue(&mut plugin.issue, issue);
    }
}

pub(super) fn min_osp_version_issue(describe: &DescribeV1) -> Option<String> {
    let min_required = describe
        .min_osp_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let current_raw = env!("CARGO_PKG_VERSION");
    let current = match Version::parse(current_raw) {
        Ok(version) => version,
        Err(err) => {
            return Some(format!(
                "osp version `{current_raw}` is invalid for plugin compatibility checks: {err}"
            ));
        }
    };
    let min = match Version::parse(min_required) {
        Ok(version) => version,
        Err(err) => {
            return Some(format!(
                "invalid min_osp_version `{min_required}` declared by plugin {}: {err}",
                describe.plugin_id
            ));
        }
    };

    if current < min {
        Some(format!(
            "plugin {} requires osp >= {min}, current version is {current}",
            describe.plugin_id
        ))
    } else {
        None
    }
}

fn load_and_validate_manifest(path: &Path) -> Result<ValidatedBundledManifest> {
    let manifest = read_bundled_manifest(path)?;
    validate_manifest_protocol(&manifest)?;
    Ok(ValidatedBundledManifest {
        by_exe: index_manifest_plugins(manifest.plugin)?,
    })
}

fn read_bundled_manifest(path: &Path) -> Result<BundledManifest> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest {}", path.display()))?;
    toml::from_str::<BundledManifest>(&raw)
        .with_context(|| format!("failed to parse manifest TOML at {}", path.display()))
}

fn validate_manifest_protocol(manifest: &BundledManifest) -> Result<()> {
    if manifest.protocol_version != 1 {
        return Err(anyhow!(
            "unsupported manifest protocol_version {}",
            manifest.protocol_version
        ));
    }
    Ok(())
}

fn index_manifest_plugins(plugins: Vec<ManifestPlugin>) -> Result<HashMap<String, ManifestPlugin>> {
    let mut by_exe: HashMap<String, ManifestPlugin> = HashMap::new();
    let mut ids = HashSet::new();

    for plugin in plugins {
        validate_manifest_plugin(&plugin)?;
        insert_manifest_plugin(&mut by_exe, &mut ids, plugin)?;
    }

    Ok(by_exe)
}

fn validate_manifest_plugin(plugin: &ManifestPlugin) -> Result<()> {
    if plugin.id.trim().is_empty() {
        return Err(anyhow!("manifest plugin id must not be empty"));
    }
    if plugin.exe.trim().is_empty() {
        return Err(anyhow!("manifest plugin exe must not be empty"));
    }
    if plugin.version.trim().is_empty() {
        return Err(anyhow!("manifest plugin version must not be empty"));
    }
    if plugin.commands.is_empty() {
        return Err(anyhow!(
            "manifest plugin {} must declare at least one command",
            plugin.id
        ));
    }
    Ok(())
}

fn insert_manifest_plugin(
    by_exe: &mut HashMap<String, ManifestPlugin>,
    ids: &mut HashSet<String>,
    plugin: ManifestPlugin,
) -> Result<()> {
    if !ids.insert(plugin.id.clone()) {
        return Err(anyhow!("duplicate plugin id in manifest: {}", plugin.id));
    }
    if by_exe.contains_key(&plugin.exe) {
        return Err(anyhow!("duplicate plugin exe in manifest: {}", plugin.exe));
    }
    by_exe.insert(plugin.exe.clone(), plugin);
    Ok(())
}

fn validate_manifest_describe(entry: &ManifestPlugin, describe: &DescribeV1) -> Result<()> {
    if entry.id != describe.plugin_id {
        return Err(anyhow!(
            "manifest id mismatch: expected {}, got {}",
            entry.id,
            describe.plugin_id
        ));
    }

    if entry.version != describe.plugin_version {
        return Err(anyhow!(
            "manifest version mismatch for {}: expected {}, got {}",
            entry.id,
            entry.version,
            describe.plugin_version
        ));
    }

    let mut expected = entry.commands.clone();
    expected.sort();
    expected.dedup();

    let mut actual = describe
        .commands
        .iter()
        .map(|cmd| cmd.name.clone())
        .collect::<Vec<String>>();
    actual.sort();
    actual.dedup();

    if expected != actual {
        return Err(anyhow!(
            "manifest commands mismatch for {}: expected {:?}, got {:?}",
            entry.id,
            expected,
            actual
        ));
    }

    Ok(())
}

fn validate_manifest_checksum(entry: &ManifestPlugin, path: &Path) -> Result<()> {
    let Some(expected_checksum) = entry.checksum_sha256.as_deref() else {
        return Ok(());
    };
    let expected_checksum = normalize_checksum(expected_checksum)?;
    let actual_checksum = file_sha256_hex(path)?;
    if expected_checksum != actual_checksum {
        return Err(anyhow!(
            "checksum mismatch for {}: expected {}, got {}",
            entry.id,
            expected_checksum,
            actual_checksum
        ));
    }
    Ok(())
}

fn describe_with_cache(
    path: &Path,
    cache: &mut DescribeCacheFile,
    seen_describe_paths: &mut HashSet<String>,
    cache_dirty: &mut bool,
    process_timeout: Duration,
) -> Result<DescribeV1> {
    let key = describe_cache_key(path);
    seen_describe_paths.insert(key.clone());
    let (size, mtime_secs, mtime_nanos) = file_fingerprint(path)?;

    if let Some(entry) = find_cached_describe(cache, &key, size, mtime_secs, mtime_nanos) {
        tracing::trace!(path = %path.display(), "describe cache hit");
        return Ok(entry.describe.clone());
    }

    tracing::trace!(path = %path.display(), "describe cache miss");

    let describe = super::dispatch::describe_plugin(path, process_timeout)?;
    upsert_cached_describe(cache, key, size, mtime_secs, mtime_nanos, describe.clone());
    *cache_dirty = true;

    Ok(describe)
}

fn describe_cache_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(super) fn find_cached_describe<'a>(
    cache: &'a DescribeCacheFile,
    key: &str,
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
) -> Option<&'a DescribeCacheEntry> {
    cache.entries.iter().find(|entry| {
        entry.path == key
            && entry.size == size
            && entry.mtime_secs == mtime_secs
            && entry.mtime_nanos == mtime_nanos
    })
}

pub(super) fn upsert_cached_describe(
    cache: &mut DescribeCacheFile,
    key: String,
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    describe: DescribeV1,
) {
    if let Some(entry) = cache.entries.iter_mut().find(|entry| entry.path == key) {
        entry.size = size;
        entry.mtime_secs = mtime_secs;
        entry.mtime_nanos = mtime_nanos;
        entry.describe = describe;
    } else {
        cache.entries.push(DescribeCacheEntry {
            path: key,
            size,
            mtime_secs,
            mtime_nanos,
            describe,
        });
    }
}

pub(super) fn prune_stale_describe_cache_entries(
    cache: &mut DescribeCacheFile,
    seen_paths: &HashSet<String>,
) -> bool {
    let before = cache.entries.len();
    cache
        .entries
        .retain(|entry| seen_paths.contains(&entry.path));
    cache.entries.len() != before
}

pub(super) fn file_fingerprint(path: &Path) -> Result<(u64, u64, u32)> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?;
    let size = metadata.len();
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read mtime for {}", path.display()))?;
    let dur = modified
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("mtime before unix epoch for {}", path.display()))?;
    Ok((size, dur.as_secs(), dur.subsec_nanos()))
}

fn bundled_plugin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(path) = std::env::var("OSP_BUNDLED_PLUGIN_DIR") {
        dirs.push(PathBuf::from(path));
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(bin_dir) = exe_path.parent()
    {
        dirs.push(bin_dir.join("plugins"));
        dirs.push(bin_dir.join("../lib/osp/plugins"));
    }

    dirs
}

pub(super) fn normalize_checksum(checksum: &str) -> Result<String> {
    let trimmed = checksum.trim().to_ascii_lowercase();
    if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "checksum must be a 64-char lowercase/uppercase hex string"
        ));
    }
    Ok(trimmed)
}

pub(super) fn file_sha256_hex(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path).with_context(|| {
        format!(
            "failed to read plugin executable for checksum: {}",
            path.display()
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];

    loop {
        let read = reader.read(&mut buffer).with_context(|| {
            format!(
                "failed to stream plugin executable for checksum: {}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();

    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(&mut out, "{b:02x}");
    }
    Ok(out)
}

fn default_true() -> bool {
    true
}

fn is_plugin_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.starts_with(PLUGIN_EXECUTABLE_PREFIX) {
        return false;
    }
    if !has_supported_plugin_extension(path) {
        return false;
    }
    if !has_valid_plugin_suffix(name) {
        return false;
    }
    is_executable_file(path)
}

#[cfg(windows)]
fn has_supported_plugin_extension(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        None => true,
        Some(ext) => ext.eq_ignore_ascii_case("exe"),
    }
}

#[cfg(not(windows))]
fn has_supported_plugin_extension(path: &Path) -> bool {
    path.extension().is_none()
}

#[cfg(windows)]
pub(super) fn has_valid_plugin_suffix(file_name: &str) -> bool {
    let base = file_name.strip_suffix(".exe").unwrap_or(file_name);
    let Some(suffix) = base.strip_prefix(PLUGIN_EXECUTABLE_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && suffix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(not(windows))]
pub(super) fn has_valid_plugin_suffix(file_name: &str) -> bool {
    let Some(suffix) = file_name.strip_prefix(PLUGIN_EXECUTABLE_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && suffix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta.permissions().mode() & 0o111 != 0,
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}
