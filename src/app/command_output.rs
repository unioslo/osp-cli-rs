use crate::config::ResolvedConfig;
use crate::core::output::OutputFormat;
use crate::core::output_model::{OutputResult, RenderRecommendation, rows_from_value};
use crate::core::plugin::{ResponseMessageLevelV1, ResponseV1};
use crate::dsl::apply_output_pipeline;
use crate::guide::GuideView;
use crate::ui::clipboard::ClipboardService;
use crate::ui::messages::{MessageBuffer, MessageLevel};
use crate::ui::{
    copy_output_to_clipboard, render_json_value, render_output, render_structured_output,
    render_structured_output_with_source_guide,
};
use miette::Result;

use crate::app::resolve_render_settings_with_hint;
use crate::app::sink::UiSink;
use crate::app::{AppSession, UiState};
use crate::cli::rows::output::{
    output_to_rows, plugin_data_to_output_result, rows_to_output_result,
};

#[derive(Debug, Clone)]
pub(crate) enum ReplCommandOutput {
    Output(Box<StructuredCommandOutput>),
    Json(serde_json::Value),
    Text(String),
}

#[derive(Debug, Clone)]
pub(crate) struct StructuredCommandOutput {
    pub(crate) source_guide: Option<GuideView>,
    pub(crate) output: OutputResult,
    pub(crate) format_hint: Option<OutputFormat>,
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
            output: Some(ReplCommandOutput::Output(Box::new(
                StructuredCommandOutput {
                    source_guide: None,
                    output,
                    format_hint,
                },
            ))),
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

    pub(crate) fn json(payload: serde_json::Value) -> Self {
        Self {
            exit_code: 0,
            messages: MessageBuffer::default(),
            output: Some(ReplCommandOutput::Json(payload)),
            stderr_text: None,
            failure_report: None,
        }
    }

    pub(crate) fn guide(guide: impl Into<GuideView>) -> Self {
        let guide = guide.into();
        let output = guide.to_output_result();
        Self::guide_with_output(guide, output, None)
    }

    pub(crate) fn guide_with_output(
        guide: impl Into<GuideView>,
        mut output: OutputResult,
        format_hint: Option<OutputFormat>,
    ) -> Self {
        let guide = guide.into();
        output
            .meta
            .render_recommendation
            .get_or_insert(RenderRecommendation::Guide);
        Self {
            exit_code: 0,
            messages: MessageBuffer::default(),
            output: Some(ReplCommandOutput::Output(Box::new(
                StructuredCommandOutput {
                    source_guide: Some(guide),
                    output,
                    format_hint,
                },
            ))),
            stderr_text: None,
            failure_report: None,
        }
    }

    pub(crate) fn from_prepared_plugin_response(prepared: PreparedPluginResponse) -> Self {
        match prepared {
            PreparedPluginResponse::Failure(failure) => Self {
                exit_code: 1,
                messages: failure.messages,
                output: None,
                stderr_text: None,
                failure_report: Some(failure.report),
            },
            PreparedPluginResponse::Output(prepared) => Self {
                exit_code: 0,
                messages: prepared.messages,
                output: Some(ReplCommandOutput::Output(Box::new(
                    StructuredCommandOutput {
                        source_guide: None,
                        output: prepared.output,
                        format_hint: prepared.format_hint,
                    },
                ))),
                stderr_text: None,
                failure_report: None,
            },
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

pub(crate) fn run_cli_command_with_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    result: CliCommandResult,
    sink: &mut dyn UiSink,
) -> Result<i32> {
    run_cli_command(&CommandRenderRuntime::new(config, ui), result, sink)
}

pub(crate) fn cli_result_from_plugin_response(
    response: ResponseV1,
    stages: &[String],
) -> Result<CliCommandResult> {
    let prepared =
        prepare_plugin_response(response, stages).map_err(|err| miette::miette!("{err:#}"))?;
    Ok(CliCommandResult::from_prepared_plugin_response(prepared))
}

pub(crate) fn emit_messages_for_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
    sink: &mut dyn UiSink,
) {
    let rendered = crate::ui::render_messages(config, &ui.render_settings, messages, verbosity);
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
    if output.meta.render_recommendation.is_none()
        && let Some(format) = format_hint
    {
        output.meta.render_recommendation = Some(RenderRecommendation::Format(format));
    }

    if !stages.is_empty() {
        tracing::trace!(stage_count = stages.len(), "applying DSL output pipeline");
        // This is the central fan-in for staged structured output. Keep all
        // callers on the canonical DSL entrypoint so semantic/output policy
        // stays consistent across CLI, REPL, and plugin flows.
        output = apply_output_pipeline(output, stages)?;
        // Once a DSL pipeline runs, producer-side format hints stop being an
        // out-of-band override. Any surviving recommendation now lives on the
        // transformed output metadata itself.
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
        "guide" => Some(OutputFormat::Guide),
        "json" => Some(OutputFormat::Json),
        "table" => Some(OutputFormat::Table),
        "md" | "markdown" => Some(OutputFormat::Markdown),
        "mreg" => Some(OutputFormat::Mreg),
        "value" => Some(OutputFormat::Value),
        _ => None,
    }
}

fn render_cli_output(
    runtime: &CommandRenderRuntime<'_>,
    output: ReplCommandOutput,
    sink: &mut dyn UiSink,
) {
    sink.write_stdout(&render_repl_output_with_runtime(runtime, &output));
    if let ReplCommandOutput::Output(structured) = output {
        maybe_copy_output_with_runtime(runtime, &structured.output, sink);
    }
}

pub(crate) fn render_repl_output_with_runtime(
    runtime: &CommandRenderRuntime<'_>,
    output: &ReplCommandOutput,
) -> String {
    match output {
        ReplCommandOutput::Output(structured) => render_structured_repl_output(
            runtime.config(),
            &runtime.ui().render_settings,
            &structured.output,
            structured.format_hint,
            structured.source_guide.as_ref(),
        ),
        ReplCommandOutput::Json(payload) => {
            let effective = resolve_render_settings_with_hint(
                &runtime.ui().render_settings,
                Some(OutputFormat::Json),
            );
            render_json_value(payload, &effective)
        }
        ReplCommandOutput::Text(text) => text.clone(),
    }
}

pub(crate) fn render_repl_command_with_runtime(
    runtime: &CommandRenderRuntime<'_>,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    result: CliCommandResult,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let CliCommandResult {
        exit_code,
        messages,
        output,
        stderr_text,
        failure_report,
        ..
    } = result;

    if exit_code != 0
        && let Some(report) = failure_report
    {
        return Err(miette::miette!("{report}"));
    }

    if !messages.is_empty() {
        emit_messages_with_runtime(runtime, &messages, runtime.ui().message_verbosity, sink);
    }

    let rendered = match output {
        Some(ReplCommandOutput::Output(structured)) => {
            render_repl_structured_command(runtime, session, line, stages, *structured, sink)?
        }
        Some(ReplCommandOutput::Text(text)) => {
            if stages.is_empty() {
                text
            } else {
                render_staged_textual_command(runtime, session, line, stages, text, sink)?
            }
        }
        Some(ReplCommandOutput::Json(payload)) => {
            if stages.is_empty() {
                render_repl_output_with_runtime(runtime, &ReplCommandOutput::Json(payload))
            } else {
                let (output, format_hint) = apply_output_stages(
                    rows_to_output_result(rows_from_value(payload)),
                    stages,
                    Some(OutputFormat::Value),
                )
                .map_err(|err| miette::miette!("{err:#}"))?;
                let render_settings =
                    resolve_render_settings_with_hint(&runtime.ui().render_settings, format_hint);
                let rendered = render_output(&output, &render_settings);
                session.record_result(line, output_to_rows(&output));
                maybe_copy_output_with_runtime(runtime, &output, sink);
                rendered
            }
        }
        None => String::new(),
    };

    if let Some(stderr_text) = stderr_text
        && !stderr_text.is_empty()
    {
        sink.write_stderr(&stderr_text);
    }

    Ok(rendered)
}

pub(crate) fn render_structured_repl_output(
    config: &ResolvedConfig,
    render_settings: &crate::ui::RenderSettings,
    output: &OutputResult,
    format_hint: Option<OutputFormat>,
    source_guide: Option<&GuideView>,
) -> String {
    let effective = resolve_render_settings_with_hint(render_settings, format_hint);
    render_structured_output_with_source_guide(
        output,
        source_guide,
        &effective,
        crate::ui::help_layout_from_config(config),
    )
}

fn render_repl_structured_command(
    runtime: &CommandRenderRuntime<'_>,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    structured: StructuredCommandOutput,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let StructuredCommandOutput {
        source_guide,
        output,
        format_hint,
    } = structured;
    let (output, format_hint) = apply_output_stages(output, stages, format_hint)
        .map_err(|err| miette::miette!("{err:#}"))?;
    let render_settings =
        resolve_render_settings_with_hint(&runtime.ui().render_settings, format_hint);
    let rendered = if stages.is_empty() {
        render_structured_repl_output(
            runtime.config(),
            &render_settings,
            &output,
            format_hint,
            source_guide.as_ref(),
        )
    } else {
        render_structured_output(runtime.config(), &render_settings, &output)
    };
    session.record_result(line, output_to_rows(&output));
    maybe_copy_output_with_runtime(runtime, &output, sink);
    Ok(rendered)
}

fn render_staged_textual_command(
    runtime: &CommandRenderRuntime<'_>,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    text: String,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let (output, format_hint) = apply_output_stages(
        text_output_to_rows(&text),
        stages,
        Some(OutputFormat::Value),
    )
    .map_err(|err| miette::miette!("{err:#}"))?;
    let render_settings =
        resolve_render_settings_with_hint(&runtime.ui().render_settings, format_hint);
    let rendered = render_output(&output, &render_settings);
    session.record_result(line, output_to_rows(&output));
    maybe_copy_output_with_runtime(runtime, &output, sink);
    Ok(rendered)
}

fn text_output_to_rows(text: &str) -> OutputResult {
    rows_to_output_result(
        text.lines()
            .filter(|line| !line.is_empty())
            .map(|line| crate::row! { "value" => line })
            .collect(),
    )
}
