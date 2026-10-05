use crate::temp_support::make_temp_dir;
use osp_cli::app::{AppRuntime, BufferedUiSink, StartupHook};
use osp_cli::config::{
    ConfigLayer, ConfigSchema, LoaderPipeline, ResolveOptions, SchemaEntry, StaticLayerLoader,
    TomlFileLoader,
};
use osp_cli::core::command_policy::{
    CommandPath, CommandPolicy, CommandPolicyContext, CommandPolicyRegistry, VisibilityMode,
};
use serde_json::json;

struct ProductSessionBootstrap;

impl StartupHook for ProductSessionBootstrap {
    fn prepare(&self, runtime: &mut AppRuntime) -> miette::Result<()> {
        let host_config = runtime.config_state().resolved();
        let path = host_config
            .get_string("extensions.site.session.path")
            .expect("product defaults should provide the session manifest path");
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", host_config.active_profile());
        let mut schema = ConfigSchema::default();
        schema.set_allow_extensions_namespace(false);
        schema.insert(
            "extensions.site.session.authenticated",
            SchemaEntry::boolean().required(),
        );
        schema.insert(
            "extensions.site.session.capabilities",
            SchemaEntry::string_list().required(),
        );
        let session = LoaderPipeline::new(StaticLayerLoader::new(defaults))
            .with_file(TomlFileLoader::new(path.into()).required())
            .with_schema(schema)
            .resolve(ResolveOptions::new())
            .map_err(miette::Report::new)?;
        runtime.set_policy_context(
            CommandPolicyContext::default()
                .authenticated(
                    session
                        .get_bool("extensions.site.session.authenticated")
                        .expect("session schema should validate authentication state"),
                )
                .with_capabilities(
                    session
                        .get_string_list("extensions.site.session.capabilities")
                        .expect("session schema should validate capabilities"),
                ),
        );
        Ok(())
    }
}

#[test]
fn product_session_bootstrap_preserves_diagnostics_and_recovers_after_manifest_repair() {
    let temp = make_temp_dir("osp-cli-product-session-diagnostics");
    let session_path = temp.path().join("session.toml");
    let interrupted_manifest = concat!(
        "[default]\n",
        "extensions.site.session.authenticated = true\n",
        "extensions.site.session.capabilities = [\"site.config.read\"\n",
    );
    std::fs::write(&session_path, interrupted_manifest)
        .expect("interrupted session manifest should be available to the client");

    let mut product_defaults = ConfigLayer::default();
    product_defaults.set(
        "extensions.site.session.path",
        session_path.to_string_lossy().to_string(),
    );
    product_defaults.set("extensions.site.service.enabled", true);
    let mut policy = CommandPolicyRegistry::new();
    policy.register(
        CommandPolicy::new(CommandPath::new(["config"]))
            .visibility(VisibilityMode::CapabilityGated)
            .require_capability("site.config.read"),
    );
    let app = osp_cli::App::new()
        .with_product_defaults(product_defaults)
        .with_builtin_policy(policy)
        .with_startup_hook(ProductSessionBootstrap);

    let read_args = [
        "osp",
        "--defaults-only",
        "--json",
        "config",
        "get",
        "extensions.site.service.enabled",
    ];
    let mut failed_read = BufferedUiSink::default();
    let report = app
        .run_with_sink(read_args, &mut failed_read)
        .expect_err("an interrupted session manifest should report its actual parse failure");
    assert!(report.to_string().contains(session_path.to_str().unwrap()));
    let labels = report
        .labels()
        .expect("the parse failure should retain its source label")
        .collect::<Vec<_>>();
    assert_eq!(labels[0].label(), Some("invalid TOML starts here"));
    let source = report
        .source_code()
        .expect("the parse failure should retain the interrupted document");
    let snippet = source
        .read_span(labels[0].inner(), 2, 2)
        .expect("the source label should address the actual document");
    assert_eq!(snippet.name(), Some("config layer"));
    assert!(
        std::str::from_utf8(snippet.data())
            .unwrap()
            .contains("extensions.site.session.capabilities"),
    );
    let parser_diagnostic = report
        .diagnostic_source()
        .expect("the file context should retain the underlying parser diagnostic");
    assert!(parser_diagnostic.to_string().contains("TOML"));
    assert!(parser_diagnostic.source_code().is_some());

    let mut human = BufferedUiSink::default();
    let exit = app.run_process_with_sink(
        [
            "osp",
            "--defaults-only",
            "--plain",
            "-vv",
            "config",
            "get",
            "extensions.site.service.enabled",
        ],
        &mut human,
    );
    assert_eq!(exit, 1);
    let human_identity = human.stderr.split_whitespace().collect::<String>();
    assert!(
        human_identity.contains(session_path.to_str().unwrap()),
        "rendered session path should remain identifiable across wrapped lines: {}",
        human.stderr,
    );
    assert!(
        human
            .stderr
            .contains("extensions.site.session.capabilities")
    );
    assert!(human.stderr.contains("invalid TOML starts here"));

    let mut machine = BufferedUiSink::default();
    let exit = app.run_process_with_sink(read_args, &mut machine);
    assert_eq!(exit, 1);
    let envelope: serde_json::Value = serde_json::from_str(&machine.stderr)
        .expect("machine clients should receive a complete JSON error envelope");
    let message = envelope["error"]["message"]
        .as_str()
        .expect("the error envelope should carry the actionable rendered diagnostic");
    assert!(message.contains(session_path.to_str().unwrap()));
    assert!(message.contains("TOML"));
    assert_eq!(
        envelope,
        json!({
            "ok": false,
            "error": {"code": "command_error", "message": message},
            "exit_code": 1
        }),
    );

    std::fs::write(
        &session_path,
        concat!(
            "[default]\n",
            "extensions.site.session.authenticated = true\n",
            "extensions.site.session.capabilities = [\"site.config.read\"]\n",
        ),
    )
    .expect("the product should repair its session manifest");
    let mut recovered = BufferedUiSink::default();
    let exit = app.run_process_with_sink(read_args, &mut recovered);
    assert_eq!(exit, 0);
    let rows: serde_json::Value = serde_json::from_str(&recovered.stdout)
        .expect("the protected read should return typed JSON");
    assert_eq!(
        rows,
        json!([{"key": "extensions.site.service.enabled", "value": true}]),
    );
    assert!(recovered.stderr.is_empty(), "{}", recovered.stderr);
}
