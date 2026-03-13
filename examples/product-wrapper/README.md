# Minimal Product Wrapper Example

This example crate shows the smallest honest product-wrapper shape on top of
`osp-cli`.

It demonstrates:

- one site-specific native command
- one thin wrapper app type around `osp_cli::App`
- one wrapper-owned builder entrypoint
- one product-owned defaults layer injected into host bootstrap
- one matching runtime-config helper for adjacent tooling

File roles:

- `src/lib.rs`: wrapper-owned defaults, native registry, `SiteApp`, and the
  optional config helper
- `src/main.rs`: one-line delegate to the wrapper crate

Try it:

```bash
cargo run --manifest-path examples/product-wrapper/Cargo.toml -- --help
cargo run --manifest-path examples/product-wrapper/Cargo.toml -- site-status
cargo run --manifest-path examples/product-wrapper/Cargo.toml -- --json config get extensions.site.banner
```

Copy-and-adapt order:

1. Rename `site_*` symbols to your product name.
2. Replace `SiteStatusCommand` with one real native command.
3. Move your defaults under `extensions.<product>.*`.
4. Keep `src/main.rs` thin and let `SiteApp::builder()` own the host wiring.

`SiteApp::builder()` injects `site_defaults()` directly into the host
bootstrap through `App::builder().with_product_defaults(...)`, so native
commands and `config` commands see the same wrapper-owned keys. The separate
`site_runtime_config_for(terminal)` helper is optional; it exists for
product-owned validation, tooling, and tests that need the same merged
defaults outside the host.
