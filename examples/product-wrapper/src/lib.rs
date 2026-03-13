use std::ffi::OsString;

use anyhow::Result;
use clap::Command;
use osp_cli::config::{
    ConfigError, ConfigLayer, ResolveOptions, ResolvedConfig, RuntimeConfigPaths, RuntimeDefaults,
    RuntimeLoadOptions, build_runtime_pipeline,
};
use osp_cli::{
    App, AppBuilder, NativeCommand, NativeCommandContext, NativeCommandOutcome,
    NativeCommandRegistry,
};

fn site_defaults() -> ConfigLayer {
    let mut layer = ConfigLayer::default();
    layer.set("extensions.site.enabled", true);
    layer.set_for_terminal("cli", "extensions.site.banner", "cli-wrapper");
    layer
}

pub fn site_runtime_config_for(terminal: &str) -> Result<ResolvedConfig, ConfigError> {
    let paths = RuntimeConfigPaths::discover();
    let mut defaults = RuntimeDefaults::from_process_env("dracula", "site> ").to_layer();
    defaults.extend_from_layer(&site_defaults());

    build_runtime_pipeline(
        defaults,
        None,
        &paths,
        RuntimeLoadOptions::default(),
        None,
        None,
    )
    .resolve(ResolveOptions::new().with_terminal(terminal))
}

struct SiteStatusCommand;

impl NativeCommand for SiteStatusCommand {
    fn command(&self) -> Command {
        Command::new("site-status").about("Show wrapper-specific status")
    }

    fn execute(
        &self,
        _args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Help(format!(
            "site wrapper active profile: {}\nsite banner: {}",
            context.config.active_profile(),
            context
                .config
                .get_string("extensions.site.banner")
                .unwrap_or("missing")
        )))
    }
}

fn site_native_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(SiteStatusCommand)
}

#[derive(Clone)]
pub struct SiteApp {
    inner: App,
}

impl SiteApp {
    pub fn builder() -> AppBuilder {
        App::builder()
            .with_native_commands(site_native_registry())
            .with_product_defaults(site_defaults())
    }

    pub fn new() -> Self {
        Self {
            inner: Self::builder().build(),
        }
    }

    pub fn run_process<I, T>(&self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.inner.run_process(args)
    }
}

impl Default for SiteApp {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_process<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    SiteApp::new().run_process(args)
}
