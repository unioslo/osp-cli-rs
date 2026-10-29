//! REPL command-format memory and its once-ever discovery hint.
//!
//! Canonical paths come from command results. Writes use ordinary config
//! commands; this module owns only previous-invocation state and hint admission.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};

use clap::Parser;
use miette::{Result, miette};

use crate::app::sink::UiSink;
use crate::app::{AppState, CommandOutput};
use crate::cli::{
    ConfigArgs, ConfigCommands, ConfigScopeArgs, ConfigSetArgs, ConfigStoreArgs, ConfigUnsetArgs,
};
use crate::config::{ConfigSource, ResolvedConfig};
use crate::core::OutputFormat;
use crate::repl::ReplResult;

const PREFIX: &str = "ui.command_formats.";

#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(
    name = "remember",
    about = "Save the previous command's explicit format as its REPL default"
)]
pub(crate) struct RememberCommand {
    /// List configured command formats.
    #[arg(long)]
    pub list: bool,
    #[arg(skip)]
    pub invocation: crate::cli::invocation::InvocationOptions,
}

#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(name = "forget", about = "Remove a command's saved format default")]
pub(crate) struct ForgetCommand {
    /// Canonical command path; defaults to the previous executed command.
    #[arg(value_name = "COMMAND")]
    pub path: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct PreviousCommand {
    pub path: Vec<String>,
    pub format: Option<OutputFormat>,
}

#[derive(Clone, Default)]
pub(crate) struct CommandFormats {
    pub previous: Option<PreviousCommand>,
    counts: BTreeMap<(String, &'static str), u8>,
    hint_done: bool,
}

impl CommandFormats {
    pub(crate) fn record(&mut self, path: &[String], format: Option<OutputFormat>) {
        self.previous = (!path.is_empty()).then(|| PreviousCommand {
            path: path.to_vec(),
            format,
        });
    }

    pub(crate) fn maybe_hint(&mut self, format: OutputFormat, sink: &mut dyn UiSink) {
        if self.hint_done || !sink.stderr_is_terminal() {
            return;
        }
        let Some(previous) = &self.previous else {
            return;
        };
        if previous.format != Some(format) {
            return;
        }
        let count = self
            .counts
            .entry((previous.path.join("."), format.as_str()))
            .or_default();
        *count = count.saturating_add(1);
        if *count != 2 {
            return;
        }
        self.hint_done = true;
        if !persist_hint_seen() {
            return;
        }
        let flag = if format == OutputFormat::Auto {
            "--format auto".to_owned()
        } else {
            format!("--{}", format.as_str())
        };
        sink.write_stderr(&format!(
            "hint: remember makes {flag} the default for {}.\n",
            previous.path.join(" ")
        ));
    }
}

// create_new arbitrates concurrent REPLs. Persist the flag before emitting any
// hint, so a crash or a second session cannot show it again.
fn persist_hint_seen() -> bool {
    let Some(root) = crate::config::default_state_root_dir() else {
        return false;
    };
    if std::fs::create_dir_all(&root).is_err() {
        return false;
    }
    let path = root.join("command-format-hint-seen");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let Ok(mut file) = options.open(&path) else {
        return false;
    };
    let persisted = file.write_all(b"seen\n").and_then(|()| file.sync_all());
    #[cfg(unix)]
    let persisted = persisted.and_then(|()| std::fs::File::open(&root)?.sync_all());
    if persisted.is_err() {
        let _ = std::fs::remove_file(path);
    }
    persisted.is_ok()
}

pub(crate) fn explicit_format(line: &str, config: &ResolvedConfig) -> Option<OutputFormat> {
    let parsed = super::input::ReplParsedLine::parse(line, config).ok()?;
    crate::cli::invocation::scan_command_tokens(&parsed.dispatch_tokens)
        .ok()?
        .invocation
        .format
}

/// Only a directly typed flag can contribute to the discovery counter.
pub(crate) fn hint_format(line: &str, config: &ResolvedConfig) -> Option<OutputFormat> {
    if crate::ui::prompt::input_is_replay()
        || !std::io::stdout().is_terminal()
        || !std::io::stderr().is_terminal()
    {
        return None;
    }
    let parsed = crate::dsl::parse::pipeline::parse_pipeline(line).ok()?;
    if !parsed.stages.is_empty() {
        return None;
    }
    let scanned =
        crate::cli::invocation::scan_command_tokens(&shell_words::split(&parsed.command).ok()?)
            .ok()?;
    let tokens = scanned
        .tokens
        .strip_prefix(&["sudo".to_owned()])
        .unwrap_or(&scanned.tokens);
    let root = tokens.first()?;
    if matches!(root.as_str(), "last" | "source" | "each")
        || super::history_store::split_bang_request(line).is_some()
        || config.get_alias_entry(root).is_some()
    {
        return None;
    }
    scanned.invocation.format.filter(|format| {
        !matches!(
            format,
            OutputFormat::Json | OutputFormat::Csv | OutputFormat::Tsv
        )
    })
}

fn previous_path(state: &AppState) -> Option<Vec<String>> {
    state
        .session
        .command_formats
        .previous
        .as_ref()
        .map(|previous| previous.path.clone())
}

pub(crate) fn remember(
    state: &mut AppState,
    command: RememberCommand,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    if command.list {
        let rows = state
            .runtime
            .config
            .resolved()
            .values()
            .keys()
            .filter_map(|key| {
                let path = key.strip_prefix(PREFIX)?.replace('.', " ");
                Some(crate::row! { "path" => path, "format" => state.runtime.config.resolved().get_string(key)? })
            })
            .collect();
        let mut output = CommandOutput::output(
            crate::core::output_model::OutputResult::from_rows(rows),
            None,
        );
        output.command_path = vec!["remember".to_owned()];
        let invocation = crate::app::resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &command.invocation,
        );
        crate::app::run_cli_command(
            state.runtime.config.resolved(),
            &invocation.ui,
            &output,
            sink,
        )?;
        return Ok(ReplResult::Continue);
    }
    let Some(previous) = state.session.command_formats.previous.clone() else {
        return Ok(ReplResult::Print(
            "No previous command to remember.\n".to_owned(),
        ));
    };
    let path = previous.path.join(" ");
    let Some(format) = previous.format else {
        return Ok(ReplResult::Print(format!(
            "{path} has no format to remember; run it with --md, --json, … first.\n"
        )));
    };
    let args = ConfigArgs {
        command: ConfigCommands::Set(ConfigSetArgs {
            key: format!("{PREFIX}{}", previous.path.join(".")),
            value: vec![format.as_str().to_owned()],
            display_user: None,
            from_file: None,
            scope: ConfigScopeArgs {
                profile: Some(state.runtime.config.resolved().active_profile().to_owned()),
                terminal: Some("repl".to_owned()),
                ..Default::default()
            },
            store: ConfigStoreArgs {
                config_store: true,
                ..Default::default()
            },
            dry_run: false,
            yes: true,
            explain: false,
        }),
    };
    edit(state, args)?;
    Ok(ReplResult::Restart {
        output: format!("{path} now defaults to {}.\n", format.as_str()),
        reload: super::ReplReloadKind::Default,
    })
}

pub(crate) fn forget(state: &mut AppState, command: ForgetCommand) -> Result<ReplResult> {
    let path = if command.path.is_empty() {
        let Some(path) = previous_path(state) else {
            return Ok(ReplResult::Print(
                "No previous command to forget.\n".to_owned(),
            ));
        };
        path
    } else {
        canonical_path(state, &command.path)?
    };
    let label = path.join(" ");
    let key = format!("{PREFIX}{}", path.join("."));
    let Some(entry) = state.runtime.config.resolved().get_value_entry(&key) else {
        return Ok(ReplResult::Print(format!(
            "{label} has no saved format to forget.\n"
        )));
    };
    if !matches!(
        entry.source,
        ConfigSource::ConfigFile | ConfigSource::Session
    ) {
        return Err(miette!(
            "{label}'s format comes from {}; remove it there.",
            entry.source
        ));
    }
    let args = ConfigArgs {
        command: ConfigCommands::Unset(ConfigUnsetArgs {
            key,
            scope: ConfigScopeArgs {
                global: entry.scope.profile.is_none(),
                profile: entry.scope.profile.clone(),
                terminal: entry.scope.terminal.clone(),
                ..Default::default()
            },
            store: ConfigStoreArgs {
                session: entry.source == ConfigSource::Session,
                config_store: entry.source == ConfigSource::ConfigFile,
                ..Default::default()
            },
            dry_run: false,
        }),
    };
    edit(state, args)?;
    Ok(ReplResult::Restart {
        output: format!("Forgot the format default for {label}.\n"),
        reload: super::ReplReloadKind::Default,
    })
}

fn edit(state: &mut AppState, args: ConfigArgs) -> Result<()> {
    crate::cli::commands::config::run_config_command(
        &state.runtime,
        &mut state.session,
        &state.runtime.ui,
        args,
    )?;
    Ok(())
}

fn canonical_path(state: &AppState, path: &[String]) -> Result<Vec<String>> {
    let catalog = crate::app::authorized_command_catalog_for(
        &state.runtime.auth,
        &state.clients,
        state.clients.native_commands().completion_catalog(),
    );
    let surface = super::surface::build_repl_surface(
        super::ReplViewContext::from_parts(&state.runtime, &state.session),
        &catalog,
    );
    // Named paths are absolute even inside a service shell.
    let mut root = crate::completion::CompletionTreeBuilder
        .build_from_specs(&surface.specs, Vec::new())
        .map_err(|error| miette!("{error}"))?
        .root;
    let mut canonical = Vec::new();
    for word in path {
        let (_, child) = root
            .resolve_child(word)
            .ok_or_else(|| miette!("unknown command path: {}", path.join(" ")))?;
        if child.value_key || child.value_leaf {
            return Err(miette!("expected a command path, without arguments"));
        }
        canonical.push(child.name.clone());
        root = child.clone();
    }
    Ok(canonical)
}
