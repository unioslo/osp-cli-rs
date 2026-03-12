use super::conversion::{
    collect_completion_words, direct_subcommand_names, to_arg_node, to_command_spec, to_flag_node,
    to_suggestion_entry, to_value_type,
};
use super::discovery::{
    DescribeCacheEntry, DescribeCacheFile, ManifestPlugin, ManifestState, SearchRoot,
    ValidatedBundledManifest, assemble_discovered_plugin, bundled_manifest_path,
    discover_root_executables, existing_unique_search_roots, file_fingerprint, file_sha256_hex,
    find_cached_describe, has_valid_plugin_suffix, load_manifest_state,
    load_manifest_state_from_path, mark_duplicate_plugin_ids, min_osp_version_issue,
    normalize_checksum, prune_stale_describe_cache_entries, upsert_cached_describe,
};
use super::dispatch::{describe_plugin, run_provider};
use super::manager::{
    DiscoveredPlugin, PluginDispatchContext, PluginDispatchError, PluginManager, PluginSource,
};
use super::state::{PluginCommandPreferences, PluginCommandState, merge_issue};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
use crate::core::plugin::{
    DescribeArgV1, DescribeCommandV1, DescribeFlagV1, DescribeSuggestionV1, DescribeV1,
};
use std::collections::HashMap;
use std::error::Error as _;
#[cfg(unix)]
use std::sync::Mutex;
use std::time::Duration;

include!("tests/helpers.rs");
include!("tests/selection.rs");
include!("tests/discovery.rs");
include!("tests/dispatch.rs");
include!("tests/conversion.rs");
