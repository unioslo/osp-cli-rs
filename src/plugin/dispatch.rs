use super::manager::{
    DiscoveredPlugin, PluginDispatchContext, PluginDispatchError, PluginManager, RawPluginOutput,
};
use crate::core::plugin::{DescribeV1, ResponseV1};
use anyhow::{Result, anyhow};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const ETXTBSY_RETRY_COUNT: usize = 5;
const ETXTBSY_RETRY_DELAY: Duration = Duration::from_millis(10);
const ENV_OSP_COMMAND: &str = "OSP_COMMAND";

enum CommandRunError {
    Execute(std::io::Error),
    TimedOut { timeout: Duration, stderr: Vec<u8> },
}

impl PluginManager {
    pub fn dispatch(
        &self,
        command: &str,
        args: &[String],
        context: &PluginDispatchContext,
    ) -> std::result::Result<ResponseV1, PluginDispatchError> {
        let provider = self.resolve_provider(command, context.provider_override.as_deref())?;

        let raw = run_provider(&provider, command, args, context, self.process_timeout)?;
        if raw.status_code != 0 {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                command = %command,
                status_code = raw.status_code,
                stderr = %raw.stderr.trim(),
                "plugin command exited with non-zero status"
            );
            return Err(PluginDispatchError::NonZeroExit {
                plugin_id: provider.plugin_id.clone(),
                status_code: raw.status_code,
                stderr: raw.stderr,
            });
        }

        let response: ResponseV1 = serde_json::from_str(&raw.stdout).map_err(|source| {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                command = %command,
                error = %source,
                "plugin command returned invalid JSON"
            );
            PluginDispatchError::InvalidJsonResponse {
                plugin_id: provider.plugin_id.clone(),
                source,
            }
        })?;

        response.validate_v1().map_err(|reason| {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                command = %command,
                reason = %reason,
                "plugin command returned invalid payload"
            );
            PluginDispatchError::InvalidResponsePayload {
                plugin_id: provider.plugin_id.clone(),
                reason,
            }
        })?;

        Ok(response)
    }

    pub fn dispatch_passthrough(
        &self,
        command: &str,
        args: &[String],
        context: &PluginDispatchContext,
    ) -> std::result::Result<RawPluginOutput, PluginDispatchError> {
        let provider = self.resolve_provider(command, context.provider_override.as_deref())?;
        run_provider(&provider, command, args, context, self.process_timeout)
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
        status_code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Output, CommandRunError> {
    configure_command_process_group(&mut command);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = spawn_command_with_retry(&mut command).map_err(CommandRunError::Execute)?;
    let deadline = Instant::now() + timeout.max(Duration::from_millis(1));

    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map_err(CommandRunError::Execute),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(PROCESS_WAIT_POLL_INTERVAL);
            }
            Ok(None) => {
                terminate_timed_out_child(&mut child);
                let output = child.wait_with_output().map_err(CommandRunError::Execute)?;
                return Err(CommandRunError::TimedOut {
                    timeout,
                    stderr: output.stderr,
                });
            }
            Err(source) => return Err(CommandRunError::Execute(source)),
        }
    }
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

    unreachable!("retry loop should always return or error");
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
