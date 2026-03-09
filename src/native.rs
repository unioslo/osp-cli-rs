use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use clap::Command;

use crate::completion::CommandSpec;
use crate::config::ResolvedConfig;
use crate::core::command_policy::CommandPolicyRegistry;
use crate::core::plugin::{DescribeCommandAuthV1, DescribeCommandV1, ResponseV1};
use crate::core::runtime::RuntimeHints;

#[derive(Debug, Clone)]
pub struct NativeCommandCatalogEntry {
    pub name: String,
    pub about: String,
    pub auth: Option<DescribeCommandAuthV1>,
    pub subcommands: Vec<String>,
    pub completion: CommandSpec,
}

pub struct NativeCommandContext<'a> {
    pub config: &'a ResolvedConfig,
    pub runtime_hints: RuntimeHints,
}

pub enum NativeCommandOutcome {
    Help(String),
    Response(Box<ResponseV1>),
    Exit(i32),
}

pub trait NativeCommand: Send + Sync {
    fn command(&self) -> Command;

    fn auth(&self) -> Option<DescribeCommandAuthV1> {
        None
    }

    fn describe(&self) -> DescribeCommandV1 {
        let mut describe = DescribeCommandV1::from_clap(self.command());
        describe.auth = self.auth();
        describe
    }

    fn execute(
        &self,
        args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome>;
}

#[derive(Clone, Default)]
pub struct NativeCommandRegistry {
    commands: Arc<BTreeMap<String, Arc<dyn NativeCommand>>>,
}

impl NativeCommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_command(mut self, command: impl NativeCommand + 'static) -> Self {
        self.register(command);
        self
    }

    pub fn register(&mut self, command: impl NativeCommand + 'static) {
        let mut next = (*self.commands).clone();
        let command = Arc::new(command) as Arc<dyn NativeCommand>;
        let name = normalize_name(&command.describe().name);
        next.insert(name, command);
        self.commands = Arc::new(next);
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn command(&self, name: &str) -> Option<&Arc<dyn NativeCommand>> {
        self.commands.get(&normalize_name(name))
    }

    pub fn catalog(&self) -> Vec<NativeCommandCatalogEntry> {
        self.commands
            .values()
            .map(|command| {
                let describe = command.describe();
                let completion = crate::plugin::conversion::to_command_spec(&describe);
                NativeCommandCatalogEntry {
                    name: describe.name.clone(),
                    about: describe.about.clone(),
                    auth: describe.auth.clone(),
                    subcommands: crate::plugin::conversion::direct_subcommand_names(&completion),
                    completion,
                }
            })
            .collect()
    }

    pub fn command_policy_registry(&self) -> CommandPolicyRegistry {
        let mut registry = CommandPolicyRegistry::new();
        for command in self.commands.values() {
            let describe = command.describe();
            register_describe_command_policies(&mut registry, &describe, &[]);
        }
        registry
    }
}

fn register_describe_command_policies(
    registry: &mut CommandPolicyRegistry,
    command: &DescribeCommandV1,
    parent: &[String],
) {
    let mut segments = parent.to_vec();
    segments.push(command.name.clone());
    if let Some(policy) = command.command_policy(crate::core::command_policy::CommandPath::new(
        segments.clone(),
    )) {
        registry.register(policy);
    }
    for subcommand in &command.subcommands {
        register_describe_command_policies(registry, subcommand, &segments);
    }
}

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry};
    use crate::core::command_policy::CommandPath;
    use crate::core::plugin::{
        DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
        ResponseMetaV1, ResponseV1,
    };
    use clap::Command;
    use serde_json::json;

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

    #[test]
    fn registry_catalog_exposes_completion_and_auth_metadata_unit() {
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
    }

    #[test]
    fn registry_normalizes_lookup_and_collects_root_policy_without_nested_auth_unit() {
        let registry = NativeCommandRegistry::new().with_command(TestNativeCommand);

        assert!(registry.command("LDAP").is_some());
        assert!(registry.command(" ldap ").is_some());

        let policy = registry.command_policy_registry();
        assert!(policy.contains(&CommandPath::new(["ldap"])));
        assert!(!policy.contains(&CommandPath::new(["ldap", "user"])));
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
        let registry = NativeCommandRegistry::new().with_command(TestNativeCommandWithNestedAuth);

        let policy = registry.command_policy_registry();
        let user_policy = policy
            .resolved_policy(&CommandPath::new(["ldap", "user"]))
            .expect("nested native policy should exist");
        assert_eq!(
            user_policy.required_capabilities,
            ["ldap.user.read".to_string()].into_iter().collect()
        );
    }
}
