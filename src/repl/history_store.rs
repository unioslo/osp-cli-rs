//! Persistent REPL history with profile-aware visibility and shell-prefix
//! scoping.
//!
//! Records are stored oldest-to-newest. Public listing helpers preserve that
//! order while filtering by the active profile scope, optional shell prefix,
//! and configured exclusion patterns. Shell-scoped views strip the prefix back
//! off so callers see the command as it was typed inside that shell.
//!
//! Pruning and clearing only remove records visible in the chosen scope. The
//! store also records terminal identifiers on entries for provenance, but that
//! metadata is not currently part of view scoping.
//!
//! Public API shape:
//!
//! - [`HistoryConfig::builder`] is the guided construction path and produces a
//!   normalized config snapshot on `build()`
//! - [`SharedHistory`] is the public facade; the raw `reedline` store stays
//!   crate-private

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use reedline::{
    CommandLineSearch, History, HistoryItem, HistoryItemId, HistorySessionId, ReedlineError,
    ReedlineErrorVariants, Result as ReedlineResult, SearchDirection, SearchFilter, SearchQuery,
};
use serde::{Deserialize, Serialize};

/// Configuration for REPL history persistence, visibility, and shell scoping.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[must_use]
pub struct HistoryConfig {
    /// Path to the history file, when persistence is enabled.
    pub path: Option<PathBuf>,
    /// Maximum number of retained history entries.
    pub max_entries: usize,
    /// Whether history capture is enabled.
    pub enabled: bool,
    /// Whether duplicate commands should be collapsed.
    pub dedupe: bool,
    /// Whether entries should be partitioned by active profile.
    pub profile_scoped: bool,
    /// Prefix patterns excluded from persistence.
    pub exclude_patterns: Vec<String>,
    /// Active profile identifier used for scoping.
    pub profile: Option<String>,
    /// Active terminal identifier recorded on saved entries.
    pub terminal: Option<String>,
    /// Shared shell-prefix scope used to filter and strip shell-prefixed views.
    pub shell_context: HistoryShellContext,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            path: None,
            max_entries: 1_000,
            enabled: true,
            dedupe: true,
            profile_scoped: true,
            exclude_patterns: Vec::new(),
            profile: None,
            terminal: None,
            shell_context: HistoryShellContext::default(),
        }
    }
}

impl HistoryConfig {
    /// Starts guided construction for REPL history configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    ///
    /// use osp_cli::repl::HistoryConfig;
    ///
    /// let config = HistoryConfig::builder()
    ///     .with_path(Some(PathBuf::from("/tmp/osp-history.jsonl")))
    ///     .with_max_entries(250)
    ///     .with_profile(Some(" Dev ".to_string()))
    ///     .build();
    ///
    /// assert_eq!(config.max_entries, 250);
    /// assert_eq!(config.profile.as_deref(), Some("dev"));
    /// ```
    pub fn builder() -> HistoryConfigBuilder {
        HistoryConfigBuilder::new()
    }

    /// Normalizes configured identifiers and exclusion patterns.
    pub fn normalized(mut self) -> Self {
        self.exclude_patterns =
            normalize_exclude_patterns(std::mem::take(&mut self.exclude_patterns));
        self.profile = normalize_identifier(self.profile.take());
        self.terminal = normalize_identifier(self.terminal.take());
        self
    }

    fn persist_enabled(&self) -> bool {
        self.enabled && self.path.is_some() && self.max_entries > 0
    }
}

/// Builder for [`HistoryConfig`].
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct HistoryConfigBuilder {
    config: HistoryConfig,
}

impl HistoryConfigBuilder {
    /// Starts a builder with normal REPL-history defaults.
    pub fn new() -> Self {
        Self {
            config: HistoryConfig::default(),
        }
    }

    /// Replaces the optional persistence path.
    ///
    /// If omitted, history persistence stays in-memory only.
    pub fn with_path(mut self, path: Option<PathBuf>) -> Self {
        self.config.path = path;
        self
    }

    /// Replaces the retained-entry limit.
    ///
    /// If omitted, the builder keeps the default retained-entry limit.
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.config.max_entries = max_entries;
        self
    }

    /// Enables or disables history capture.
    ///
    /// If omitted, history capture stays enabled.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.config.enabled = enabled;
        self
    }

    /// Enables or disables duplicate collapsing.
    ///
    /// If omitted, duplicate collapsing stays enabled.
    pub fn with_dedupe(mut self, dedupe: bool) -> Self {
        self.config.dedupe = dedupe;
        self
    }

    /// Enables or disables profile scoping.
    ///
    /// If omitted, history remains profile-scoped.
    pub fn with_profile_scoped(mut self, profile_scoped: bool) -> Self {
        self.config.profile_scoped = profile_scoped;
        self
    }

    /// Replaces the excluded command patterns.
    ///
    /// If omitted, no exclusion patterns are applied.
    pub fn with_exclude_patterns<I, S>(mut self, exclude_patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.exclude_patterns = exclude_patterns.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the active profile used for scoping.
    ///
    /// If omitted, history entries are not tagged with a profile identifier.
    pub fn with_profile(mut self, profile: Option<String>) -> Self {
        self.config.profile = profile;
        self
    }

    /// Replaces the active terminal label recorded on entries.
    ///
    /// If omitted, saved entries carry no terminal label.
    pub fn with_terminal(mut self, terminal: Option<String>) -> Self {
        self.config.terminal = terminal;
        self
    }

    /// Replaces the shared shell context used for scoped history views.
    ///
    /// If omitted, the builder keeps [`HistoryShellContext::default`].
    pub fn with_shell_context(mut self, shell_context: HistoryShellContext) -> Self {
        self.config.shell_context = shell_context;
        self
    }

    /// Builds a normalized history configuration.
    pub fn build(self) -> HistoryConfig {
        self.config.normalized()
    }
}

/// Shared shell-prefix state used to scope history to nested shell integrations.
#[derive(Clone, Default, Debug)]
pub struct HistoryShellContext {
    inner: Arc<RwLock<Option<String>>>,
}

impl HistoryShellContext {
    /// Creates a shell context with an initial normalized prefix.
    pub fn new(prefix: impl Into<String>) -> Self {
        let context = Self::default();
        context.set_prefix(prefix);
        context
    }

    /// Sets or replaces the normalized shell prefix.
    pub fn set_prefix(&self, prefix: impl Into<String>) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = normalize_shell_prefix(prefix.into());
        }
    }

    /// Clears the current shell prefix.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = None;
        }
    }

    /// Returns the current normalized shell prefix, if one is set.
    pub fn prefix(&self) -> Option<String> {
        self.inner.read().map(|value| value.clone()).unwrap_or(None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryRecord {
    id: i64,
    command_line: String,
    #[serde(default)]
    timestamp_ms: Option<i64>,
    #[serde(default)]
    duration_ms: Option<i64>,
    #[serde(default)]
    exit_status: Option<i64>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    session_id: Option<i64>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    terminal: Option<String>,
}

/// Visible history entry returned by listing operations after scope filtering.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Stable identifier within the visible ordered entry list.
    pub id: i64,
    /// Recorded timestamp in milliseconds since the Unix epoch, when available.
    pub timestamp_ms: Option<i64>,
    /// Command line as presented in the selected scope.
    pub command: String,
}

/// Thread-safe facade over the REPL history store.
///
/// Listing helpers return entries in oldest-to-newest order. Mutating helpers
/// such as prune and clear only touch entries visible in the chosen scope.
#[derive(Clone)]
pub struct SharedHistory {
    inner: Arc<Mutex<OspHistoryStore>>,
}

impl SharedHistory {
    /// Creates a shared history store from the provided configuration.
    ///
    /// Persisted history loading is best-effort: unreadable files and malformed
    /// lines are ignored during initialization.
    pub fn new(config: HistoryConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(OspHistoryStore::new(config))),
        }
    }

    /// Returns whether history capture is enabled for the current config.
    pub fn enabled(&self) -> bool {
        self.inner
            .lock()
            .map(|store| store.history_enabled())
            .unwrap_or(false)
    }

    /// Returns visible commands in oldest-to-newest order using the active
    /// shell scope.
    pub fn recent_commands(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|store| store.recent_commands())
            .unwrap_or_default()
    }

    /// Returns visible commands in oldest-to-newest order for the provided
    /// shell prefix.
    ///
    /// Matching profile scope and exclusion patterns still apply. When a shell
    /// prefix is provided, the returned commands have that prefix stripped.
    pub fn recent_commands_for(&self, shell_prefix: Option<&str>) -> Vec<String> {
        self.inner
            .lock()
            .map(|store| store.recent_commands_for(shell_prefix))
            .unwrap_or_default()
    }

    /// Returns visible history entries in oldest-to-newest order using the
    /// active shell scope.
    pub fn list_entries(&self) -> Vec<HistoryEntry> {
        self.inner
            .lock()
            .map(|store| store.list_entries())
            .unwrap_or_default()
    }

    /// Returns visible history entries in oldest-to-newest order for the
    /// provided shell prefix.
    pub fn list_entries_for(&self, shell_prefix: Option<&str>) -> Vec<HistoryEntry> {
        self.inner
            .lock()
            .map(|store| store.list_entries_for(shell_prefix))
            .unwrap_or_default()
    }

    /// Removes the oldest visible entries, keeping at most `keep` entries in
    /// the active scope.
    ///
    /// Returns the number of removed entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the history lock is poisoned or when persisting
    /// the updated history file fails.
    pub fn prune(&self, keep: usize) -> Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        guard.prune(keep)
    }

    /// Removes the oldest visible entries for a specific shell scope, keeping
    /// at most `keep`.
    ///
    /// Returns the number of removed entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the history lock is poisoned or when persisting
    /// the updated history file fails.
    pub fn prune_for(&self, keep: usize, shell_prefix: Option<&str>) -> Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        guard.prune_for(keep, shell_prefix)
    }

    /// Clears all entries visible in the current scope.
    ///
    /// Returns the number of removed entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the history lock is poisoned or when persisting
    /// the updated history file fails.
    pub fn clear_scoped(&self) -> Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        guard.clear_scoped()
    }

    /// Clears all entries visible to the provided shell prefix.
    ///
    /// Returns the number of removed entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the history lock is poisoned or when persisting
    /// the updated history file fails.
    pub fn clear_for(&self, shell_prefix: Option<&str>) -> Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        guard.clear_for(shell_prefix)
    }

    /// Saves one command line through the underlying `reedline::History`
    /// implementation.
    ///
    /// # Errors
    ///
    /// Returns an error when the history lock is poisoned or when the
    /// underlying history backend rejects or fails to persist the item.
    pub fn save_command_line(&self, command_line: &str) -> Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        let item = HistoryItem::from_command_line(command_line);
        History::save(&mut *guard, item).map(|_| ())?;
        Ok(())
    }
}

/// `reedline::History` implementation backed by newline-delimited JSON records.
///
/// The store keeps insertion order, applies visibility rules when presenting
/// commands back to callers, and writes the full persisted record stream back
/// out atomically after mutations.
pub(crate) struct OspHistoryStore {
    config: HistoryConfig,
    records: Vec<HistoryRecord>,
}

impl OspHistoryStore {
    /// Creates a history store and eagerly loads persisted records when
    /// persistence is enabled.
    pub fn new(config: HistoryConfig) -> Self {
        let config = config.normalized();
        let mut records = Vec::new();
        if config.persist_enabled()
            && let Some(path) = &config.path
        {
            records = load_records(path);
        }
        let mut store = Self { config, records };
        store.trim_to_capacity();
        store
    }

    /// Returns whether history operations are active for this store.
    ///
    /// This is false when history is disabled or when the configured capacity is
    /// zero.
    pub fn history_enabled(&self) -> bool {
        self.config.enabled && self.config.max_entries > 0
    }

    /// Returns visible commands in oldest-to-newest order using the active
    /// shell scope.
    pub fn recent_commands(&self) -> Vec<String> {
        self.recent_commands_for(self.shell_prefix().as_deref())
    }

    /// Returns visible commands in oldest-to-newest order for the provided
    /// shell prefix.
    ///
    /// Profile scoping and exclusion patterns still apply.
    pub fn recent_commands_for(&self, shell_prefix: Option<&str>) -> Vec<String> {
        let shell_prefix = normalize_scope_prefix(shell_prefix);
        self.records
            .iter()
            .filter_map(|record| {
                self.record_view_if_allowed(record, shell_prefix.as_deref(), true)
                    .map(|_| record.command_line.clone())
            })
            .collect()
    }

    /// Returns visible history entries in oldest-to-newest order using the
    /// active shell scope.
    pub fn list_entries(&self) -> Vec<HistoryEntry> {
        self.list_entries_for(self.shell_prefix().as_deref())
    }

    /// Returns visible history entries in oldest-to-newest order for the
    /// provided shell prefix.
    pub fn list_entries_for(&self, shell_prefix: Option<&str>) -> Vec<HistoryEntry> {
        if !self.history_enabled() {
            return Vec::new();
        }
        let shell_prefix = normalize_scope_prefix(shell_prefix);
        let mut out = Vec::new();
        let mut id = 0i64;
        for record in &self.records {
            let Some(view) = self.record_view_if_allowed(record, shell_prefix.as_deref(), true)
            else {
                continue;
            };
            id += 1;
            out.push(HistoryEntry {
                id,
                timestamp_ms: record.timestamp_ms,
                command: view,
            });
        }
        out
    }

    /// Removes the oldest visible entries, keeping at most `keep` in the active
    /// scope.
    ///
    /// Returns the number of removed entries.
    pub fn prune(&mut self, keep: usize) -> Result<usize> {
        let shell_prefix = self.shell_prefix();
        self.prune_for(keep, shell_prefix.as_deref())
    }

    /// Removes the oldest visible entries for a specific shell scope, keeping
    /// at most `keep`.
    ///
    /// Returns the number of removed entries.
    pub fn prune_for(&mut self, keep: usize, shell_prefix: Option<&str>) -> Result<usize> {
        if !self.history_enabled() {
            return Ok(0);
        }
        let shell_prefix = normalize_scope_prefix(shell_prefix);
        let mut eligible = Vec::new();
        for (idx, record) in self.records.iter().enumerate() {
            if self
                .record_view_if_allowed(record, shell_prefix.as_deref(), true)
                .is_some()
            {
                eligible.push(idx);
            }
        }

        if keep == 0 {
            return self.remove_records(&eligible);
        }

        if eligible.len() <= keep {
            return Ok(0);
        }

        let remove_count = eligible.len() - keep;
        let to_remove = eligible.into_iter().take(remove_count).collect::<Vec<_>>();
        self.remove_records(&to_remove)
    }

    /// Clears all entries visible in the current scope.
    ///
    /// This is equivalent to `prune(0)`.
    ///
    /// Returns the number of removed entries.
    pub fn clear_scoped(&mut self) -> Result<usize> {
        self.prune(0)
    }

    /// Clears all entries visible to the provided shell prefix.
    ///
    /// This is equivalent to `prune_for(0, shell_prefix)`.
    ///
    /// Returns the number of removed entries.
    pub fn clear_for(&mut self, shell_prefix: Option<&str>) -> Result<usize> {
        self.prune_for(0, shell_prefix)
    }

    fn profile_allows(&self, record: &HistoryRecord) -> bool {
        if !self.config.profile_scoped {
            return true;
        }
        match (self.config.profile.as_deref(), record.profile.as_deref()) {
            (Some(active), Some(profile)) => active == profile,
            (Some(_), None) => false,
            _ => true,
        }
    }

    fn shell_prefix(&self) -> Option<String> {
        self.config.shell_context.prefix()
    }

    fn shell_allows(&self, record: &HistoryRecord, shell_prefix: Option<&str>) -> bool {
        command_matches_shell_prefix(&record.command_line, shell_prefix)
    }

    fn view_command_line(&self, command: &str, shell_prefix: Option<&str>) -> String {
        strip_shell_prefix(command, shell_prefix)
    }

    fn record_view_if_allowed(
        &self,
        record: &HistoryRecord,
        shell_prefix: Option<&str>,
        require_shell: bool,
    ) -> Option<String> {
        if !self.profile_allows(record) {
            return None;
        }
        if require_shell && !self.shell_allows(record, shell_prefix) {
            return None;
        }
        let view_command = self.view_command_line(&record.command_line, shell_prefix);
        if self.is_command_excluded(&view_command) {
            return None;
        }
        Some(view_command)
    }

    fn is_command_excluded(&self, command: &str) -> bool {
        is_excluded_command(command, &self.config.exclude_patterns)
    }

    fn next_id(&self) -> i64 {
        self.records.len() as i64
    }

    fn trim_to_capacity(&mut self) {
        if self.config.max_entries == 0 {
            self.records.clear();
            return;
        }
        if self.records.len() > self.config.max_entries {
            let start = self.records.len() - self.config.max_entries;
            self.records = self.records.split_off(start);
        }
        for (idx, record) in self.records.iter_mut().enumerate() {
            record.id = idx as i64;
        }
    }

    fn append_record(&mut self, mut record: HistoryRecord) -> HistoryItemId {
        record.id = self.next_id();
        self.records.push(record);
        self.trim_to_capacity();
        HistoryItemId::new(self.records.len() as i64 - 1)
    }

    fn remove_records(&mut self, indices: &[usize]) -> Result<usize> {
        if indices.is_empty() {
            return Ok(0);
        }
        let mut drop_flags = vec![false; self.records.len()];
        for idx in indices {
            if *idx < drop_flags.len() {
                drop_flags[*idx] = true;
            }
        }
        let mut cursor = 0usize;
        let removed = drop_flags.iter().filter(|flag| **flag).count();
        self.records.retain(|_| {
            let keep = !drop_flags.get(cursor).copied().unwrap_or(false);
            cursor += 1;
            keep
        });
        self.trim_to_capacity();
        if let Err(err) = self.write_all() {
            return Err(err.into());
        }
        Ok(removed)
    }

    fn write_all(&self) -> std::io::Result<()> {
        if !self.config.persist_enabled() {
            return Ok(());
        }
        let Some(path) = &self.config.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut payload = Vec::new();
        for record in &self.records {
            serde_json::to_writer(&mut payload, record).map_err(std::io::Error::other)?;
            payload.push(b'\n');
        }
        crate::config::write_text_atomic(path, &payload, false)
    }

    fn should_skip_command(&self, command: &str) -> bool {
        is_excluded_command(command, &self.config.exclude_patterns)
    }

    fn command_list_for_expansion(&self) -> Vec<String> {
        self.recent_commands()
    }

    fn expand_if_needed(&self, command: &str, shell_prefix: Option<&str>) -> Option<String> {
        if !command.starts_with('!') {
            return Some(command.to_string());
        }
        let history = self.command_list_for_expansion();
        expand_history(command, &history, shell_prefix, false)
    }

    fn record_matches_filter(
        &self,
        record: &HistoryRecord,
        filter: &SearchFilter,
        shell_prefix: Option<&str>,
    ) -> bool {
        if !self.profile_allows(record) {
            return false;
        }
        if !self.shell_allows(record, shell_prefix) {
            return false;
        }
        let view_command = self.view_command_line(&record.command_line, shell_prefix);
        if self.is_command_excluded(&view_command) {
            return false;
        }
        if let Some(search) = &filter.command_line {
            let matches = match search {
                CommandLineSearch::Prefix(prefix) => view_command.starts_with(prefix),
                CommandLineSearch::Substring(substr) => view_command.contains(substr),
                CommandLineSearch::Exact(exact) => view_command == *exact,
            };
            if !matches {
                return false;
            }
        }
        if let Some(hostname) = &filter.hostname
            && record.hostname.as_deref() != Some(hostname.as_str())
        {
            return false;
        }
        if let Some(cwd) = &filter.cwd_exact
            && record.cwd.as_deref() != Some(cwd.as_str())
        {
            return false;
        }
        if let Some(prefix) = &filter.cwd_prefix {
            match record.cwd.as_deref() {
                Some(value) if value.starts_with(prefix) => {}
                _ => return false,
            }
        }
        if let Some(exit_successful) = filter.exit_successful {
            let is_success = record.exit_status == Some(0);
            if exit_successful != is_success {
                return false;
            }
        }
        if let Some(session) = filter.session
            && record.session_id != Some(i64::from(session))
        {
            return false;
        }
        true
    }

    fn record_from_item(&self, item: &HistoryItem, command_line: String) -> HistoryRecord {
        HistoryRecord {
            id: -1,
            command_line,
            timestamp_ms: item.start_timestamp.map(|ts| ts.timestamp_millis()),
            duration_ms: item.duration.map(|value| value.as_millis() as i64),
            exit_status: item.exit_status,
            cwd: item.cwd.clone(),
            hostname: item.hostname.clone(),
            session_id: item.session_id.map(i64::from),
            profile: self.config.profile.clone(),
            terminal: self.config.terminal.clone(),
        }
    }

    fn history_item_from_record(
        &self,
        record: &HistoryRecord,
        shell_prefix: Option<&str>,
    ) -> HistoryItem {
        let command_line = self.view_command_line(&record.command_line, shell_prefix);
        HistoryItem {
            id: Some(HistoryItemId::new(record.id)),
            start_timestamp: None,
            command_line,
            session_id: None,
            hostname: record.hostname.clone(),
            cwd: record.cwd.clone(),
            duration: record
                .duration_ms
                .map(|value| Duration::from_millis(value as u64)),
            exit_status: record.exit_status,
            more_info: None,
        }
    }

    fn reedline_error(message: &'static str) -> ReedlineError {
        ReedlineError(ReedlineErrorVariants::OtherHistoryError(message))
    }

    fn record_matches_query(
        &self,
        record: &HistoryRecord,
        filter: &SearchFilter,
        start_time_ms: Option<i64>,
        end_time_ms: Option<i64>,
        shell_prefix: Option<&str>,
        skip_command_line: Option<&str>,
    ) -> bool {
        if !self.record_matches_filter(record, filter, shell_prefix) {
            return false;
        }
        if let Some(skip) = skip_command_line {
            let view_command = self.view_command_line(&record.command_line, shell_prefix);
            if view_command == skip {
                return false;
            }
        }
        if let Some(start) = start_time_ms {
            match record.timestamp_ms {
                Some(value) if value >= start => {}
                _ => return false,
            }
        }
        if let Some(end) = end_time_ms {
            match record.timestamp_ms {
                Some(value) if value <= end => {}
                _ => return false,
            }
        }
        true
    }
}

impl History for OspHistoryStore {
    fn save(&mut self, h: HistoryItem) -> ReedlineResult<HistoryItem> {
        if !self.config.enabled || self.config.max_entries == 0 {
            return Ok(h);
        }

        let raw = h.command_line.trim();
        if raw.is_empty() {
            return Ok(h);
        }

        let shell_prefix = self.shell_prefix();
        let Some(expanded) = self.expand_if_needed(raw, shell_prefix.as_deref()) else {
            return Ok(h);
        };
        if self.should_skip_command(&expanded) {
            return Ok(h);
        }
        let expanded_full = apply_shell_prefix(&expanded, shell_prefix.as_deref());

        if self.config.dedupe {
            let last_match = self.records.iter().rev().find(|record| {
                self.profile_allows(record) && self.shell_allows(record, shell_prefix.as_deref())
            });
            if let Some(last) = last_match
                && last.command_line == expanded_full
            {
                return Ok(h);
            }
        }

        let mut record = self.record_from_item(&h, expanded_full);
        if record.timestamp_ms.is_none() {
            record.timestamp_ms = Some(now_ms());
        }
        let id = self.append_record(record);

        if let Err(err) = self.write_all() {
            return Err(ReedlineError(ReedlineErrorVariants::IOError(err)));
        }

        Ok(HistoryItem {
            id: Some(id),
            command_line: self.records[id.0 as usize].command_line.clone(),
            ..h
        })
    }

    fn load(&self, id: HistoryItemId) -> ReedlineResult<HistoryItem> {
        let idx = id.0 as usize;
        let shell_prefix = self.shell_prefix();
        let record = self
            .records
            .get(idx)
            .ok_or_else(|| Self::reedline_error("history item not found"))?;
        Ok(self.history_item_from_record(record, shell_prefix.as_deref()))
    }

    fn count(&self, query: SearchQuery) -> ReedlineResult<i64> {
        Ok(self.search(query)?.len() as i64)
    }

    fn search(&self, query: SearchQuery) -> ReedlineResult<Vec<HistoryItem>> {
        let (min_id, max_id) = {
            let start = query.start_id.map(|value| value.0);
            let end = query.end_id.map(|value| value.0);
            if let SearchDirection::Backward = query.direction {
                (end, start)
            } else {
                (start, end)
            }
        };
        let min_id = min_id.map(|value| value + 1).unwrap_or(0);
        let max_id = max_id
            .map(|value| value - 1)
            .unwrap_or(self.records.len().saturating_sub(1) as i64);

        if self.records.is_empty() || max_id < 0 || min_id > max_id {
            return Ok(Vec::new());
        }

        let intrinsic_limit = max_id - min_id + 1;
        let limit = query
            .limit
            .map(|value| std::cmp::min(intrinsic_limit, value) as usize)
            .unwrap_or(intrinsic_limit as usize);

        let start_time_ms = query.start_time.map(|ts| ts.timestamp_millis());
        let end_time_ms = query.end_time.map(|ts| ts.timestamp_millis());
        let shell_prefix = self.shell_prefix();

        let mut results = Vec::new();
        let iter = self
            .records
            .iter()
            .enumerate()
            .skip(min_id as usize)
            .take(intrinsic_limit as usize);
        let skip_command_line = query
            .start_id
            .and_then(|id| self.records.get(id.0 as usize))
            .map(|record| self.view_command_line(&record.command_line, shell_prefix.as_deref()));

        if let SearchDirection::Backward = query.direction {
            for (idx, record) in iter.rev() {
                if results.len() >= limit {
                    break;
                }
                if !self.record_matches_query(
                    record,
                    &query.filter,
                    start_time_ms,
                    end_time_ms,
                    shell_prefix.as_deref(),
                    skip_command_line.as_deref(),
                ) {
                    continue;
                }
                let mut item = self.history_item_from_record(record, shell_prefix.as_deref());
                item.id = Some(HistoryItemId::new(idx as i64));
                results.push(item);
            }
        } else {
            for (idx, record) in iter {
                if results.len() >= limit {
                    break;
                }
                if !self.record_matches_query(
                    record,
                    &query.filter,
                    start_time_ms,
                    end_time_ms,
                    shell_prefix.as_deref(),
                    skip_command_line.as_deref(),
                ) {
                    continue;
                }
                let mut item = self.history_item_from_record(record, shell_prefix.as_deref());
                item.id = Some(HistoryItemId::new(idx as i64));
                results.push(item);
            }
        }

        Ok(results)
    }

    fn update(
        &mut self,
        _id: HistoryItemId,
        _updater: &dyn Fn(HistoryItem) -> HistoryItem,
    ) -> ReedlineResult<()> {
        Err(ReedlineError(
            ReedlineErrorVariants::HistoryFeatureUnsupported {
                history: "OspHistoryStore",
                feature: "updating entries",
            },
        ))
    }

    fn clear(&mut self) -> ReedlineResult<()> {
        self.records.clear();
        if let Some(path) = &self.config.path {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    fn delete(&mut self, _h: HistoryItemId) -> ReedlineResult<()> {
        Err(ReedlineError(
            ReedlineErrorVariants::HistoryFeatureUnsupported {
                history: "OspHistoryStore",
                feature: "removing entries",
            },
        ))
    }

    fn sync(&mut self) -> std::io::Result<()> {
        self.write_all()
    }

    fn session(&self) -> Option<HistorySessionId> {
        None
    }
}

impl History for SharedHistory {
    fn save(&mut self, h: HistoryItem) -> ReedlineResult<HistoryItem> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| OspHistoryStore::reedline_error("history lock poisoned"))?;
        History::save(&mut *guard, h)
    }

    fn load(&self, id: HistoryItemId) -> ReedlineResult<HistoryItem> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| OspHistoryStore::reedline_error("history lock poisoned"))?;
        History::load(&*guard, id)
    }

    fn count(&self, query: SearchQuery) -> ReedlineResult<i64> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| OspHistoryStore::reedline_error("history lock poisoned"))?;
        History::count(&*guard, query)
    }

    fn search(&self, query: SearchQuery) -> ReedlineResult<Vec<HistoryItem>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| OspHistoryStore::reedline_error("history lock poisoned"))?;
        History::search(&*guard, query)
    }

    fn update(
        &mut self,
        id: HistoryItemId,
        updater: &dyn Fn(HistoryItem) -> HistoryItem,
    ) -> ReedlineResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| OspHistoryStore::reedline_error("history lock poisoned"))?;
        History::update(&mut *guard, id, updater)
    }

    fn clear(&mut self) -> ReedlineResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| OspHistoryStore::reedline_error("history lock poisoned"))?;
        History::clear(&mut *guard)
    }

    fn delete(&mut self, h: HistoryItemId) -> ReedlineResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| OspHistoryStore::reedline_error("history lock poisoned"))?;
        History::delete(&mut *guard, h)
    }

    fn sync(&mut self) -> std::io::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| std::io::Error::other("history lock poisoned"))?;
        History::sync(&mut *guard)
    }

    fn session(&self) -> Option<HistorySessionId> {
        let guard = self.inner.lock().ok()?;
        History::session(&*guard)
    }
}

fn load_records(path: &Path) -> Vec<HistoryRecord> {
    if !path.exists() {
        return Vec::new();
    }
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record: HistoryRecord = match serde_json::from_str(trimmed) {
            Ok(record) => record,
            Err(_) => continue,
        };
        if record.command_line.trim().is_empty() {
            continue;
        }
        records.push(record);
    }
    records
}

fn normalize_identifier(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

fn normalize_exclude_patterns(patterns: Vec<String>) -> Vec<String> {
    patterns
        .into_iter()
        .map(|pattern| pattern.trim().to_string())
        .filter(|pattern| !pattern.is_empty())
        .collect()
}

fn normalize_shell_prefix(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = trimmed.to_string();
    if !out.ends_with(' ') {
        out.push(' ');
    }
    Some(out)
}

fn normalize_scope_prefix(shell_prefix: Option<&str>) -> Option<String> {
    shell_prefix.and_then(|value| normalize_shell_prefix(value.to_string()))
}

fn command_matches_shell_prefix(command: &str, shell_prefix: Option<&str>) -> bool {
    match shell_prefix {
        Some(prefix) => command.starts_with(prefix),
        None => true,
    }
}

pub(crate) fn apply_shell_prefix(command: &str, shell_prefix: Option<&str>) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match shell_prefix {
        Some(prefix) => {
            let prefix_trimmed = prefix.trim_end();
            if trimmed == prefix_trimmed || trimmed.starts_with(prefix) {
                return trimmed.to_string();
            }
            let mut out = String::with_capacity(prefix.len() + trimmed.len());
            out.push_str(prefix);
            out.push_str(trimmed);
            out
        }
        _ => trimmed.to_string(),
    }
}

fn strip_shell_prefix(command: &str, shell_prefix: Option<&str>) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match shell_prefix {
        Some(prefix) => trimmed
            .strip_prefix(prefix)
            .map(|rest| rest.trim_start().to_string())
            .unwrap_or_else(|| trimmed.to_string()),
        None => trimmed.to_string(),
    }
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    now.as_millis() as i64
}

/// Expands shell-style history references against the provided command list.
///
/// Supports `!!`, `!-N`, `!N`, and prefix search forms such as `!osp`.
pub(crate) fn expand_history(
    input: &str,
    history: &[String],
    shell_prefix: Option<&str>,
    strip_prefix: bool,
) -> Option<String> {
    if !input.starts_with('!') {
        return Some(input.to_string());
    }

    let entries: Vec<(&str, String)> = history
        .iter()
        .filter(|cmd| command_matches_shell_prefix(cmd, shell_prefix))
        .map(|cmd| {
            let view = strip_shell_prefix(cmd, shell_prefix);
            (cmd.as_str(), view)
        })
        .collect();

    if entries.is_empty() {
        return None;
    }

    let select = |full: &str, view: &str, strip: bool| -> String {
        if strip {
            view.to_string()
        } else {
            full.to_string()
        }
    };

    if input == "!!" {
        let (full, view) = entries.last()?;
        return Some(select(full, view, strip_prefix));
    }

    if let Some(rest) = input.strip_prefix("!-") {
        let idx = rest.parse::<usize>().ok()?;
        if idx == 0 || idx > entries.len() {
            return None;
        }
        let (full, view) = entries.get(entries.len() - idx)?;
        return Some(select(full, view, strip_prefix));
    }

    let rest = input.strip_prefix('!')?;
    if let Ok(abs_id) = rest.parse::<usize>() {
        if abs_id == 0 || abs_id > entries.len() {
            return None;
        }
        let (full, view) = entries.get(abs_id - 1)?;
        return Some(select(full, view, strip_prefix));
    }

    for (full, view) in entries.iter().rev() {
        if view.starts_with(rest) {
            return Some(select(full, view, strip_prefix));
        }
    }

    None
}

fn is_excluded_command(command: &str, exclude_patterns: &[String]) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with('!') {
        return true;
    }
    if trimmed.contains("--help") {
        return true;
    }
    exclude_patterns
        .iter()
        .any(|pattern| matches_pattern(pattern, trimmed))
}

fn matches_pattern(pattern: &str, command: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == command;
    }

    let parts: Vec<&str> = pattern.split('*').collect();
    let mut cursor = 0usize;

    let mut first = true;
    for part in &parts {
        if part.is_empty() {
            continue;
        }
        if first && !pattern.starts_with('*') {
            if !command[cursor..].starts_with(part) {
                return false;
            }
            cursor += part.len();
        } else if let Some(pos) = command[cursor..].find(part) {
            cursor += pos + part.len();
        } else {
            return false;
        }
        first = false;
    }

    if !pattern.ends_with('*')
        && let Some(last) = parts.iter().rev().find(|part| !part.is_empty())
        && !command.ends_with(last)
    {
        return false;
    }

    true
}

#[cfg(test)]
mod tests;
