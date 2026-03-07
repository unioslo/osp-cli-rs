use miette::Result;
use osp_config::ResolvedConfig;
use osp_core::output::OutputFormat;
use osp_core::output_model::OutputResult;
use osp_ui::clipboard::ClipboardService;
use osp_ui::messages::{MessageBuffer, MessageLevel, MessageRenderFormat};
use osp_ui::{copy_output_to_clipboard, render_output};

use crate::app::resolve_effective_render_settings;
use crate::state::{AppState, UiState};

pub(crate) enum ReplCommandOutput {
    Output {
        output: OutputResult,
        format_hint: Option<OutputFormat>,
    },
    Text(String),
}

pub(crate) struct CliCommandResult {
    pub(crate) exit_code: i32,
    pub(crate) output: Option<ReplCommandOutput>,
}

impl CliCommandResult {
    pub(crate) fn exit(exit_code: i32) -> Self {
        Self {
            exit_code,
            output: None,
        }
    }

    pub(crate) fn output(output: OutputResult, format_hint: Option<OutputFormat>) -> Self {
        Self {
            exit_code: 0,
            output: Some(ReplCommandOutput::Output {
                output,
                format_hint,
            }),
        }
    }

    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            exit_code: 0,
            output: Some(ReplCommandOutput::Text(text.into())),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CommandRenderRuntime<'a> {
    config: &'a ResolvedConfig,
    ui: &'a UiState,
}

impl<'a> CommandRenderRuntime<'a> {
    pub(crate) fn new(config: &'a ResolvedConfig, ui: &'a UiState) -> Self {
        Self { config, ui }
    }

    pub(crate) fn from_state(state: &'a AppState) -> Self {
        Self::new(state.config.resolved(), &state.ui)
    }

    pub(crate) fn ui(&self) -> &UiState {
        self.ui
    }

    pub(crate) fn config(&self) -> &ResolvedConfig {
        self.config
    }
}

pub(crate) fn run_cli_command(
    runtime: &CommandRenderRuntime<'_>,
    result: CliCommandResult,
) -> Result<i32> {
    if let Some(output) = result.output {
        render_cli_output(runtime, output);
    }
    Ok(result.exit_code)
}

pub(crate) fn emit_messages(state: &AppState, messages: &MessageBuffer) {
    emit_messages_with_runtime(
        &CommandRenderRuntime::from_state(state),
        messages,
        state.ui.message_verbosity,
    );
}

pub(crate) fn emit_messages_with_verbosity(
    state: &AppState,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
) {
    emit_messages_with_runtime(
        &CommandRenderRuntime::from_state(state),
        messages,
        verbosity,
    );
}

pub(crate) fn emit_messages_for_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
) {
    let resolved = ui.render_settings.resolve_render_settings();
    let message_format = config
        .get_string("ui.messages.format")
        .and_then(MessageRenderFormat::parse)
        .unwrap_or(MessageRenderFormat::Rules);
    let rendered = messages.render_grouped_with_options(osp_ui::messages::GroupedRenderOptions {
        max_level: verbosity,
        color: resolved.color,
        unicode: resolved.unicode,
        width: resolved.width,
        theme: &resolved.theme,
        format: message_format,
        style_overrides: resolved.style_overrides.clone(),
    });
    if !rendered.is_empty() {
        eprint!("{rendered}");
    }
}

pub(crate) fn emit_messages_with_runtime(
    runtime: &CommandRenderRuntime<'_>,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
) {
    emit_messages_for_ui(runtime.config(), runtime.ui(), messages, verbosity);
}

pub(crate) fn maybe_copy_output(state: &AppState, output: &OutputResult) {
    maybe_copy_output_with_runtime(&CommandRenderRuntime::from_state(state), output);
}

pub(crate) fn maybe_copy_output_with_runtime(
    runtime: &CommandRenderRuntime<'_>,
    output: &OutputResult,
) {
    if !output.meta.wants_copy {
        return;
    }
    let clipboard = ClipboardService::new();
    if let Err(err) = copy_output_to_clipboard(output, &runtime.ui().render_settings, &clipboard) {
        let mut messages = MessageBuffer::default();
        messages.warning(format!("clipboard copy failed: {err}"));
        emit_messages_with_runtime(runtime, &messages, runtime.ui().message_verbosity);
    }
}

fn render_cli_output(runtime: &CommandRenderRuntime<'_>, output: ReplCommandOutput) {
    match output {
        ReplCommandOutput::Output {
            output,
            format_hint,
        } => {
            let effective =
                resolve_effective_render_settings(&runtime.ui().render_settings, format_hint);
            print!("{}", render_output(&output, &effective));
            maybe_copy_output_with_runtime(runtime, &output);
        }
        ReplCommandOutput::Text(text) => {
            print!("{text}");
        }
    }
}
