use crate::{assert_snapshot_text, assert_snapshot_text_with};
use assert_cmd::Command;
use predicates::prelude::*;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

include!("config_commands/helpers.rs");
include!("config_commands/read.rs");
include!("config_commands/explain.rs");
include!("config_commands/sources_and_profiles.rs");
include!("config_commands/mutate.rs");
