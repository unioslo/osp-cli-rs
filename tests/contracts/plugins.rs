use crate::assert_snapshot_text;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

include!("plugins/fixtures.rs");
include!("plugins/management.rs");
include!("plugins/dispatch.rs");
include!("plugins/discovery_and_help.rs");
include!("plugins/provider_selection.rs");
