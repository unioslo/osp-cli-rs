# Embedding And Product Wrappers

This guide is for teams building a site-specific product crate on top of
`osp-cli`.

Use this when your product wants:

- the upstream CLI/REPL host
- the upstream config system and output/rendering behavior
- site-specific commands, policy, auth, or integrations

Do not use this guide if you are only writing external plugins. That path is
covered by [USING_PLUGINS.md](USING_PLUGINS.md),
[WRITING_PLUGINS.md](WRITING_PLUGINS.md), and
[PLUGIN_PROTOCOL.md](PLUGIN_PROTOCOL.md).

If you want the shortest copy-and-adapt path, read `Recommended Shape` first
and then jump straight to `Worked Recipe`.

The matching runnable example lives in
[`examples/product-wrapper`](../examples/product-wrapper).

## Fast Path

If you want the shortest successful wrapper:

1. Copy [`examples/product-wrapper/src/lib.rs`](../examples/product-wrapper/src/lib.rs)
   and [`examples/product-wrapper/src/main.rs`](../examples/product-wrapper/src/main.rs).
2. Rename `site_*` and `SiteApp` to your product name.
3. Move defaults under `extensions.<product>.*`.
4. Replace `SiteStatusCommand` with one real native command.
5. Keep `main.rs` thin and let `SiteApp::builder()` own host wiring.

You can ignore for now:

- [`osp_cli::app::AppStateBuilder`]
- manual runtime/session assembly
- the optional `site_runtime_config_for(...)` helper if you do not need
  wrapper-owned tooling/tests outside the host

## Ownership Split

Keep the split boring:

- `osp-cli` owns generic mechanism
- your product crate owns site-specific facts

Generic mechanism includes:

- CLI and REPL host behavior
- config loading, precedence, and explanation
- rendering and output formatting
- completion/help infrastructure
- native-command registration mechanics

Site-specific facts include:

- auth and policy inputs
- domain integrations
- native commands that expose those integrations
- site-only config under your own namespace

If a change could make sense for another site without changing meaning, it
probably belongs upstream.

## Recommended Shape

The normal wrapper shape is:

1. Keep a thin product-level app type that contains `osp_cli::App`.
2. Expose one wrapper-owned `builder()` that applies your defaults and native
   commands.
3. Build a `NativeCommandRegistry` for your site-specific commands.
4. Build one `ConfigLayer` containing your product-owned defaults under
   `extensions.<site>.*`.
5. Inject both through `App::builder()`.
6. Expose a small product API such as `run_process`, `builder`, or
   `assembly`.

Minimal sketch:

```rust
use std::ffi::OsString;

fn site_defaults() -> osp_cli::config::ConfigLayer {
    let mut defaults = osp_cli::config::ConfigLayer::default();
    defaults.set("extensions.site.enabled", true);
    defaults.set_for_terminal("cli", "extensions.site.banner", "cli-wrapper");
    defaults
}

#[derive(Clone)]
pub struct SiteApp {
    inner: osp_cli::App,
}

impl SiteApp {
    pub fn builder() -> osp_cli::AppBuilder {
        osp_cli::App::builder()
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

fn site_native_registry() -> osp_cli::NativeCommandRegistry {
    osp_cli::NativeCommandRegistry::new()
    // .with_command(MyNativeCommand)
}
```

That shape keeps the generic host untouched while giving the product crate one
obvious place to add its own commands and state.

## Wrapper Checklist

For a minimal but honest wrapper crate, keep this checklist true:

- one wrapper app type owns `osp_cli::App`
- one wrapper `builder()` applies product defaults and native commands
- one namespace owns product-only config: `extensions.<product>.*`
- one registration point owns native commands
- `src/main.rs` delegates into the wrapper crate instead of rebuilding host
  setup inline
- any extra config helper is optional and used only by wrapper-owned tooling,
  tests, or validation

## Worked Recipe

This is the end-to-end pattern to copy and adapt.

It shows:

1. product-owned defaults injected into the same host bootstrap path used by
   native commands and `config` commands
2. the optional matching helper for product-owned tooling that wants to
   resolve config outside the host
3. one complete native command
4. a thin product app wrapper that keeps upstream host startup behavior

Keep the file split simple:

- `src/lib.rs` owns defaults, native command registration, and the wrapper app
- `src/main.rs` should usually be a one-line delegate into that wrapper
- the config helper is for tooling/tests, not for ordinary startup

```rust
use std::ffi::OsString;

use anyhow::Result;
use clap::Command;
use osp_cli::config::{
    ConfigError, ConfigLayer, ResolveOptions, ResolvedConfig, RuntimeConfigPaths,
    RuntimeDefaults, RuntimeLoadOptions, build_runtime_pipeline,
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

fn site_runtime_config_for(terminal: &str) -> Result<ResolvedConfig, ConfigError> {
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
        Command::new("site-status").about("Show site-specific status")
    }

    fn execute(
        &self,
        _args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Help(format!(
            "site enabled: {}",
            context
                .config
                .get_bool("extensions.site.enabled")
                .unwrap_or(false)
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
```

`src/main.rs` should usually stay boring:

```rust
fn main() {
    std::process::exit(site_product::SiteApp::new().run_process(std::env::args_os()));
}
```

Notes:

- the wrapper crate owns `extensions.site.*` defaults and native commands
- `osp_cli::App` still owns the generic host behavior
- `with_product_defaults(...)` pushes wrapper defaults into the same runtime
  bootstrap path used by native commands, help rendering, `config get`, and
  `config explain`
- `site_runtime_config_for(terminal)` is optional and meant for product-owned
  tests, validation, and adjacent tooling that need the same merged defaults
  outside the host
- `SiteApp::builder()` is the clean downstream seam when your product wants to
  keep the upstream host but still expose one wrapper-owned construction path
- if the product does not need its own preflight config step, keep the wrapper
  thinner and only inject the native registry and product defaults

## Common Mistakes

Avoid these wrapper-crate mistakes:

- putting product-only keys at the top level instead of under
  `extensions.<product>.*`
- resolving wrapper config in a side channel while the host reads a different
  config snapshot
- putting startup wiring directly in `main.rs`
- leaking site-specific state into `osp-cli::app` instead of keeping it in the
  wrapper crate
- forking generic help/completion/config behavior when native commands and
  product defaults are enough

## Config Strategy

Do not create a second config system in the wrapper crate.

The intended approach is:

- keep product-only keys under a dedicated namespace such as
  `extensions.<site>.*`
- continue using upstream `ConfigLayer`, `LoaderPipeline`,
  `ConfigResolver`, and `ResolvedConfig`
- use `RuntimeDefaults`, `RuntimeConfigPaths`, and
  `build_runtime_pipeline` when you need stock host-style bootstrap

If you need product-specific defaults, inject them as normal config layers.
Do not fork source precedence or file-discovery rules unless the product
really needs a different contract.

## Native Commands

Use native commands when the product wants built-in commands that participate
in the same help, completion, policy, and dispatch surfaces as the rest of the
host.

Keep the boundary small:

- command implementations should consume a `NativeCommandContext`
- product-specific state should live in the wrapper crate, not in
  `osp-cli::app`
- registration should happen in one place, usually a product integrations
  module

## What Not To Fork

Avoid downstream copies of:

- config precedence logic
- CLI parsing and REPL shell behavior
- help/completion/catalog wiring
- plugin protocol or subprocess dispatch mechanics
- renderer decisions and output shape rules

If those need to change generically, upstream is the right place.

## Pointers

For rendered API docs, prefer docs.rs or `cargo doc --open`.

For a runnable rustdoc version of the minimal wrapper pattern, see the crate
root in [`src/lib.rs`](../src/lib.rs).

For a copyable wrapper crate, start with
[`examples/product-wrapper/src/lib.rs`](../examples/product-wrapper/src/lib.rs)
and [`examples/product-wrapper/src/main.rs`](../examples/product-wrapper/src/main.rs).

For code-level entrypoints, start with:

- [`src/lib.rs`](../src/lib.rs)
- [`src/app/mod.rs`](../src/app/mod.rs)
- [`src/native.rs`](../src/native.rs)
- [`src/config/mod.rs`](../src/config/mod.rs)
- [`src/config/runtime.rs`](../src/config/runtime.rs)
