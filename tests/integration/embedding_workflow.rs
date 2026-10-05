use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use clap::Command;
use osp_cli::app::{
    AccessRecoveryOutcome, AccessRecoveryRequest, AppRuntime, AppSession, BufferedUiSink,
    CommandAccessKind, CommandAccessRecovery, TerminalKind, UiSink,
};
use osp_cli::config::{ConfigLayer, ResolvedConfig};
use osp_cli::core::command_policy::{
    AccessReason, AuthStrength, CommandAccess, CommandPolicyContext, CredentialState,
};
use osp_cli::core::plugin::{
    DescribeAuthStrengthV1, DescribeCommandAuthV1, DescribeCredentialRequirementV1,
    DescribeSessionRequirementsV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
    ResponseMessageLevelV1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
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
        _config: &ResolvedConfig,
    ) -> Result<bool> {
        assert_eq!(args, &["get"]);
        Ok(!self.0.swap(true, Ordering::SeqCst))
    }

    fn auth(&self) -> Option<DescribeCommandAuthV1> {
        Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::Public),
            run_session: Some(DescribeSessionRequirementsV1 {
                auth_strength: Some(DescribeAuthStrengthV1::Strong),
                credentials: vec![DescribeCredentialRequirementV1::Fresh {
                    service: "product".into(),
                    min_ttl_seconds: 900,
                }],
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
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!([{"resource": "db01", "cpu": 4, "ready": true}]),
            error: None,
            messages: Vec::new(),
            meta: ResponseMetaV1::default(),
        })))
    }
}

struct ProductAccessRecovery {
    credential: Arc<Mutex<CredentialState>>,
    requests: Arc<Mutex<Vec<AccessRecoveryRequest>>>,
}

impl CommandAccessRecovery for ProductAccessRecovery {
    fn refresh(&self, runtime: &mut AppRuntime) -> miette::Result<()> {
        let context = runtime
            .auth()
            .policy_context()
            .clone()
            .with_credential("product", self.credential.lock().unwrap().clone());
        runtime.set_policy_context(context);
        Ok(())
    }

    fn try_recover(
        &self,
        request: &AccessRecoveryRequest,
        runtime: &mut AppRuntime,
        _session: &mut AppSession,
    ) -> miette::Result<AccessRecoveryOutcome> {
        self.requests.lock().unwrap().push(request.clone());
        *self.credential.lock().unwrap() = CredentialState::valid_for(1800);
        self.refresh(runtime)?;
        Ok(AccessRecoveryOutcome::Recovered)
    }
}

#[test]
fn embedded_access_recovery_refreshes_current_facts_and_retries_canonical_command() {
    let credential = Arc::new(Mutex::new(CredentialState::valid_for(1800)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = App::new()
        .with_native_commands(NativeCommandRegistry::new().with_command(CredentialProbe::default()))
        .with_policy_context(
            CommandPolicyContext::default()
                .with_auth_strength(AuthStrength::Strong)
                .with_credential("product", CredentialState::valid_for(1800)),
        )
        .with_access_recovery(ProductAccessRecovery {
            credential: Arc::clone(&credential),
            requests: Arc::clone(&requests),
        });

    let session_diagnostics = || {
        let mut sink = BufferedUiSink::default();
        assert_eq!(
            app.run_with_sink(["osp", "--defaults-only", "--json", "doctor"], &mut sink)
                .unwrap(),
            0
        );
        let report: Value = serde_json::from_str(&sink.stdout).unwrap();
        report["session"][0].clone()
    };

    for ttl in [1800, 60] {
        *credential.lock().unwrap() = CredentialState::valid_for(ttl);
        // Doctor refreshes the product-owned facts before reporting them. It
        // does not require the protected command's credential upgrade.
        let current = session_diagnostics();
        assert_eq!(current["status"], "ok");
        assert_eq!(current["authenticated"], true);
        assert_eq!(current["auth_strength"], "strong");
        assert_eq!(
            current["credentials"],
            json!([{"service": "product", "valid": true, "ttl_seconds": ttl}])
        );
        assert_eq!(*credential.lock().unwrap(), CredentialState::valid_for(ttl));
        let mut doctor_sink = BufferedUiSink::default();
        assert_eq!(
            app.run_with_sink(["osp", "--defaults-only", "doctor"], &mut doctor_sink)
                .unwrap(),
            0
        );
        assert!(doctor_sink.stdout.contains("product"));
        assert!(doctor_sink.stdout.contains(&ttl.to_string()));
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
                &mut sink,
            )
            .unwrap(),
            0
        );
        assert_eq!(
            serde_json::from_str::<Value>(&sink.stdout).unwrap(),
            json!([{"resource": "db01", "cpu": 4, "ready": true}])
        );
        assert_eq!(
            *credential.lock().unwrap(),
            CredentialState::valid_for(1800)
        );
        assert_eq!(
            session_diagnostics()["credentials"],
            json!([{"service": "product", "valid": true, "ttl_seconds": 1800}])
        );
    }
    assert_eq!(
        *requests.lock().unwrap(),
        vec![AccessRecoveryRequest::new(
            TerminalKind::Cli,
            CommandAccessKind::External,
            "credential-probe read",
            CommandAccess::visible_denied(AccessReason::InsufficientCredentialTtl {
                service: "product".into(),
                required_ttl_seconds: 900,
            }),
        )]
    );
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
            presentation_lines: vec!["Task assessed".into()],
            progress_replace: true,
            ..ResponseMetaV1::default()
        })
        .with_messages(vec![ResponseMessageV1 {
            level: ResponseMessageLevelV1::Success,
            text: "Plan validated for task-42".into(),
        }]);
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
        let mut messages = serde_json::Deserializer::from_str(&sink.stderr).into_iter::<Value>();
        assert_eq!(
            messages.next().unwrap().unwrap(),
            json!({
                "messages": [{"level": "success", "text": "Plan validated for task-42"}]
            })
        );
        assert_eq!(
            sink.stderr[messages.byte_offset()..].trim_start_matches('\n'),
            format!("task-42\nassessed\n{notice}\n")
        );
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
    let app = App::builder()
        .with_product_defaults(defaults)
        .with_product_help_option("--site <NAME>", "Select the product site")
        .with_native_commands(
            NativeCommandRegistry::new()
                .with_command(ApprovalWorkflow(Arc::new(AtomicBool::new(true)))),
        )
        .build();

    let mut help = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(["osp", "--defaults-only", "--json", "--help"], &mut help)
            .unwrap(),
        0
    );
    let guide: Value = serde_json::from_str(&help.stdout).unwrap();
    let sections = guide[0]["sections"].as_array().unwrap();
    let options = sections
        .iter()
        .find(|section| section["title"] == "Product options")
        .unwrap();
    assert_eq!(options["kind"], "options");
    let option = options["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "--site <NAME>")
        .unwrap();
    assert_eq!(option["short_help"], "Select the product site");
    let commands = sections
        .iter()
        .find(|section| section["kind"] == "commands")
        .unwrap();
    assert!(
        commands["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "provision"
                && entry["short_help"] == "Follow a product-owned approval workflow")
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
