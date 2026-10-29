//! One command output contract shared by CLI, REPL, progress and replay.

use crate::config::ResolvedConfig;
use crate::core::document::Document;
use crate::core::output::OutputFormat;
use crate::core::output_model::{
    DisplayRule, OutputDocument, OutputItems, OutputResult, RenderRecommendation,
};
use crate::core::plugin::{ResponseErrorV1, ResponseMessageV1, ResponseMetaV1, ResponseV1};
use crate::dsl::apply_output_pipeline;
use crate::ui::clipboard::ClipboardService;
use crate::ui::messages::{MessageBuffer, MessageLevel};
use crate::ui::{RenderRequest, RenderSettings, RenderTarget};
use miette::{Result, WrapErr, miette};
use serde_json::Value;
use terminal_size::{Height, terminal_size};

use crate::app::sink::UiSink;
use crate::app::{AppSession, UiState};
use crate::cli::rows::output::{plugin_data_to_output_result, rows_to_output_result};

/// The mutually exclusive data returned by a command.
#[derive(Debug, Clone)]
pub(crate) enum CommandBody {
    None,
    Output(Box<OutputResult>),
    Text(String),
    Json(Value),
}

impl CommandBody {
    fn into_output(self) -> Option<OutputResult> {
        match self {
            Self::None => None,
            Self::Output(output) => Some(*output),
            Self::Text(text) => Some(text_output_to_rows(&text)),
            Self::Json(value) => Some(OutputResult::from_rows(
                crate::core::output_model::rows_from_value(value),
            )),
        }
    }

    fn json_value(&self) -> Value {
        match self {
            Self::None => Value::Null,
            Self::Output(output) => crate::ui::json_value(output),
            Self::Text(text) => serde_json::from_str(text).unwrap_or(Value::Null),
            Self::Json(value) => value.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct CommandOutput {
    /// Canonical executed path, retained independently of arguments and aliases.
    pub(crate) command_path: Vec<String>,
    pub(crate) diagnostics: (String, String),
    pub(crate) retained_evidence: String,
    pub(crate) exit_code: i32,
    /// External process status must not replace captured output with an envelope.
    pub(crate) preserve_process_output: bool,
    pub(crate) messages: MessageBuffer,
    pub(crate) body: CommandBody,
    pub(crate) source: Option<Value>,
    /// Retained with both documents so plain `last` repeats this selection.
    pub(crate) use_source: bool,
    pub(crate) format_hint: Option<OutputFormat>,
    pub(crate) progress_replace: bool,
    /// Native display supplements, suppressed when the source is selected.
    pub(crate) display_details: String,
    pub(crate) intro_decoration: Option<Vec<String>>,
    /// Passthrough stderr emitted alongside the body, without UI formatting.
    pub(crate) stderr_text: String,
    pub(crate) failure_report: Option<miette::Report>,
}

impl CommandOutput {
    pub(crate) fn exit(exit_code: i32) -> Self {
        Self {
            command_path: Vec::new(),
            diagnostics: Default::default(),
            retained_evidence: String::new(),
            exit_code,
            preserve_process_output: false,
            messages: MessageBuffer::default(),
            body: CommandBody::None,
            source: None,
            use_source: false,
            format_hint: None,
            progress_replace: false,
            display_details: String::new(),
            intro_decoration: None,
            stderr_text: String::new(),
            failure_report: None,
        }
    }

    pub(crate) fn output(output: OutputResult, format_hint: Option<OutputFormat>) -> Self {
        Self {
            body: CommandBody::Output(Box::new(output)),
            format_hint,
            ..Self::exit(0)
        }
    }

    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            body: CommandBody::Text(text.into()),
            format_hint: Some(OutputFormat::Value),
            ..Self::exit(0)
        }
    }

    pub(crate) fn json(payload: Value) -> Self {
        Self {
            body: CommandBody::Json(payload),
            format_hint: Some(OutputFormat::Value),
            ..Self::exit(0)
        }
    }

    pub(crate) fn document(document: Document) -> Self {
        Self::output(document.to_output_result(), None)
    }

    pub(crate) fn from_data(
        data: Value,
        meta: &ResponseMetaV1,
        messages: &[ResponseMessageV1],
        stages: &[String],
    ) -> Self {
        let mut result = Self::output(
            plugin_data_to_output_result(data, Some(meta)),
            meta.format_hint.as_deref().and_then(OutputFormat::parse),
        );
        for message in messages {
            result.messages.push(message.level, message.text.clone());
        }
        if let Some(rest) = meta.partial_rows.as_deref().filter(|_| !stages.is_empty()) {
            result.messages.warning(format!(
                "Pipe stages ran over this page only, so counts and groups are partial; {rest}"
            ));
        }
        if let CommandBody::Output(output) = &result.body {
            let data = crate::ui::json_value(output);
            let data = meta
                .row_path
                .as_deref()
                .and_then(|path| data.get(path))
                .unwrap_or(&data);
            let rows = data
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| std::slice::from_ref(data));
            for (index, row) in rows.iter().enumerate() {
                for column in &meta.detail_columns {
                    if let Some(value) = column
                        .split('.')
                        .try_fold(row, |value, part| value.get(part))
                    {
                        let path = if rows.len() > 1 {
                            format!("{index}.{column}")
                        } else {
                            column.clone()
                        };
                        render_detail_fields(&path, value, &mut result.display_details);
                    }
                }
            }
        }
        result.progress_replace = meta.progress_replace;
        result
    }

    /// Retains successful command data for replay without diagnostic ownership.
    fn saved(&self) -> Self {
        let mut body = self.body.clone();
        if let CommandBody::Output(output) = &mut body {
            output
                .meta
                .display_rules
                .retain(|rule| !matches!(rule, DisplayRule::Hyperlink { .. }));
        }
        Self {
            body,
            command_path: self.command_path.clone(),
            source: self.source.clone(),
            use_source: self.use_source,
            format_hint: self.format_hint,
            progress_replace: false,
            ..Self::exit(0)
        }
    }

    /// Selects the document before rendering, recording or running any pipeline.
    fn select_source(&mut self) {
        if self.use_source
            && let Some(source) = self.source.take()
        {
            let context = match &self.body {
                CommandBody::Output(output) => output.meta.context.clone(),
                _ => Default::default(),
            };
            self.body = CommandBody::Output(Box::new(plugin_data_to_output_result(
                source,
                Some(&ResponseMetaV1 {
                    preserve_json_document: true,
                    context,
                    ..ResponseMetaV1::default()
                }),
            )));
            self.format_hint = None;
            self.display_details.clear();
        }
    }

    pub(crate) fn apply_stages(&mut self, stages: &[String]) -> Result<()> {
        self.select_source();
        if stages.is_empty() {
            return Ok(());
        }
        let Some(output) = std::mem::replace(&mut self.body, CommandBody::None).into_output()
        else {
            return Ok(());
        };
        let (output, format_hint) = apply_output_stages(output, stages, self.format_hint)?;
        self.body = CommandBody::Output(Box::new(output));
        self.format_hint = format_hint;
        self.progress_replace = false;
        Ok(())
    }

    pub(crate) fn output_format(&self, settings: &RenderSettings) -> OutputFormat {
        match &self.body {
            CommandBody::Json(_) => OutputFormat::Json,
            CommandBody::Output(output) => {
                settings
                    .resolve_output_format(&output_with_format_hint(output, self.format_hint))
                    .0
            }
            _ => self
                .format_hint
                .filter(|_| settings.format == OutputFormat::Auto)
                .unwrap_or(settings.format),
        }
    }

    pub(super) fn render(&self, config: &ResolvedConfig, settings: &RenderSettings) -> String {
        self.render_for_target(config, settings, RenderTarget::screen(settings))
    }

    pub(crate) fn render_for_target(
        &self,
        config: &ResolvedConfig,
        settings: &RenderSettings,
        target: RenderTarget,
    ) -> String {
        let mut effective = settings.clone();
        let output = match &self.body {
            CommandBody::None => return String::new(),
            CommandBody::Text(text) => {
                return if matches!(
                    target,
                    RenderTarget::Editor | RenderTarget::Clipboard | RenderTarget::Pipe
                ) {
                    super::sink::plain_output(text)
                } else {
                    text.clone()
                };
            }
            CommandBody::Output(output) => output_with_format_hint(output, self.format_hint),
            CommandBody::Json(payload) => {
                effective.format = OutputFormat::Json;
                effective.format_explicit = true;
                std::borrow::Cow::Owned(
                    OutputResult::from_rows(Vec::new())
                        .with_document(OutputDocument::Json(payload.clone())),
                )
            }
        };
        crate::ui::render(
            &output,
            &RenderRequest {
                target,
                leading_decoration: self.intro_decoration.as_deref(),
                layout: Some(crate::ui::help_layout_from_config(config)),
                ..RenderRequest::new(&effective)
            },
        )
    }
}

fn render_detail_fields(path: &str, value: &Value, out: &mut String) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if !crate::config::is_sensitive_key(key) {
                    render_detail_fields(&format!("{path}.{key}"), value, out);
                }
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                render_detail_fields(&format!("{path}.{index}"), value, out);
            }
        }
        Value::Null => {}
        value => out.push_str(&format!(
            "detail: {}={}\n",
            crate::diagnostics::safe_text(path),
            crate::diagnostics::safe_text(
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
                    .as_str()
            )
        )),
    }
}

#[derive(Debug)]
pub(crate) struct CommandBackendDetails {
    pub(super) error: ResponseErrorV1,
    source: miette::Report,
    pub(super) backtrace: std::backtrace::Backtrace,
}

impl std::fmt::Display for CommandBackendDetails {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.source, f)
    }
}

impl std::error::Error for CommandBackendDetails {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl miette::Diagnostic for CommandBackendDetails {}

pub(crate) fn json_failure_error(
    report: Option<&miette::Report>,
    code: &str,
    message: String,
    settings: &crate::ui::RenderSettings,
) -> Value {
    if let Some(backend) =
        report.and_then(super::host::find_error_in_chain::<CommandBackendDetails>)
    {
        serde_json::json!(backend.error)
    } else {
        let error = report
            .map(|report| {
                super::host::report_error_details(report, MessageLevel::Success, settings)
            })
            .unwrap_or_else(|| crate::core::CommandError::new(message));
        let mut details = serde_json::Map::new();
        if let Some(action) = &error.action {
            details.insert("action".into(), serde_json::json!(action));
        }
        if let Some(id) = &error.request_id {
            details.insert("request_id".into(), serde_json::json!(id));
        }
        let details = if details.is_empty() {
            Value::Null
        } else {
            Value::Object(details)
        };
        serde_json::json!({"code": code, "message": error.message, "details": details})
    }
}

pub(crate) fn render_json_failure(
    error: Value,
    exit_code: i32,
    messages: Vec<Value>,
    data: Value,
) -> String {
    let payload = serde_json::json!({
        "ok": false,
        "error": error,
        "exit_code": exit_code,
        "messages": messages,
        "data": data,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
    )
}

pub(crate) fn run_cli_command(
    config: &ResolvedConfig,
    ui: &UiState,
    result: &CommandOutput,
    sink: &mut dyn UiSink,
) -> Result<i32> {
    let mut ui = ui.clone();
    super::host::apply_command_format(config, &mut ui, &result.command_path);
    let ui = &ui;

    super::sink::write_diagnostics(sink, &result.diagnostics.0, &ui.render_settings);
    if ui.render_settings.format == OutputFormat::Json
        && result.exit_code != 0
        && !result.preserve_process_output
    {
        let data = result.body.json_value();
        let messages = result.messages.entries().iter()
            .map(|entry| serde_json::json!({"level": entry.level.as_str(), "text": entry.text.as_str()}))
            .collect();
        let message = result
            .messages
            .entries()
            .iter()
            .find(|entry| entry.level == MessageLevel::Error)
            .map(|entry| entry.text.clone())
            .or_else(|| {
                result.failure_report.as_ref().map(|report| {
                    super::host::render_report_message(
                        report,
                        MessageLevel::Success,
                        &ui.render_settings,
                    )
                })
            })
            .unwrap_or_else(|| "command failed".to_string());
        let code = if result.exit_code == super::host::EXIT_CODE_USAGE {
            "usage_error"
        } else if result.messages.entries().is_empty()
            && result.failure_report.as_ref().is_some_and(|report| {
                super::host::find_error_in_chain::<CommandBackendDetails>(report).is_none()
            })
        {
            "command_error"
        } else {
            "command_failed"
        };
        let error = json_failure_error(
            result.failure_report.as_ref(),
            code,
            message,
            &ui.render_settings,
        );
        sink.write_stdout(&render_json_failure(
            error,
            result.exit_code,
            messages,
            data,
        ));
        if let Some(report) = &result.failure_report
            && ui.debug_level >= 2
            && let Some(backtrace) = super::host::render_backtrace(report, &ui.render_settings)
        {
            sink.write_stderr(&format!("{backtrace}\n"));
        }
        sink.write_stderr(&result.display_details);
        if !result.stderr_text.is_empty() {
            sink.write_stderr(&result.stderr_text);
        }
        super::sink::write_diagnostics(sink, &result.diagnostics.1, &ui.render_settings);
        return Ok(result.exit_code);
    }
    let report_message = result.failure_report.as_ref().map(|report| {
        super::host::report_error_details(report, ui.verbosity, &ui.render_settings).render()
    });
    let mut messages = MessageBuffer::default();
    for entry in result.messages.entries() {
        if !(entry.level == MessageLevel::Error && Some(&entry.text) == report_message.as_ref()) {
            messages.push_message(entry.clone());
        }
    }
    emit_messages_for_ui(config, ui, &messages, ui.verbosity, sink);
    if let Some(report) = &result.failure_report {
        let choices = super::host::report_hint_commands(report, ui.verbosity, None)
            .as_ref()
            .map_or(0, Vec::len);
        let hint_action = if matches!(
            ui.render_settings.runtime.command_scope,
            crate::core::CommandScope::OneShot
        ) {
            None
        } else {
            crate::repl::lifecycle::hint_action(config, ui.render_settings.format, choices)
        };
        super::sink::write_diagnostics(
            sink,
            &super::render_process_error(
                report,
                &ui.render_settings,
                ui.verbosity,
                hint_action.as_deref(),
            ),
            &ui.render_settings,
        );
    }
    let rendered = result.render(config, &ui.render_settings);
    if (result.failure_report.is_some() && !result.preserve_process_output)
        || result
            .messages
            .entries()
            .iter()
            .any(|entry| entry.level == MessageLevel::Error)
    {
        sink.write_stderr(&rendered);
    } else {
        let rendered = apply_output_sinks(config, ui, result, rendered, sink)?;
        if !rendered.is_empty()
            && let Some(pager) = pager_for_output(config, ui, result, &rendered)
        {
            sink.write_stdout_paged(
                &result.render_for_target(config, &ui.render_settings, RenderTarget::Pager),
                &pager,
            );
        } else {
            sink.write_stdout(&rendered);
        }
    }
    if ui.debug_level >= 2
        && let Some(report) = &result.failure_report
        && let Some(backtrace) = super::host::render_backtrace(report, &ui.render_settings)
    {
        sink.write_stderr(&format!("{backtrace}\n"));
    }
    sink.write_stderr(&result.display_details);
    if !result.stderr_text.is_empty() {
        sink.write_stderr(&result.stderr_text);
    }
    super::sink::write_diagnostics(sink, &result.diagnostics.1, &ui.render_settings);
    Ok(result.exit_code)
}

/// Renders transient progress on stderr at normal verbosity and above.
/// Quiet output suppresses replaceable progress while retaining durable notices.
pub(crate) fn run_progress_command_with_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    result: CommandOutput,
    sink: &mut dyn UiSink,
    stream: bool,
) -> Result<()> {
    let CommandBody::Output(output) = &result.body else {
        return Ok(());
    };
    let json_output = result.output_format(&ui.render_settings) == OutputFormat::Json;
    if stream && json_output {
        emit_messages_for_ui(config, ui, &result.messages, ui.verbosity, sink);
        if progress_output_is_empty(output) {
            return Ok(());
        }
        let data = crate::ui::json_value(output);
        let events = data
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_else(|| std::slice::from_ref(&data));
        for event in events {
            let line = serde_json::to_string(event).map_err(|err| miette!("{err}"))?;
            sink.write_stderr(&format!("{line}\n"));
        }
        return Ok(());
    }
    if !stream
        && result.progress_replace
        && (!sink.stderr_is_terminal()
            || matches!(
                result.output_format(&ui.render_settings),
                OutputFormat::Json | OutputFormat::Csv | OutputFormat::Tsv
            ))
    {
        emit_messages_for_ui(config, ui, &result.messages, ui.verbosity, sink);
        return Ok(());
    }
    let show_body = !result.progress_replace || ui.verbosity >= MessageLevel::Success;
    let framed = show_body
        && result.progress_replace
        && !json_output
        && sink.stderr_is_terminal()
        && !progress_output_is_empty(output);
    if !result.messages.is_empty() && !framed {
        emit_messages_for_ui(config, ui, &result.messages, ui.verbosity, sink);
    }
    if !show_body || progress_output_is_empty(output) {
        return Ok(());
    }
    let mut settings = ui.render_settings.clone();
    if let Some(width) = sink.stderr_width() {
        settings.width = Some(width);
        settings.runtime.width = Some(width);
    }
    let mut rendered = if !json_output && sink.stderr_is_terminal() {
        result.render(config, &settings)
    } else {
        plain_progress_settings(&mut settings);
        crate::ui::render(
            output,
            &RenderRequest {
                layout: Some(crate::ui::help_layout_from_config(config)),
                ..RenderRequest::new(&settings)
            },
        )
    };
    if framed && !result.messages.is_empty() {
        rendered.insert_str(
            0,
            &crate::ui::render_messages(&result.messages, &{
                let mut request = crate::ui::MessageRequest::new(&ui.render_settings, ui.verbosity);
                request.layout = crate::ui::messages::message_layout_from_config(config);
                request
            }),
        );
    }
    if !rendered.is_empty() {
        sink.write_progress(&rendered, result.progress_replace && !json_output);
    }
    Ok(())
}

fn plain_progress_settings(settings: &mut RenderSettings) {
    settings.format = OutputFormat::Value;
    settings.format_explicit = true;
    settings.mode = crate::core::output::RenderMode::Plain;
    settings.color = crate::core::output::ColorMode::Never;
    settings.unicode = crate::core::output::UnicodeMode::Never;
}

fn progress_output_is_empty(output: &OutputResult) -> bool {
    match &output.items {
        OutputItems::Rows(rows) => {
            rows.is_empty()
                || rows.iter().all(|row| {
                    row.is_empty()
                        || (row.len() == 1 && row.get("value").is_some_and(Value::is_null))
                })
        }
        OutputItems::Groups(groups) => groups.is_empty(),
    }
}

pub(crate) fn apply_stages_to_cli_result(
    mut result: CommandOutput,
    stages: &[String],
) -> Result<CommandOutput> {
    let context = match &result.body {
        CommandBody::Text(_) => "failed to apply DSL stages to textual command output",
        CommandBody::Json(_) => "failed to apply DSL stages to JSON command output",
        CommandBody::None | CommandBody::Output(_) => {
            "failed to apply DSL stages to builtin command output"
        }
    };
    result.apply_stages(stages).wrap_err(context)?;
    Ok(result)
}

pub(crate) fn emit_messages_for_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
    sink: &mut dyn UiSink,
) {
    if ui.render_settings.format == OutputFormat::Json {
        let entries = messages
            .entries()
            .iter()
            .filter(|entry| entry.level <= verbosity)
            .map(|entry| {
                serde_json::json!({
                    "level": entry.level.as_str(),
                    "text": entry.text.as_str(),
                })
            })
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            let rendered = serde_json::to_string_pretty(&serde_json::json!({
                "messages": entries,
            }))
            .unwrap_or_else(|_| r#"{"messages":[]}"#.to_string());
            sink.write_stderr(&format!("{rendered}\n"));
        }
        return;
    }

    let _ = config;
    for entry in messages
        .entries()
        .iter()
        .filter(|entry| entry.level <= verbosity)
    {
        let prefix = match entry.level {
            MessageLevel::Error => "error",
            MessageLevel::Warning => "warning",
            MessageLevel::Success => "success",
            MessageLevel::Info => "info",
            MessageLevel::Trace => "detail",
            MessageLevel::Evidence => "evidence",
        };
        let (prefix, text) = if entry.level == MessageLevel::Info {
            entry
                .text
                .strip_prefix("hint: ")
                .map(|hint| ("hint", hint))
                .unwrap_or((prefix, entry.text.as_str()))
        } else {
            (prefix, entry.text.as_str())
        };
        super::sink::write_diagnostics(
            sink,
            &format!("{prefix}: {}\n", crate::ui::sanitize_human_text(text)),
            &ui.render_settings,
        );
    }
}

fn maybe_copy_output(
    config: &ResolvedConfig,
    ui: &UiState,
    result: &CommandOutput,
    edited: Option<&str>,
    sink: &mut dyn UiSink,
) {
    let CommandBody::Output(output) = &result.body else {
        return;
    };
    if !output.meta.wants_copy {
        return;
    }
    let mut messages = MessageBuffer::default();
    let clipboard = ClipboardService::new();
    let copied = match edited {
        Some(text) => clipboard.copy_text(text),
        None => clipboard.copy_text(&result.render_for_target(
            config,
            &ui.render_settings,
            RenderTarget::Clipboard,
        )),
    };
    match copied {
        Ok(()) if result.output_format(&ui.render_settings) == OutputFormat::Json => return,
        Ok(()) if edited.is_some() => messages.success("Copied edited output."),
        Ok(()) => {
            let (count, noun) = match &output.items {
                OutputItems::Rows(rows) => (rows.len(), "row"),
                OutputItems::Groups(groups) => (groups.len(), "group"),
            };
            messages.success(format!(
                "Copied {count} {noun}{}.",
                if count == 1 { "" } else { "s" }
            ));
        }
        Err(err) => messages.warning(format!("clipboard copy failed: {err}")),
    }
    emit_messages_for_ui(config, ui, &messages, ui.verbosity, sink);
}

pub(crate) fn cli_result_from_plugin_response(
    response: ResponseV1,
    stages: &[String],
) -> Result<CommandOutput> {
    let messages = response
        .messages
        .into_iter()
        .filter(|message| response.ok || message.level != MessageLevel::Success)
        .collect::<Vec<_>>();
    let mut result = CommandOutput::from_data(response.data, &response.meta, &messages, stages);
    result.source = response.source;
    if !response.ok {
        let report = if let Some(error) = response.error {
            let action = error.details.get("action").and_then(Value::as_str);
            let request_id = error.details.get("request_id").and_then(Value::as_str);
            let report = miette::Report::new(crate::core::CommandError {
                message: error.message.clone(),
                action: action.map(str::to_owned),
                request_id: request_id.map(crate::diagnostics::safe_text),
            });
            miette::Report::new(CommandBackendDetails {
                error,
                source: report,
                backtrace: std::backtrace::Backtrace::force_capture(),
            })
        } else {
            result.messages.error("command failed");
            miette!("command failed")
        };
        result.exit_code = 1;
        result.failure_report = Some(report);
    }
    Ok(result)
}

pub(crate) fn output_with_format_hint(
    output: &OutputResult,
    format_hint: Option<OutputFormat>,
) -> std::borrow::Cow<'_, OutputResult> {
    let mut output = std::borrow::Cow::Borrowed(output);
    if output.meta.render_recommendation.is_none()
        && let Some(format) = format_hint
    {
        output.to_mut().meta.render_recommendation = Some(RenderRecommendation::Format(format));
    }
    output
}

pub(crate) fn apply_output_stages(
    mut output: OutputResult,
    stages: &[String],
    format_hint: Option<OutputFormat>,
) -> Result<(OutputResult, Option<OutputFormat>)> {
    output = output_with_format_hint(&output, format_hint).into_owned();

    if !stages.is_empty() {
        output
            .meta
            .display_rules
            .retain(|rule| !matches!(rule, DisplayRule::Hyperlink { .. }));
        tracing::trace!(stage_count = stages.len(), "applying DSL output pipeline");
        // This is the central fan-in for staged structured output. Keep all
        // callers on the canonical DSL entrypoint so semantic/output policy
        // stays consistent across CLI, REPL, and plugin flows.
        let context = output.meta.context.clone();
        output = apply_output_pipeline(output, stages).map_err(|err| {
            crate::app::report_anyhow_with_context(
                err,
                crate::app::host::ErrorContext::Operation("failed to apply DSL output pipeline"),
            )
        })?;
        output.meta.context = context;
        output.meta.display_limit = None;
        // Once a DSL pipeline runs, producer-side format hints stop being an
        // out-of-band override. Any surviving recommendation now lives on the
        // transformed output metadata itself.
        return Ok((output, None));
    }

    Ok((output, format_hint))
}

fn apply_output_sinks(
    config: &ResolvedConfig,
    ui: &UiState,
    result: &CommandOutput,
    rendered: String,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let output = match &result.body {
        CommandBody::Output(output) => Some(output.as_ref()),
        _ => None,
    };
    let edited = output
        .filter(|output| output.meta.wants_editor)
        .map(|_| {
            super::sink::edit_output(
                &result.render_for_target(config, &ui.render_settings, RenderTarget::Editor),
                result.output_format(&ui.render_settings),
                false,
            )
        })
        .transpose()?;
    maybe_copy_output(config, ui, result, edited.as_deref(), sink);
    // Editor changes only feed Y; paging renders the original structured result.
    let pager = if output.is_some_and(|output| output.meta.wants_page)
        && ui.render_settings.runtime.stdout_is_tty
        && !matches!(
            result.output_format(&ui.render_settings),
            OutputFormat::Json | OutputFormat::Csv | OutputFormat::Tsv
        ) {
        Some(super::sink::pager_command())
    } else {
        None
    };
    if let Some(pager) = pager {
        sink.write_stdout_paged(
            &result.render_for_target(config, &ui.render_settings, RenderTarget::Pager),
            &pager,
        );
        Ok(String::new())
    } else {
        Ok(rendered)
    }
}

fn pager_for_output(
    config: &ResolvedConfig,
    ui: &UiState,
    output: &CommandOutput,
    rendered: &str,
) -> Option<String> {
    if matches!(
        output.output_format(&ui.render_settings),
        OutputFormat::Json | OutputFormat::Csv | OutputFormat::Tsv
    ) || config.get_string("ui.pager") != Some("auto")
        || RenderTarget::screen(&ui.render_settings) != RenderTarget::OneShotScreen
        || rendered.lines().count() <= terminal_height()?
    {
        return None;
    }
    Some(super::sink::pager_command())
}

fn terminal_height() -> Option<usize> {
    terminal_size()
        .map(|(_, Height(rows))| rows as usize)
        .filter(|rows| *rows > 0)
        .or_else(|| {
            std::env::var("LINES")
                .ok()
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|rows| *rows > 0)
        })
}

pub(crate) fn render_saved_repl_output(
    config: &ResolvedConfig,
    ui: &UiState,
    output: &CommandOutput,
    stages: &[String],
    source: bool,
    sink: Option<&mut dyn UiSink>,
) -> Result<(String, CommandOutput)> {
    let mut staged = output.saved();
    staged.use_source |= source;
    staged.select_source();
    if let Some(recording) = crate::repl::recording::current() {
        recording.observe(&staged.body.json_value());
    }
    let context = match &staged.body {
        CommandBody::Text(_) | CommandBody::Json(_)
            if stages.is_empty() && !ui.render_settings.format_explicit =>
        {
            return Ok((staged.render(config, &ui.render_settings), staged));
        }
        CommandBody::Text(_) => "failed to replay staged textual output",
        CommandBody::Json(_) => "failed to replay staged JSON output",
        CommandBody::None | CommandBody::Output(_) => "failed to replay staged structured output",
    };
    staged.body = match staged.body.into_output() {
        Some(output) => CommandBody::Output(Box::new(output)),
        None => CommandBody::None,
    };
    staged.apply_stages(stages).wrap_err(context)?;
    let rendered = staged.render(config, &ui.render_settings);
    let rendered = match sink {
        Some(sink) => apply_output_sinks(config, ui, &staged, rendered, sink)?,
        None => rendered,
    };
    Ok((rendered, staged))
}

pub(crate) fn render_repl_command(
    config: &ResolvedConfig,
    ui: &UiState,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    mut result: CommandOutput,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let saved = result.saved();
    result.select_source();
    if let Some(recording) = crate::repl::recording::current() {
        recording.observe(&result.body.json_value());
    }
    if let Some(hints) = result.failure_report.as_ref().and_then(|report| {
        super::host::report_hint_commands(
            report,
            ui.verbosity,
            Some((line, &ui.render_settings.runtime.command_scope)),
        )
    }) {
        session.repl_feedback.remember_hints(hints);
    }

    if result.failure_report.is_some() {
        run_cli_command(config, ui, &result, sink)?;
        if let Some(report) = result.failure_report.take() {
            session.record_failure_report(
                line,
                std::sync::Arc::new(report),
                ui.render_settings.plain_copy_settings(),
            );
            if let Some(last) = session.last_failure.as_mut() {
                last.command_evidence = result.retained_evidence.clone();
            }
        }
        return Ok(String::new());
    }
    super::sink::write_diagnostics(sink, &result.diagnostics.0, &ui.render_settings);
    if !result.messages.is_empty() {
        emit_messages_for_ui(config, ui, &result.messages, ui.verbosity, sink);
    }
    let context = match &result.body {
        CommandBody::Text(_) => "failed to apply staged textual output pipeline",
        CommandBody::Json(_) => "failed to apply staged JSON output pipeline",
        CommandBody::None | CommandBody::Output(_) => {
            "failed to apply staged structured output pipeline"
        }
    };
    result.apply_stages(stages).wrap_err(context)?;
    let rendered = result.render(config, &ui.render_settings);
    session.last_output = Some((
        std::sync::Arc::new(result.saved()),
        ui.render_settings.clone(),
    ));
    let rendered = apply_output_sinks(config, ui, &result, rendered, sink)?;
    sink.write_stderr(&result.display_details);
    if !result.stderr_text.is_empty() {
        sink.write_stderr(&result.stderr_text);
    }
    if !matches!(saved.body, CommandBody::None) {
        session.record_success_output(line, saved, stages);
    }
    super::sink::write_diagnostics(sink, &result.diagnostics.1, &ui.render_settings);
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
