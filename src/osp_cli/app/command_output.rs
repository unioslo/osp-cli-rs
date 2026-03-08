use crate::osp_config::ResolvedConfig;
use crate::osp_core::output::OutputFormat;
use crate::osp_core::output_model::OutputResult;
use crate::osp_core::plugin::{ResponseMessageLevelV1, ResponseV1};
use crate::osp_dsl::apply_output_pipeline;
use crate::osp_ui::clipboard::ClipboardService;
use crate::osp_ui::document::{Block, Document, JsonBlock, LineBlock, LinePart};
use crate::osp_ui::messages::{MessageBuffer, MessageLevel};
use crate::osp_ui::{copy_output_to_clipboard, render_document, render_output};
use miette::Result;

use crate::osp_cli::app::resolve_effective_render_settings;
use crate::osp_cli::rows::output::plugin_data_to_output_result;
use crate::osp_cli::state::UiState;
use crate::osp_cli::ui_presentation::effective_message_layout;
use crate::osp_cli::ui_sink::UiSink;

#[derive(Debug, Clone)]
pub(crate) enum ReplCommandOutput {
    Output {
        output: OutputResult,
        format_hint: Option<OutputFormat>,
    },
    Document(Document),
    Text(String),
}

#[derive(Debug, Clone)]
pub(crate) struct CliCommandResult {
    pub(crate) exit_code: i32,
    pub(crate) messages: MessageBuffer,
    pub(crate) output: Option<ReplCommandOutput>,
    pub(crate) stderr_text: Option<String>,
    pub(crate) failure_report: Option<String>,
}

pub(crate) struct PreparedPluginOutput {
    pub(crate) messages: MessageBuffer,
    pub(crate) output: OutputResult,
    pub(crate) format_hint: Option<OutputFormat>,
}

pub(crate) struct FailedPluginOutput {
    pub(crate) messages: MessageBuffer,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) report: String,
}

pub(crate) enum PreparedPluginResponse {
    Output(PreparedPluginOutput),
    Failure(FailedPluginOutput),
}

impl CliCommandResult {
    pub(crate) fn exit(exit_code: i32) -> Self {
        Self {
            exit_code,
            messages: MessageBuffer::default(),
            output: None,
            stderr_text: None,
            failure_report: None,
        }
    }

    pub(crate) fn output(output: OutputResult, format_hint: Option<OutputFormat>) -> Self {
        Self {
            exit_code: 0,
            messages: MessageBuffer::default(),
            output: Some(ReplCommandOutput::Output {
                output,
                format_hint,
            }),
            stderr_text: None,
            failure_report: None,
        }
    }

    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            exit_code: 0,
            messages: MessageBuffer::default(),
            output: Some(ReplCommandOutput::Text(text.into())),
            stderr_text: None,
            failure_report: None,
        }
    }

    pub(crate) fn document(document: Document) -> Self {
        Self {
            exit_code: 0,
            messages: MessageBuffer::default(),
            output: Some(ReplCommandOutput::Document(document)),
            stderr_text: None,
            failure_report: None,
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
    sink: &mut dyn UiSink,
) -> Result<i32> {
    if !result.messages.is_empty() {
        emit_messages_with_runtime(
            runtime,
            &result.messages,
            runtime.ui().message_verbosity,
            sink,
        );
    }
    if let Some(output) = result.output {
        render_cli_output(runtime, output, sink);
    }
    if let Some(stderr_text) = result.stderr_text
        && !stderr_text.is_empty()
    {
        sink.write_stderr(&stderr_text);
    }
    Ok(result.exit_code)
}

pub(crate) fn emit_messages_for_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
    sink: &mut dyn UiSink,
) {
    let resolved = ui.render_settings.resolve_render_settings();
    let message_layout = effective_message_layout(config);
    let rendered =
        messages.render_grouped_with_options(crate::osp_ui::messages::GroupedRenderOptions {
            max_level: verbosity,
            color: resolved.color,
            unicode: resolved.unicode,
            width: resolved.width,
            theme: &resolved.theme,
            layout: message_layout,
            chrome_frame: resolved.chrome_frame,
            style_overrides: resolved.style_overrides.clone(),
        });
    if !rendered.is_empty() {
        sink.write_stderr(&rendered);
    }
}

pub(crate) fn emit_messages_with_runtime(
    runtime: &CommandRenderRuntime<'_>,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
    sink: &mut dyn UiSink,
) {
    emit_messages_for_ui(runtime.config(), runtime.ui(), messages, verbosity, sink);
}

pub(crate) fn maybe_copy_output_with_runtime(
    runtime: &CommandRenderRuntime<'_>,
    output: &OutputResult,
    sink: &mut dyn UiSink,
) {
    if !output.meta.wants_copy {
        return;
    }
    let clipboard = ClipboardService::new();
    if let Err(err) = copy_output_to_clipboard(output, &runtime.ui().render_settings, &clipboard) {
        let mut messages = MessageBuffer::default();
        messages.warning(format!("clipboard copy failed: {err}"));
        emit_messages_with_runtime(runtime, &messages, runtime.ui().message_verbosity, sink);
    }
}

pub(crate) fn prepare_plugin_response(
    response: ResponseV1,
    stages: &[String],
) -> anyhow::Result<PreparedPluginResponse> {
    let mut messages = plugin_response_messages(&response);
    if !response.ok {
        let report = if let Some(error) = response.error {
            messages.error(format!("{}: {}", error.code, error.message));
            format!("{}: {}", error.code, error.message)
        } else {
            messages.error("plugin command failed");
            "plugin command failed".to_string()
        };
        return Ok(PreparedPluginResponse::Failure(FailedPluginOutput {
            messages,
            report,
        }));
    }

    let (output, format_hint) = apply_output_stages(
        plugin_data_to_output_result(response.data, Some(&response.meta)),
        stages,
        parse_output_format_hint(response.meta.format_hint.as_deref()),
    )?;

    Ok(PreparedPluginResponse::Output(PreparedPluginOutput {
        messages,
        output,
        format_hint,
    }))
}

pub(crate) fn apply_output_stages(
    mut output: OutputResult,
    stages: &[String],
    format_hint: Option<OutputFormat>,
) -> anyhow::Result<(OutputResult, Option<OutputFormat>)> {
    if !stages.is_empty() {
        tracing::trace!(stage_count = stages.len(), "applying DSL output pipeline");
        output = apply_output_pipeline(output, stages)?;
        // Once a DSL pipeline runs, the transformed output should use the
        // caller's format settings rather than the producer's original hint.
        return Ok((output, None));
    }

    Ok((output, format_hint))
}

pub(crate) fn plugin_response_messages(response: &ResponseV1) -> MessageBuffer {
    let mut out = MessageBuffer::default();
    for message in &response.messages {
        let level = match message.level {
            ResponseMessageLevelV1::Error => MessageLevel::Error,
            ResponseMessageLevelV1::Warning => MessageLevel::Warning,
            ResponseMessageLevelV1::Success => MessageLevel::Success,
            ResponseMessageLevelV1::Info => MessageLevel::Info,
            ResponseMessageLevelV1::Trace => MessageLevel::Trace,
        };
        out.push(level, message.text.clone());
    }
    out
}

pub(crate) fn parse_output_format_hint(value: Option<&str>) -> Option<OutputFormat> {
    let normalized = value?.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "auto" => Some(OutputFormat::Auto),
        "json" => Some(OutputFormat::Json),
        "table" => Some(OutputFormat::Table),
        "md" | "markdown" => Some(OutputFormat::Markdown),
        "mreg" => Some(OutputFormat::Mreg),
        "value" => Some(OutputFormat::Value),
        _ => None,
    }
}

pub(crate) fn document_from_text(text: &str) -> Document {
    let normalized = text.trim_end_matches('\n');
    if normalized.is_empty() {
        return Document::default();
    }

    Document {
        blocks: normalized
            .split('\n')
            .map(|line| {
                Block::Line(LineBlock {
                    parts: vec![LinePart {
                        text: line.to_string(),
                        token: None,
                    }],
                })
            })
            .collect(),
    }
}

pub(crate) fn document_from_json(payload: serde_json::Value) -> Document {
    Document {
        blocks: vec![Block::Json(JsonBlock { payload })],
    }
}

fn render_cli_output(
    runtime: &CommandRenderRuntime<'_>,
    output: ReplCommandOutput,
    sink: &mut dyn UiSink,
) {
    match output {
        ReplCommandOutput::Output {
            output,
            format_hint,
        } => {
            let effective =
                resolve_effective_render_settings(&runtime.ui().render_settings, format_hint);
            sink.write_stdout(&render_output(&output, &effective));
            maybe_copy_output_with_runtime(runtime, &output, sink);
        }
        ReplCommandOutput::Document(document) => {
            sink.write_stdout(&render_document(&document, &runtime.ui().render_settings));
        }
        ReplCommandOutput::Text(text) => {
            sink.write_stdout(&text);
        }
    }
}
