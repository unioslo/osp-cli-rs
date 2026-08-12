//! Plugin process dispatch and validation boundary.
//!
//! This module exists so the rest of the app can treat plugin commands as
//! structured responses instead of hand-managing child processes, timeouts, and
//! payload validation everywhere.
//!
//! High-level flow:
//!
//! - resolve the provider that should handle a command
//! - execute it with the correct environment and timeout policy
//! - capture stdout/stderr and exit status
//! - validate the returned JSON payload before handing it back to the host
//!
//! Contract:
//!
//! - subprocess spawning and timeout policy live here
//! - higher layers should consume validated plugin results instead of shelling
//!   out directly
//! - plugin DTO validation should stay aligned with [`crate::core::plugin`]

use super::manager::{
    DiscoveredPlugin, PluginDispatchContext, PluginDispatchError, PluginManager, RawPluginOutput,
};
use crate::core::plugin::{DescribeV1, ResponseV1};
use anyhow::{Result, anyhow};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const PROCESS_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ETXTBSY_RETRY_COUNT: usize = 5;
const ETXTBSY_RETRY_DELAY: Duration = Duration::from_millis(10);
const ENV_OSP_COMMAND: &str = "OSP_COMMAND";
const MAX_PLUGIN_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

enum CommandRunError {
    Execute(std::io::Error),
    TimedOut { timeout: Duration, stderr: Vec<u8> },
    OutputTooLarge { stream: &'static str, limit: usize },
}

struct CommandOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

pub(super) struct CapturedOutput {
    pub(super) bytes: Vec<u8>,
    pub(super) exceeded_limit: bool,
}

struct ExecutedPluginCommand {
    provider: DiscoveredPlugin,
    raw: RawPluginOutput,
}

impl ExecutedPluginCommand {
    fn run(
        manager: &PluginManager,
        command: &str,
        args: &[String],
        context: &PluginDispatchContext,
    ) -> std::result::Result<Self, PluginDispatchError> {
        let provider = manager.resolve_provider(command, context.provider_override.as_deref())?;
        let raw = run_provider(&provider, command, args, context, manager.process_timeout)?;
        Ok(Self { provider, raw })
    }

    fn into_raw(self) -> RawPluginOutput {
        self.raw
    }

    fn into_response(self, command: &str) -> std::result::Result<ResponseV1, PluginDispatchError> {
        if self.raw.status_code != 0 {
            tracing::warn!(
                plugin_id = %self.provider.plugin_id,
                command = %command,
                status_code = self.raw.status_code,
                stderr = %self.raw.stderr.trim(),
                "plugin command exited with non-zero status"
            );
            return Err(PluginDispatchError::NonZeroExit {
                plugin_id: self.provider.plugin_id,
                status_code: self.raw.status_code,
                stderr: self.raw.stderr,
            });
        }

        let response: ResponseV1 = serde_json::from_str(&self.raw.stdout).map_err(|source| {
            tracing::warn!(
                plugin_id = %self.provider.plugin_id,
                command = %command,
                error = %source,
                "plugin command returned invalid JSON"
            );
            PluginDispatchError::InvalidJsonResponse {
                plugin_id: self.provider.plugin_id.clone(),
                source,
            }
        })?;

        response.validate_v1().map_err(|reason| {
            tracing::warn!(
                plugin_id = %self.provider.plugin_id,
                command = %command,
                reason = %reason,
                "plugin command returned invalid payload"
            );
            PluginDispatchError::InvalidResponsePayload {
                plugin_id: self.provider.plugin_id,
                reason,
            }
        })?;

        Ok(response)
    }
}

impl PluginManager {
    /// Runs a plugin command and returns its validated structured response.
    ///
    /// `command` is the full command path resolved against the active plugin
    /// catalog. `args` are passed to the plugin after that command name.
    /// `context` carries runtime hints, optional environment overrides, and an
    /// optional one-shot provider override for this dispatch only.
    ///
    /// # Errors
    ///
    /// Returns [`PluginDispatchError`] when provider resolution fails, the
    /// plugin subprocess cannot be executed, the subprocess times out, the
    /// plugin exits non-zero, or the returned JSON is syntactically or
    /// semantically invalid.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::{PluginDispatchContext, PluginDispatchError, PluginManager};
    ///
    /// let err = PluginManager::new(Vec::new())
    ///     .dispatch("shared", &[], &PluginDispatchContext::default())
    ///     .unwrap_err();
    ///
    /// assert!(matches!(err, PluginDispatchError::CommandNotFound { .. }));
    /// ```
    pub fn dispatch(
        &self,
        command: &str,
        args: &[String],
        context: &PluginDispatchContext,
    ) -> std::result::Result<ResponseV1, PluginDispatchError> {
        ExecutedPluginCommand::run(self, command, args, context)?.into_response(command)
    }

    /// Runs a plugin command and returns raw stdout, stderr, and exit status.
    ///
    /// Unlike [`PluginManager::dispatch`], this does not attempt to decode or
    /// validate plugin JSON output. Non-zero exit codes are returned in
    /// [`RawPluginOutput::status_code`] rather than surfaced as
    /// [`PluginDispatchError::NonZeroExit`].
    ///
    /// # Errors
    ///
    /// Returns [`PluginDispatchError`] when provider resolution fails, the
    /// plugin subprocess cannot be executed, or the subprocess times out.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::{PluginDispatchContext, PluginDispatchError, PluginManager};
    ///
    /// let err = PluginManager::new(Vec::new())
    ///     .dispatch_passthrough("shared", &[], &PluginDispatchContext::default())
    ///     .unwrap_err();
    ///
    /// assert!(matches!(err, PluginDispatchError::CommandNotFound { .. }));
    /// ```
    pub fn dispatch_passthrough(
        &self,
        command: &str,
        args: &[String],
        context: &PluginDispatchContext,
    ) -> std::result::Result<RawPluginOutput, PluginDispatchError> {
        Ok(ExecutedPluginCommand::run(self, command, args, context)?.into_raw())
    }
}

pub(super) fn describe_plugin(path: &std::path::Path, timeout: Duration) -> Result<DescribeV1> {
    let mut command = Command::new(path);
    command.arg("--describe");
    let started_at = Instant::now();
    tracing::debug!(
        executable = %path.display(),
        timeout_ms = timeout.as_millis(),
        "running plugin describe"
    );
    let output = run_command_with_timeout(command, timeout).map_err(|err| match err {
        CommandRunError::Execute(source) => {
            tracing::warn!(
                executable = %path.display(),
                error = %source,
                "plugin describe execution failed"
            );
            anyhow!(
                "failed to execute --describe for {}: {source}",
                path.display()
            )
        }
        CommandRunError::TimedOut { timeout, stderr } => {
            let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
            tracing::warn!(
                executable = %path.display(),
                timeout_ms = timeout.as_millis(),
                stderr = %stderr,
                "plugin describe timed out"
            );
            if stderr.is_empty() {
                anyhow!(
                    "--describe timed out after {} ms for {}",
                    timeout.as_millis(),
                    path.display()
                )
            } else {
                anyhow!(
                    "--describe timed out after {} ms for {}: {}",
                    timeout.as_millis(),
                    path.display(),
                    stderr
                )
            }
        }
        CommandRunError::OutputTooLarge { stream, limit } => anyhow!(
            "--describe from {} exceeded the {limit}-byte {stream} limit",
            path.display()
        ),
    })?;

    tracing::debug!(
        executable = %path.display(),
        elapsed_ms = started_at.elapsed().as_millis(),
        status = ?output.status.code(),
        "plugin describe completed"
    );

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("--describe failed with status {}", output.status)
        } else {
            format!(
                "--describe failed with status {}: {}",
                output.status, stderr
            )
        };
        return Err(anyhow!(message));
    }

    let describe: DescribeV1 = serde_json::from_slice(&output.stdout)
        .map_err(anyhow::Error::from)
        .map_err(|err| err.context(format!("invalid describe JSON from {}", path.display())))?;
    describe
        .validate_v1()
        .map_err(|err| anyhow!("invalid describe payload from {}: {err}", path.display()))?;

    Ok(describe)
}

pub(super) fn run_provider(
    provider: &DiscoveredPlugin,
    selected_command: &str,
    args: &[String],
    context: &PluginDispatchContext,
    timeout: Duration,
) -> std::result::Result<RawPluginOutput, PluginDispatchError> {
    let env_issues = context
        .env_issues_for(&provider.plugin_id)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !env_issues.is_empty() {
        tracing::warn!(
            plugin_id = %provider.plugin_id,
            command = %selected_command,
            issue_count = env_issues.len(),
            "plugin command blocked by invalid injected environment config"
        );
        return Err(PluginDispatchError::InvalidEnvironment {
            plugin_id: provider.plugin_id.clone(),
            issues: env_issues,
        });
    }

    let mut command = Command::new(&provider.executable);
    let started_at = Instant::now();
    tracing::debug!(
        plugin_id = %provider.plugin_id,
        executable = %provider.executable.display(),
        command = %selected_command,
        arg_count = args.len(),
        timeout_ms = timeout.as_millis(),
        "dispatching plugin command"
    );
    command.arg(selected_command);
    command.args(args);
    command.env(ENV_OSP_COMMAND, selected_command);
    for (key, value) in context.runtime_hints.env_pairs() {
        command.env(key, value);
    }
    for (key, value) in context.env_pairs_for(&provider.plugin_id) {
        command.env(key, value);
    }

    let output = run_command_with_timeout(command, timeout).map_err(|err| match err {
        CommandRunError::Execute(source) => {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                executable = %provider.executable.display(),
                command = %selected_command,
                error = %source,
                "plugin command execution failed"
            );
            PluginDispatchError::ExecuteFailed {
                plugin_id: provider.plugin_id.clone(),
                source,
            }
        }
        CommandRunError::TimedOut { timeout, stderr } => {
            let stderr_text = String::from_utf8_lossy(&stderr).to_string();
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                executable = %provider.executable.display(),
                command = %selected_command,
                timeout_ms = timeout.as_millis(),
                stderr = %stderr_text.trim(),
                "plugin command timed out"
            );
            PluginDispatchError::TimedOut {
                plugin_id: provider.plugin_id.clone(),
                timeout,
                stderr: stderr_text,
            }
        }
        CommandRunError::OutputTooLarge { stream, limit } => PluginDispatchError::OutputTooLarge {
            plugin_id: provider.plugin_id.clone(),
            stream,
            limit,
        },
    })?;

    tracing::debug!(
        plugin_id = %provider.plugin_id,
        executable = %provider.executable.display(),
        command = %selected_command,
        elapsed_ms = started_at.elapsed().as_millis(),
        status = ?output.status.code(),
        "plugin command completed"
    );

    Ok(RawPluginOutput {
        status_code: exit_status_code(output.status),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub(super) fn exit_status_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }

    1
}

fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<CommandOutput, CommandRunError> {
    configure_command_process_group(&mut command);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = DrainedChild::spawn(command).map_err(CommandRunError::Execute)?;
    let deadline = Instant::now() + timeout.max(Duration::from_millis(1));

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.finish(status).map_err(CommandRunError::Execute)?;
                return output.into_result();
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(PROCESS_WAIT_POLL_INTERVAL);
            }
            Ok(None) => {
                terminate_timed_out_child(child.child_mut());
                let status = child.wait().map_err(CommandRunError::Execute)?;
                let output = child.finish(status).map_err(CommandRunError::Execute)?;
                return Err(CommandRunError::TimedOut {
                    timeout,
                    stderr: output.stderr.bytes,
                });
            }
            Err(source) => return Err(CommandRunError::Execute(source)),
        }
    }
}

struct DrainedChild {
    child: Child,
    stdout: JoinHandle<std::io::Result<CapturedOutput>>,
    stderr: JoinHandle<std::io::Result<CapturedOutput>>,
}

struct DrainedOutput {
    status: ExitStatus,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
}

impl DrainedOutput {
    fn into_result(self) -> Result<CommandOutput, CommandRunError> {
        if self.stdout.exceeded_limit {
            return Err(CommandRunError::OutputTooLarge {
                stream: "stdout",
                limit: MAX_PLUGIN_OUTPUT_BYTES,
            });
        }
        if self.stderr.exceeded_limit {
            return Err(CommandRunError::OutputTooLarge {
                stream: "stderr",
                limit: MAX_PLUGIN_OUTPUT_BYTES,
            });
        }
        Ok(CommandOutput {
            status: self.status,
            stdout: self.stdout.bytes,
            stderr: self.stderr.bytes,
        })
    }
}

impl DrainedChild {
    fn spawn(mut command: Command) -> std::io::Result<Self> {
        let mut child = spawn_command_with_retry(&mut command)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("failed to capture child stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("failed to capture child stderr"))?;
        Ok(Self {
            child,
            stdout: spawn_capture_thread(stdout),
            stderr: spawn_capture_thread(stderr),
        })
    }

    fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait()
    }

    fn finish(self, status: ExitStatus) -> std::io::Result<DrainedOutput> {
        Ok(DrainedOutput {
            status,
            stdout: join_capture(self.stdout)?,
            stderr: join_capture(self.stderr)?,
        })
    }
}

fn spawn_capture_thread<R>(reader: R) -> JoinHandle<std::io::Result<CapturedOutput>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || capture_reader(reader, MAX_PLUGIN_OUTPUT_BYTES))
}

pub(super) fn capture_reader<R: Read>(
    mut reader: R,
    limit: usize,
) -> std::io::Result<CapturedOutput> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    let mut exceeded_limit = false;
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        let retained = read.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..retained]);
        exceeded_limit |= retained < read;
    }
    Ok(CapturedOutput {
        bytes,
        exceeded_limit,
    })
}

fn join_capture(
    handle: JoinHandle<std::io::Result<CapturedOutput>>,
) -> std::io::Result<CapturedOutput> {
    handle
        .join()
        .map_err(|_| std::io::Error::other("plugin output capture thread panicked"))?
}

fn spawn_command_with_retry(command: &mut Command) -> std::io::Result<Child> {
    for attempt in 0..=ETXTBSY_RETRY_COUNT {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(err) if is_text_file_busy(&err) && attempt < ETXTBSY_RETRY_COUNT => {
                thread::sleep(ETXTBSY_RETRY_DELAY);
            }
            Err(err) => return Err(err),
        }
    }

    Err(std::io::Error::other(
        "plugin spawn retry loop exhausted unexpectedly",
    ))
}

fn is_text_file_busy(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(26)
}

#[cfg(unix)]
fn configure_command_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_command_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_timed_out_child(child: &mut Child) {
    const SIGTERM: i32 = 15;
    const SIGKILL: i32 = 9;

    let process_group = child.id() as i32;
    let _ = signal_process_group(process_group, SIGTERM);
    let grace_deadline = Instant::now() + Duration::from_millis(50);

    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < grace_deadline => {
                thread::sleep(PROCESS_WAIT_POLL_INTERVAL);
            }
            Ok(None) | Err(_) => break,
        }
    }

    let _ = signal_process_group(process_group, SIGKILL);
}

#[cfg(not(unix))]
fn terminate_timed_out_child(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(unix)]
fn signal_process_group(process_group: i32, signal: i32) -> std::io::Result<()> {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }

    let result = unsafe { kill(-process_group, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
