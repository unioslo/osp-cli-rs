use super::{NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions, ResolvedConfig};
use crate::core::command_policy::CommandPath;
use crate::core::plugin::{
    DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
    ResponseMetaV1, ResponseV1,
};
use crate::core::runtime::RuntimeHints;
use clap::Command;
use serde_json::json;

fn resolved_config() -> ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver
        .resolve(ResolveOptions::default())
        .expect("resolved config")
}

fn native_context() -> NativeCommandContext<'static> {
    let config = Box::leak(Box::new(resolved_config()));
    NativeCommandContext {
        config,
        runtime_hints: RuntimeHints::default(),
    }
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
        });
        root.subcommands[0].auth = Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
            required_capabilities: vec!["ldap.user.read".to_string()],
            feature_flags: Vec::new(),
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
