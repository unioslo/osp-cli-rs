use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use reedline::{
    CommandLineSearch, History, HistoryItem, HistoryItemId, HistorySessionId, ReedlineError,
    ReedlineErrorVariants, Result as ReedlineResult, SearchDirection, SearchFilter, SearchQuery,
};
use serde::{Deserialize, Serialize};

const EXCLUDED_PREFIXES: [&str; 4] = ["exit", "quit", "help", "history list"];

pub(crate) fn should_record_command(command: &str, exclude_patterns: &[String]) -> bool {
    !is_excluded_command(command, exclude_patterns)
}

#[derive(Debug, Clone)]
pub struct HistoryConfig {
    pub path: Option<PathBuf>,
    pub max_entries: usize,
    pub enabled: bool,
    pub dedupe: bool,
    pub profile_scoped: bool,
    pub exclude_patterns: Vec<String>,
    pub profile: Option<String>,
    pub terminal: Option<String>,
    pub shell_context: Option<HistoryShellContext>,
}

impl HistoryConfig {
    pub fn new(
        path: Option<PathBuf>,
        max_entries: usize,
        enabled: bool,
        dedupe: bool,
        profile_scoped: bool,
        exclude_patterns: Vec<String>,
        profile: Option<String>,
        terminal: Option<String>,
        shell_context: Option<HistoryShellContext>,
    ) -> Self {
        Self {
            path,
            max_entries,
            enabled,
            dedupe,
            profile_scoped,
            exclude_patterns: normalize_exclude_patterns(exclude_patterns),
            profile: normalize_identifier(profile),
            terminal: normalize_identifier(terminal),
            shell_context,
        }
    }

    fn persist_enabled(&self) -> bool {
        self.enabled && self.path.is_some() && self.max_entries > 0
    }
}

#[derive(Clone, Default, Debug)]
pub struct HistoryShellContext {
    inner: Arc<RwLock<String>>,
}

impl HistoryShellContext {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(prefix.into())),
        }
    }

    pub fn set_prefix(&self, prefix: impl Into<String>) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = prefix.into();
        }
    }

    pub fn prefix(&self) -> String {
        self.inner
            .read()
            .map(|value| value.clone())
            .unwrap_or_default()
    }

    pub(crate) fn normalized_prefix(&self) -> Option<String> {
        normalize_shell_prefix(self.prefix())
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

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub timestamp_ms: Option<i64>,
    pub command: String,
}

#[derive(Clone)]
pub struct SharedHistory {
    inner: Arc<Mutex<OspHistoryStore>>,
}

impl SharedHistory {
    pub fn new(config: HistoryConfig) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(OspHistoryStore::new(config)?)),
        })
    }

    pub fn enabled(&self) -> bool {
        self.inner
            .lock()
            .map(|store| store.history_enabled())
            .unwrap_or(false)
    }

    pub fn recent_commands(&self) -> Vec<String> {
        self.inner
            .lock()
            .map(|store| store.recent_commands())
            .unwrap_or_default()
    }

    pub fn list_entries(&self) -> Vec<HistoryEntry> {
        self.inner
            .lock()
            .map(|store| store.list_entries())
            .unwrap_or_default()
    }

    pub fn prune(&self, keep: usize) -> Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        guard.prune(keep)
    }

    pub fn clear_scoped(&self) -> Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("history lock poisoned"))?;
        guard.clear_scoped()
    }

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

pub struct OspHistoryStore {
    config: HistoryConfig,
    records: Vec<HistoryRecord>,
}

impl OspHistoryStore {
    pub fn new(config: HistoryConfig) -> Result<Self> {
        let mut records = Vec::new();
        if config.persist_enabled() {
            if let Some(path) = &config.path {
                records = load_records(path);
            }
        }
        let mut store = Self { config, records };
        store.trim_to_capacity();
        Ok(store)
    }

    pub fn history_enabled(&self) -> bool {
        self.config.enabled && self.config.max_entries > 0
    }

    pub fn recent_commands(&self) -> Vec<String> {
        let shell_prefix = self.shell_prefix();
        self.records
            .iter()
            .filter_map(|record| {
                self.record_view_if_allowed(record, shell_prefix.as_deref(), false)
                    .map(|_| record.command_line.clone())
            })
            .collect()
    }

    pub fn list_entries(&self) -> Vec<HistoryEntry> {
        if !self.history_enabled() {
            return Vec::new();
        }
        let shell_prefix = self.shell_prefix();
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

    pub fn prune(&mut self, keep: usize) -> Result<usize> {
        if !self.history_enabled() {
            return Ok(0);
        }
        let shell_prefix = self.shell_prefix();
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

    pub fn clear_scoped(&mut self) -> Result<usize> {
        self.prune(0)
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
        self.config
            .shell_context
            .as_ref()
            .and_then(HistoryShellContext::normalized_prefix)
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
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        for record in &self.records {
            let payload = serde_json::to_string(record)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
            writer.write_all(payload.as_bytes())?;
            writer.write_all(b"\n")?;
        }
        writer.flush()
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
        if let Some(hostname) = &filter.hostname {
            if record.hostname.as_deref() != Some(hostname.as_str()) {
                return false;
            }
        }
        if let Some(cwd) = &filter.cwd_exact {
            if record.cwd.as_deref() != Some(cwd.as_str()) {
                return false;
            }
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
        if let Some(session) = filter.session {
            if record.session_id != Some(i64::from(session)) {
                return false;
            }
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
            session_id: item.session_id.map(|id| i64::from(id)),
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
            if let Some(last) = last_match {
                if last.command_line == expanded_full {
                    return Ok(h);
                }
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
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "history lock poisoned"))?;
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
    for line in reader.lines().flatten() {
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

pub fn expand_history(
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
    EXCLUDED_PREFIXES
        .iter()
        .any(|prefix| trimmed == *prefix || trimmed.starts_with(&format!("{prefix} ")))
        || exclude_patterns
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

    if !pattern.ends_with('*') {
        if let Some(last) = parts.iter().rev().find(|part| !part.is_empty()) {
            if !command.ends_with(last) {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matching_handles_prefix_and_infix() {
        assert!(matches_pattern("ldap user *", "ldap user bob"));
        assert!(matches_pattern("*token*", "auth token read"));
        assert!(!matches_pattern("auth", "auth token"));
        assert!(matches_pattern("auth*", "auth token"));
        assert!(matches_pattern("*user", "ldap user"));
        assert!(!matches_pattern("*user", "ldap user bob"));
    }

    #[test]
    fn excluded_commands_respect_prefixes_and_patterns() {
        assert!(is_excluded_command("help", &[]));
        assert!(is_excluded_command("history list", &[]));
        assert!(!is_excluded_command("history prune 10", &[]));
        assert!(is_excluded_command("ldap user --help", &[]));
        assert!(is_excluded_command(
            "login oistes",
            &[String::from("login *")]
        ));
    }

    #[test]
    fn list_entries_filters_shell_and_excludes() {
        let shell = HistoryShellContext::new("ldap");
        let config = HistoryConfig::new(
            None,
            10,
            true,
            false,
            false,
            vec!["user *".to_string()],
            None,
            None,
            Some(shell),
        );
        let mut store = OspHistoryStore::new(config).expect("history store should init");
        let _ = History::save(
            &mut store,
            HistoryItem::from_command_line("ldap user alice"),
        )
        .expect("save should succeed");
        let _ = History::save(
            &mut store,
            HistoryItem::from_command_line("ldap netgroup ucore"),
        )
        .expect("save should succeed");
        let _ = History::save(&mut store, HistoryItem::from_command_line("mreg host a"))
            .expect("save should succeed");

        let entries = store.list_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].command, "netgroup ucore");
        assert_eq!(entries[1].command, "mreg host a");
    }
}
