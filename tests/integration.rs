#![allow(missing_docs)]

#[path = "support/temp.rs"]
mod temp_support;

#[path = "integration/aliases.rs"]
mod aliases;

#[path = "integration/app/mod.rs"]
mod app;

#[path = "integration/completion.rs"]
mod completion;

#[path = "integration/config/mod.rs"]
mod config;

#[path = "integration/dsl/mod.rs"]
mod dsl;

#[path = "integration/guide_ui.rs"]
mod guide_ui;

#[path = "integration/plugin_manager.rs"]
mod plugin_manager;

#[path = "integration/repl/mod.rs"]
mod repl;

#[path = "integration/services/mod.rs"]
mod services;
