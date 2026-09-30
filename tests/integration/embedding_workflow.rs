use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use clap::Command;
use osp_cli::app::{BufferedUiSink, UiSink};
use osp_cli::core::plugin::{PLUGIN_PROTOCOL_V1, ResponseMetaV1, ResponseV1};
use osp_cli::{
    App, NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    NativeProgressEvent,
};
use serde_json::{Value, json};

struct ApprovalWorkflow(Arc<AtomicBool>);

impl NativeCommand for ApprovalWorkflow {
    fn command(&self) -> Command {
        Command::new("provision").about("Follow a product-owned approval workflow")
    }

    fn execute(
        &self,
        _args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        let approved = self.0.load(Ordering::SeqCst);
        let notice = if approved {
            "Approval received"
        } else {
            "Approval required"
        };
        let mut progress = NativeProgressEvent::new(json!({
            "task_id": "task-42", "state": "assessed"
        }))
        .with_meta(ResponseMetaV1 {
            presentation_lines: vec!["Task assessed".into()],
            progress_replace: true,
            ..ResponseMetaV1::default()
        });
        // Historical progress may be coalesced until the operator interaction.
        progress.replay = true;
        context.emit_progress(progress)?;
        context.present(
            NativeProgressEvent::new(json!({"notice": notice})).with_meta(ResponseMetaV1 {
                presentation_lines: vec![notice.into()],
                ..ResponseMetaV1::default()
            }),
        )?;
        context.flush_progress()?;
        Ok(NativeCommandOutcome::ResponseWithExit {
            response: Box::new(ResponseV1 {
                protocol_version: PLUGIN_PROTOCOL_V1,
                ok: true,
                data: json!([{
                    "task_id": "task-42",
                    "state": if approved { "completed" } else { "waiting_approval" },
                    "cpu": 4,
                    "approved": approved,
                }]),
                error: None,
                messages: Vec::new(),
                meta: ResponseMetaV1::default(),
            }),
            exit_code: if approved { 0 } else { 3 },
        })
    }
}

#[derive(Default)]
struct TerminalCapture(BufferedUiSink);

impl UiSink for TerminalCapture {
    fn write_stdout(&mut self, text: &str) {
        self.0.write_stdout(text);
    }
    fn write_stderr(&mut self, text: &str) {
        self.0.write_stderr(text);
    }
    fn stderr_is_terminal(&self) -> bool {
        true
    }
    fn stderr_width(&self) -> Option<usize> {
        Some(80)
    }
    fn stderr_height(&self) -> Option<usize> {
        Some(24)
    }
}

#[test]
fn embedded_approval_workflow_preserves_notices_progress_and_typed_results() {
    let approved = Arc::new(AtomicBool::new(false));
    let app = App::builder()
        .with_native_commands(
            NativeCommandRegistry::new().with_command(ApprovalWorkflow(Arc::clone(&approved))),
        )
        .build();
    let mut terminal = TerminalCapture::default();
    assert_eq!(
        app.run_with_sink(
            ["osp", "--defaults-only", "--plain", "provision"],
            &mut terminal
        )
        .unwrap(),
        3
    );
    let assessed = terminal.0.stderr.find("Task assessed").unwrap();
    let notice = terminal.0.stderr.find("Approval required").unwrap();
    assert!(
        assessed < notice,
        "pending progress must precede the interaction"
    );
    assert!(terminal.0.stdout.contains("waiting_approval"));

    for (is_approved, expected_status, notice) in [
        (false, 3, "Approval required"),
        (true, 0, "Approval received"),
    ] {
        approved.store(is_approved, Ordering::SeqCst);
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            app.run_with_sink(
                [
                    "osp",
                    "--defaults-only",
                    "--json",
                    "provision",
                    "|",
                    "task-42"
                ],
                &mut sink
            )
            .unwrap(),
            expected_status
        );
        let data = json!([{
            "task_id": "task-42",
            "state": if is_approved { "completed" } else { "waiting_approval" },
            "cpu": 4,
            "approved": is_approved,
        }]);
        let expected = if is_approved {
            data
        } else {
            json!({
                "ok": false,
                "error": {"code": "command_failed", "message": "command failed"},
                "messages": [],
                "data": data,
            })
        };
        assert_eq!(
            serde_json::from_str::<Value>(&sink.stdout).unwrap(),
            expected
        );
        assert_eq!(sink.stderr, format!("task-42\nassessed\n{notice}\n"));
    }
}
