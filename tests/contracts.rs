#![allow(missing_docs)]

#[path = "contracts/cli_ldap.rs"]
mod cli_ldap;

#[path = "contracts/plugins.rs"]
mod plugins;

#[path = "contracts/profile_cli.rs"]
mod profile_cli;

#[path = "contracts/config_commands.rs"]
mod config_commands;

#[path = "contracts/doctor_commands.rs"]
mod doctor_commands;

#[path = "contracts/history_commands.rs"]
mod history_commands;

#[path = "contracts/help_commands.rs"]
mod help_commands;

#[path = "contracts/command_surfaces.rs"]
mod command_surfaces;

#[path = "contracts/repl_debug.rs"]
mod repl_debug;

#[path = "contracts/theme_commands.rs"]
mod theme_commands;

#[path = "contracts/version_commands.rs"]
mod version_commands;

#[path = "contracts/snapshot_support.rs"]
mod snapshot_support;

#[path = "contracts/test_env.rs"]
mod test_env;

#[path = "support/temp.rs"]
mod temp_support;

#[path = "support/output.rs"]
mod output_support;
