#![allow(missing_docs)]

#[path = "contracts/test_env.rs"]
mod test_env;

#[path = "support/temp.rs"]
mod temp_support;

#[path = "e2e/support.rs"]
mod support;

#[path = "e2e/binary_surface.rs"]
mod binary_surface;

#[path = "e2e/cli_invocation.rs"]
mod cli_invocation;

#[path = "e2e/json_output.rs"]
mod json_output;

#[path = "e2e/plugin_processes.rs"]
mod plugin_processes;

#[path = "e2e/repl_completion.rs"]
mod repl_completion;

#[path = "e2e/repl_highlight.rs"]
mod repl_highlight;

#[path = "e2e/repl_help.rs"]
mod repl_help;

#[path = "e2e/repl_intro.rs"]
mod repl_intro;

#[path = "e2e/repl_plugins.rs"]
mod repl_plugins;

#[path = "e2e/repl_prompt.rs"]
mod repl_prompt;

#[path = "e2e/repl_smoke.rs"]
mod repl_smoke;
