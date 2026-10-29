use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Command;
use osp_cli::app::{AccessPolicy, AppRuntime, BufferedUiSink, UiSink};
use osp_cli::config::ConfigLayer;
use osp_cli::core::MessageLevel;
use osp_cli::core::command_policy::{CredentialState, SessionFacts};
use osp_cli::core::plugin::{
    DescribeCommandAuthV1, DescribeCredentialRequirementV1, DescribeSessionRequirementsV1,
    DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
};
use osp_cli::{
    App, NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    NativeProgressEvent,
};
use serde_json::{Value, json};

#[derive(Default)]
struct CredentialProbe(AtomicBool);

impl NativeCommand for CredentialProbe {
    fn command(&self) -> Command {
        let command = Command::new("credential-probe").subcommand_required(true);
        if self.0.load(Ordering::SeqCst) {
            command.subcommand(Command::new("read").alias("get"))
        } else {
            command
        }
    }

    fn prepare_invocation_metadata(
        &self,
        args: &[String],
        _context: &osp_cli::NativeCommandContext<'_>,
    ) -> Result<bool> {
        assert!(args == ["get"] || args == ["--help"]);
        Ok(!self.0.swap(true, Ordering::SeqCst))
    }

    fn auth(&self) -> Option<DescribeCommandAuthV1> {
        Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::Public),
            run_session: Some(DescribeSessionRequirementsV1 {
                auth_strength: None,
                credentials: vec![DescribeCredentialRequirementV1::valid("product")],
            }),
            ..DescribeCommandAuthV1::default()
        })
    }

    fn execute(
        &self,
        args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        let matches = self.command().try_get_matches_from(
            std::iter::once("credential-probe".to_owned()).chain(args.iter().cloned()),
        )?;
        assert_eq!(matches.subcommand_name(), Some("read"));
        Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
            source: None,
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!([{"resource": "db01", "cpu": 4, "ready": true}]),
            error: None,
            messages: Vec::new(),
            context: None,
            meta: ResponseMetaV1::default(),
        })))
    }
}

struct ProductAccess {
    credential: Arc<Mutex<CredentialState>>,
    paths: Arc<Mutex<Vec<Vec<String>>>>,
}

impl AccessPolicy for ProductAccess {
    fn session_facts(&self) -> SessionFacts {
        let credential = self.credential.lock().unwrap().clone();
        SessionFacts {
            authenticated: credential.valid,
            credentials: [("product".into(), credential)].into(),
        }
    }

    fn before_run(&self, path: &[String], _runtime: &mut AppRuntime) -> miette::Result<()> {
        self.paths.lock().unwrap().push(path.to_vec());
        *self.credential.lock().unwrap() = CredentialState::valid_for(1800);
        Ok(())
    }
}

#[test]
fn embedded_access_hook_reads_live_facts_and_prepares_each_execution_once() {
    let credential = Arc::new(Mutex::new(CredentialState {
        valid: false,
        ttl_seconds: None,
    }));
    let paths = Arc::new(Mutex::new(Vec::new()));
    let mut defaults = ConfigLayer::default();
    defaults.set("ui.format", "table");
    defaults.set("ui.command_formats.credential-probe.read", "json");
    let app = App::new()
        .with_product_defaults(defaults)
        .with_native_commands(NativeCommandRegistry::new().with_command(CredentialProbe::default()))
        .with_access_policy(ProductAccess {
            credential: credential.clone(),
            paths: paths.clone(),
        });
    let mut sink = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(
            ["osp", "--defaults-only", "credential-probe", "get"],
            &mut sink
        )
        .unwrap(),
        0
    );
    assert_eq!(
        serde_json::from_str::<Value>(&sink.stdout).unwrap(),
        json!([{"resource": "db01", "cpu": 4, "ready": true}])
    );
    assert_eq!(
        *paths.lock().unwrap(),
        vec![vec!["credential-probe".to_string()]]
    );

    for ttl in [1800, 60] {
        *credential.lock().unwrap() = CredentialState::valid_for(ttl);
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            app.run_with_sink(["osp", "--defaults-only", "--json", "doctor"], &mut sink)
                .unwrap(),
            0
        );
        let report: Value = serde_json::from_str(&sink.stdout).unwrap();
        assert_eq!(report["session"][0]["status"], "ok");
        assert_eq!(report["session"][0]["authenticated"], true);
        assert_eq!(
            report["session"][0]["credentials"],
            json!([{"service": "product", "valid": true, "ttl_seconds": ttl}])
        );
    }
    let mut human = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(["osp", "--defaults-only", "doctor"], &mut human)
            .unwrap(),
        0
    );
    assert!(human.stdout.contains("product"));
    assert!(human.stdout.contains("60"));
    assert_eq!(*credential.lock().unwrap(), CredentialState::valid_for(60));
    let mut sink = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(
            ["osp", "--defaults-only", "credential-probe", "--help"],
            &mut sink
        )
        .unwrap(),
        0
    );
    assert_eq!(paths.lock().unwrap().len(), 1);
    let mut sink = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(
            [
                "osp",
                "--defaults-only",
                "--json",
                "credential-probe",
                "get"
            ],
            &mut sink
        )
        .unwrap(),
        0
    );
    assert_eq!(paths.lock().unwrap()[1], vec!["credential-probe", "read"]);
}

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
            context: Default::default(),
            format_hint: Some("mreg".into()),
            progress_replace: true,
            ..ResponseMetaV1::default()
        })
        .with_frame(json!({"task_id": "task-42", "state": "assessed", "stages": [{"name": "planning", "complete": true}]}), ResponseMetaV1 {
            format_hint: Some("mreg".into()),
            progress_replace: true,
            ..ResponseMetaV1::default()
        })
        .with_messages(vec![ResponseMessageV1 {
            level: MessageLevel::Success,
            text: "Plan validated for task-42".into(),
        }]);
        // Stream consumers retain historical events without terminal coalescing.
        progress.replay = true;
        if context.can_present() {
            // An immediate frame exercises the embedding sink without a timer.
            progress.frame.as_mut().unwrap().meta.progress_replace = false;
            context.emit_progress_measured(progress)?;
        } else {
            context.emit_stream_measured(progress)?;
        }
        context.present(
            NativeProgressEvent::new(json!({"notice": notice})).with_meta(ResponseMetaV1 {
                context: Default::default(),
                format_hint: Some("value".into()),
                ..ResponseMetaV1::default()
            }),
        )?;
        context.flush_progress()?;
        Ok(NativeCommandOutcome::ResponseWithExit {
            response: Box::new(ResponseV1 {
                source: None,
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
                context: None,
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
    let app = App::new().with_native_commands(
        NativeCommandRegistry::new().with_command(ApprovalWorkflow(Arc::clone(&approved))),
    );
    let mut terminal = TerminalCapture::default();
    assert_eq!(
        app.run_with_sink(
            ["osp", "--defaults-only", "--plain", "provision"],
            &mut terminal
        )
        .unwrap(),
        3
    );
    assert!(terminal.0.stderr.contains("planning"));
    assert!(terminal.0.stderr.contains("Approval required"));
    assert!(terminal.0.stdout.contains("waiting_approval"));

    for (is_approved, expected_status, filter) in [
        (false, 3, "task_id=task-42"),
        (true, 0, "task_id=task-42"),
        (true, 0, "state=completed"),
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
                    "F",
                    filter
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
                "error": {"code": "command_failed", "message": "command failed", "details": null},
                "exit_code": 3,
                "messages": [],
                "data": data,
            })
        };
        assert_eq!(
            serde_json::from_str::<Value>(&sink.stdout).unwrap(),
            expected
        );
        let mut messages = serde_json::Deserializer::from_str(&sink.stderr).into_iter::<Value>();
        assert_eq!(
            messages.next().unwrap().unwrap(),
            json!({
                "messages": [{"level": "success", "text": "Plan validated for task-42"}]
            })
        );
        if filter == "task_id=task-42" {
            assert_eq!(
                messages.next().unwrap().unwrap(),
                json!({"task_id": "task-42", "state": "assessed"})
            );
        }
        assert!(messages.next().is_none());
    }
}

#[test]
fn embedded_product_bootstrap_options_feed_config_and_remain_visible_in_help() {
    let wrapper = Command::new("product")
        .arg(clap::Arg::new("site").long("site").required(true))
        .try_get_matches_from(["product", "--site", "uio"])
        .unwrap();
    let mut defaults = ConfigLayer::default();
    defaults.set(
        "extensions.site.name",
        wrapper.get_one::<String>("site").unwrap().clone(),
    );
    let app = App::new()
        .with_product_defaults(defaults)
        .with_product_help_option("--site <NAME>", "Select the product site")
        .with_native_commands(
            NativeCommandRegistry::new()
                .with_command(ApprovalWorkflow(Arc::new(AtomicBool::new(true)))),
        );

    let mut help = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(["osp", "--defaults-only", "--json", "--help"], &mut help)
            .unwrap(),
        0
    );
    let guide: Value = serde_json::from_str(&help.stdout).unwrap();
    let sections = guide["blocks"].as_array().unwrap();
    let options = sections
        .iter()
        .find(|section| section["section"] == "Product options")
        .unwrap();
    let option = options["blocks"][0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "--site <NAME>")
        .unwrap();
    assert_eq!(option["description"], "Select the product site");
    let commands = sections
        .iter()
        .find(|section| section["section"] == "Commands")
        .unwrap();
    assert!(
        commands["blocks"][0]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "provision"
                && entry["description"] == "Follow a product-owned approval workflow")
    );

    let mut config = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(
            [
                "osp",
                "--defaults-only",
                "--json",
                "config",
                "get",
                "extensions.site.name",
                "--sources"
            ],
            &mut config
        )
        .unwrap(),
        0
    );
    let values: Value = serde_json::from_str(&config.stdout).unwrap();
    assert_eq!(values[0]["key"], "extensions.site.name");
    assert_eq!(values[0]["value"], "uio");
    assert_eq!(values[0]["source"], "defaults");
}
