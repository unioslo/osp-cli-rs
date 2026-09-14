use osp_cli::cli::Cli;

#[test]
fn native_configuration_is_available_to_help_and_execution() {
    use osp_cli::app::BufferedUiSink;
    use osp_cli::config::{ConfigLayer, ResolvedConfig};
    use osp_cli::{
        App, NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    };
    use std::sync::{Arc, Mutex};

    struct ConfiguredCommand(Arc<Mutex<String>>);
    impl NativeCommand for ConfiguredCommand {
        fn configure(&self, config: &ResolvedConfig) {
            *self.0.lock().unwrap() = config
                .get_string("extensions.site.label")
                .unwrap()
                .to_string();
        }
        fn command(&self) -> clap::Command {
            clap::Command::new("site-probe").about(self.0.lock().unwrap().clone())
        }
        fn execute(
            &self,
            _: &[String],
            context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            assert_eq!(
                self.0.lock().unwrap().as_str(),
                context.config.get_string("extensions.site.label").unwrap()
            );
            Ok(NativeCommandOutcome::Exit(0))
        }
    }
    let mut defaults = ConfigLayer::default();
    defaults.set("extensions.site.label", "configured-site");
    let app = App::builder()
        .with_product_defaults(defaults)
        .with_native_commands(
            NativeCommandRegistry::new()
                .with_command(ConfiguredCommand(Arc::new(Mutex::new(String::new())))),
        )
        .build();
    let mut sink = BufferedUiSink::default();
    assert_eq!(
        app.run_process_with_sink(["osp", "--defaults-only", "--help"], &mut sink),
        0
    );
    assert!(sink.stdout.contains("configured-site"));
    assert_eq!(
        app.run_process_with_sink(["osp", "--defaults-only", "site-probe"], &mut sink),
        0
    );
}

#[test]
fn product_bootstrap_uses_host_grammar_and_keeps_command_arguments_local() {
    for (args, expected_user, expected_profile, expected_login, expected_no_env) in [
        (
            vec![
                "osp",
                "--user",
                "alice",
                "--profile=dev",
                "--login",
                "config",
                "show",
                "--no-env",
            ],
            Some("alice"),
            Some("dev"),
            Some(""),
            true,
        ),
        (
            vec![
                "osp",
                "siteadmin",
                "login",
                "--user",
                "bob",
                "--login=command-data",
            ],
            None,
            None,
            None,
            false,
        ),
        (
            vec![
                "osp",
                "--",
                "--login=literal",
                "--profile=literal",
                "--no-env",
            ],
            None,
            None,
            None,
            false,
        ),
        (
            vec!["osp", "--login=alice", "siteadmin", "--", "--login=bob"],
            None,
            None,
            Some("alice"),
            false,
        ),
        (
            vec!["osp", "--json", "--no-env", "--help"],
            None,
            None,
            None,
            true,
        ),
    ] {
        let (cli, matches) = Cli::bootstrap_from(
            args.clone(),
            [clap::Arg::new("login")
                .long("login")
                .num_args(0..=1)
                .require_equals(true)
                .default_missing_value("")],
        )
        .unwrap();
        assert_eq!(cli.user.as_deref(), expected_user, "{args:?}");
        assert_eq!(cli.profile.as_deref(), expected_profile, "{args:?}");
        assert_eq!(
            matches.get_one::<String>("login").map(String::as_str),
            expected_login,
            "{args:?}"
        );
        assert_eq!(cli.no_env, expected_no_env, "{args:?}");
    }
}
