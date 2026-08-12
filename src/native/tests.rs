use super::{
    NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    NativeProgressEvent, NativeProgressSink, NativeSessionContext,
};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions, ResolvedConfig};
use crate::core::command_policy::CommandPath;
use crate::core::plugin::{
    DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
    ResponseMetaV1, ResponseV1,
};
use crate::core::runtime::RuntimeHints;
use clap::Command;
use serde_json::json;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::Duration;

struct CompletionAugmentingCommand;

impl NativeCommand for CompletionAugmentingCommand {
    fn command(&self) -> Command {
        Command::new("deploy").arg(clap::Arg::new("provider").long("provider"))
    }

    fn augment_completion(&self, completion: &mut crate::completion::CommandSpec) {
        completion.flags.insert(
            "--flavor".to_string(),
            crate::completion::FlagNode::new()
                .suggestions([crate::completion::SuggestionEntry::from("m1.small")]),
        );
    }

    fn execute(
        &self,
        _args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> anyhow::Result<NativeCommandOutcome> {
        unreachable!("not used in completion test")
    }
}

#[test]
fn native_completion_augmentation_is_repl_only_unit() {
    let registry = NativeCommandRegistry::new().with_command(CompletionAugmentingCommand);

    assert!(
        !registry.catalog()[0]
            .completion
            .flags
            .contains_key("--flavor")
    );
    assert!(
        registry.completion_catalog()[0]
            .completion
            .flags
            .contains_key("--flavor")
    );
}

struct BackgroundCompletionCommand {
    name: &'static str,
    started: mpsc::Sender<&'static str>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl NativeCommand for BackgroundCompletionCommand {
    fn command(&self) -> Command {
        Command::new(self.name)
    }

    fn refresh_completion(&self) -> anyhow::Result<()> {
        self.started.send(self.name)?;
        let (lock, ready) = &*self.release;
        let mut released = lock.lock().expect("release lock");
        while !*released {
            released = ready.wait(released).expect("release wait");
        }
        Ok(())
    }

    fn execute(
        &self,
        _args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> anyhow::Result<NativeCommandOutcome> {
        unreachable!("not used in refresh test")
    }
}

#[test]
fn native_completion_refresh_is_background_and_filtered_unit() {
    let (started, observed) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let registry = NativeCommandRegistry::new()
        .with_command(BackgroundCompletionCommand {
            name: "visible",
            started: started.clone(),
            release: Arc::clone(&release),
        })
        .with_command(BackgroundCompletionCommand {
            name: "hidden",
            started,
            release: Arc::clone(&release),
        });

    registry.refresh_completion_in_background(|name| name == "visible");

    assert_eq!(observed.recv_timeout(Duration::from_secs(1)), Ok("visible"));
    assert!(observed.try_recv().is_err());

    let catalog_started = std::time::Instant::now();
    assert_eq!(registry.completion_catalog().len(), 2);
    assert!(
        catalog_started.elapsed() < Duration::from_millis(100),
        "completion catalog waited for the background refresh"
    );

    let (lock, ready) = &*release;
    *lock.lock().expect("release lock") = true;
    ready.notify_all();
}

#[test]
fn native_session_context_completion_refresh_request_is_one_shot_unit() {
    let context = NativeSessionContext::default();
    assert!(!context.take_completion_refresh_request());

    context.request_completion_refresh();

    assert!(context.take_completion_refresh_request());
    assert!(!context.take_completion_refresh_request());
}

fn resolved_config() -> ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("theme.path", Vec::<String>::new());
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver
        .resolve(ResolveOptions::default())
        .expect("resolved config")
}

fn native_context() -> NativeCommandContext<'static> {
    let config = Box::leak(Box::new(resolved_config()));
    NativeCommandContext::new(config, RuntimeHints::default())
}

struct TestNativeCommand;

impl NativeCommand for TestNativeCommand {
    fn command(&self) -> Command {
        Command::new("ldap")
            .about("Directory lookups")
            .subcommand(Command::new("user").about("Look up a user"))
    }

    fn auth(&self) -> Option<DescribeCommandAuthV1> {
        Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::Public),
            required_capabilities: Vec::new(),
            feature_flags: vec!["uio".to_string()],
            ..DescribeCommandAuthV1::default()
        })
    }

    fn execute(
        &self,
        args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> anyhow::Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!([{ "args": args }]),
            error: None,
            messages: Vec::new(),
            meta: ResponseMetaV1::default(),
        })))
    }
}

struct DefaultAuthCommand;

impl NativeCommand for DefaultAuthCommand {
    fn command(&self) -> Command {
        Command::new("version").about("Show version")
    }

    fn execute(
        &self,
        _args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> anyhow::Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Help("version help".to_string()))
    }
}

#[test]
fn registry_catalog_and_policy_projection_cover_lookup_completion_and_root_auth_unit() {
    let registry = NativeCommandRegistry::new().with_command(TestNativeCommand);

    let catalog = registry.catalog();
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].name, "ldap");
    assert_eq!(catalog[0].about, "Directory lookups");
    assert_eq!(
        catalog[0]
            .auth
            .as_ref()
            .and_then(|auth| auth.hint())
            .as_deref(),
        Some("feature: uio")
    );
    assert_eq!(catalog[0].subcommands, vec!["user".to_string()]);
    assert!(
        catalog[0]
            .completion
            .subcommands
            .iter()
            .any(|child| child.name == "user")
    );

    assert!(registry.command("LDAP").is_some());
    assert!(registry.command(" ldap ").is_some());

    let policy = registry.command_policy_registry();
    assert!(policy.contains(&CommandPath::new(["ldap"])));
    assert!(!policy.contains(&CommandPath::new(["ldap", "user"])));
}

#[test]
fn empty_registry_and_default_auth_catalog_paths_unit() {
    assert!(NativeCommandRegistry::new().is_empty());
    assert!(NativeCommandRegistry::new().command("missing").is_none());

    let describe = DefaultAuthCommand.describe();
    assert_eq!(describe.name, "version");
    assert!(describe.auth.is_none());
    assert!(describe.subcommands.is_empty());
}

#[test]
fn registered_command_executes_through_registry_unit() {
    let registry = NativeCommandRegistry::new().with_command(TestNativeCommand);
    let context = native_context();
    let outcome = registry
        .command("ldap")
        .expect("registered command")
        .execute(&["user".to_string()], &context)
        .expect("native command should execute");

    let NativeCommandOutcome::Response(response) = outcome else {
        panic!("expected response outcome");
    };
    assert_eq!(response.protocol_version, PLUGIN_PROTOCOL_V1);
    assert_eq!(response.data, json!([{ "args": ["user"] }]));
}

#[derive(Default)]
struct CapturingProgressSink {
    events: Mutex<Vec<serde_json::Value>>,
}

impl NativeProgressSink for CapturingProgressSink {
    fn emit(&self, event: NativeProgressEvent) -> anyhow::Result<()> {
        self.events.lock().expect("progress lock").push(event.data);
        Ok(())
    }
}

#[test]
fn native_context_emits_structured_progress_to_the_host_boundary_unit() {
    let config = resolved_config();
    let sink = CapturingProgressSink::default();
    let context =
        NativeCommandContext::new(&config, RuntimeHints::default()).with_progress_sink(&sink);

    context
        .emit_progress(NativeProgressEvent::new(json!({
            "status": {"name": "running", "code": 160},
            "message": "Creating virtual machine"
        })))
        .expect("progress emission should succeed");

    assert_eq!(
        *sink.events.lock().expect("progress lock"),
        vec![json!({
            "status": {"name": "running", "code": 160},
            "message": "Creating virtual machine"
        })]
    );
}

struct TestNativeCommandWithNestedAuth;

impl NativeCommand for TestNativeCommandWithNestedAuth {
    fn command(&self) -> Command {
        Command::new("ldap")
            .about("Directory lookups")
            .subcommand(Command::new("user").about("Look up a user"))
    }

    fn describe(&self) -> DescribeCommandV1 {
        let mut root = DescribeCommandV1::from_clap(
            Command::new("ldap")
                .about("Directory lookups")
                .subcommand(Command::new("user").about("Look up a user")),
        );
        root.auth = Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::Public),
            required_capabilities: Vec::new(),
            feature_flags: vec!["uio".to_string()],
            ..DescribeCommandAuthV1::default()
        });
        root.subcommands[0].auth = Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
            required_capabilities: vec!["ldap.user.read".to_string()],
            feature_flags: Vec::new(),
            ..DescribeCommandAuthV1::default()
        });
        root
    }

    fn execute(
        &self,
        _args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> anyhow::Result<NativeCommandOutcome> {
        unreachable!("not used in policy test");
    }
}

#[test]
fn registry_collects_nested_auth_policies_when_describe_is_overridden_unit() {
    let default_registry = NativeCommandRegistry::new().with_command(TestNativeCommand);
    assert!(
        default_registry
            .command_policy_registry()
            .resolved_policy(&CommandPath::new(["ldap", "user"]))
            .is_none()
    );

    let overridden_registry =
        NativeCommandRegistry::new().with_command(TestNativeCommandWithNestedAuth);
    let user_policy = overridden_registry
        .command_policy_registry()
        .resolved_policy(&CommandPath::new(["ldap", "user"]))
        .expect("nested native policy should exist");
    assert_eq!(
        user_policy.required_capabilities,
        ["ldap.user.read".to_string()].into_iter().collect()
    );
}
