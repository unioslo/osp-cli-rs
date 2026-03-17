# Changelog

All notable changes to this project will be documented in this file.

The current release flow requires one versioned changelog section and one
matching `docs/releases/vX.Y.Z.md` file before a tag can be published.

## [1.5.1] - 2026-03-17

- Stabilizes the internal host, UI, config, and REPL architecture with a broad behavior-preserving cleanup that centralizes ownership without changing the public release shape.
- Tightens rustdoc, release metadata, and local gate discipline so public API examples, release checks, and coverage/readiness contracts stay aligned with the crate surface.
- Removes low-signal duplicate tests, restores changed-file coverage on the app host facade, and keeps the full test and coverage gates green after the internal reorganization.

## [1.4.5] - 2026-03-08

- First real public binary release of the Rust `osp` CLI, aligned to the current `osprov-cli` version line.
- Ships the rebuilt UI and REPL model with explicit `expressive`, `compact`, and `austere` presentation profiles.
- Enforces release verification, coverage gating, versioned release notes, and changelog-backed tagging.

## [1.4.6] - 2026-03-08

- Promoted the root package to the canonical single-crate implementation and removed the mirrored `osp_*` modules from the active build.
- Compiled presentation presets into canonical config/UI keys so REPL and help behavior stop relying on ad hoc fallback logic.
- Finished the crates.io release path with trusted publishing while keeping cross-platform GitHub release artifacts.

## [1.4.7] - 2026-03-08

- Adds a generic upstream command-policy layer with typed visibility, runnability, policy registry, and access evaluation so downstream distributions can supply auth facts without reimplementing policy mechanics.
- Extends plugin describe metadata, plugin listings, and REPL/help surfaces with generic auth metadata and compact auth hints while keeping capability semantics downstream-owned.
- Tightens upstream dispatch and runtime integration so command policy is enforced consistently across discovery, UI surfaces, and execution, with coverage kept above the repo push gate.

## [1.4.8] - 2026-03-09

- Adds the first-class native top-level command registry so downstream products can register commands like `osp ldap` cleanly through upstream help, completion, REPL, and dispatch.
- Removes the long-stale legacy `workspace/` mirror and updates release/test tooling to reflect the single-crate layout.
- Stabilizes the release verification path by fixing clippy noise in native command outcomes, hardening brittle fixture tests, and lowering the changed-file coverage floor to `85%`.

## [1.4.9] - 2026-03-11

- Tightens the public Rust API around canonical entrypoints and guided construction so embedders have one documented path for builders, constructors, and runtime setup.
- Centralizes host assembly and runtime planning so startup, rebuild, plugin activation, rendering, and REPL coordination stop re-deriving the same policy in multiple places.
- Rebuilds the intro/help document path around semantic `osp` template blocks, ordered guide sections, and shared ruled chrome so REPL startup and help output keep the intended structure across rich, compact, ASCII, and JSON surfaces.
- Fixes quick-search and structural DSL restore semantics so filtered help/intro payloads keep only the intended branches without resurrecting stale canonical buckets or leaking unrelated siblings.
- Replaces the `JQ` DSL verb's external `jq` subprocess dependency with an embedded `jaq` evaluator so the pipeline stays stable in-process while keeping the familiar `JQ` user surface.
- Consolidates architecture, contract, integration, end-to-end, and coverage-gate tests so failures localize faster while duplicate happy-path coverage drops below the old churn-heavy baseline.

## [1.5.0] - 2026-03-13

- Tightens the public API around the real long-term seams: typed config-store edit options, `App::builder()` as the canonical host front door, wrapper-owned product defaults, and a clearer guided-construction story for embedders.
- Removes retired DSL rollout compatibility shims, hardens runtime and completion paths against panic-style failure, and adds regression guards that keep the non-test runtime free of `panic!`, `unwrap`, and `expect`.
- Keeps explicit JSON output machine-readable across headless and PTY execution by forcing plain rendering on JSON surfaces, and hardens the local merge gate by running contract and integration lanes under a hermetic runtime environment.
- Rebuilds the rustdoc and repo docs experience around credible entrypoints, wrapper guidance, and runnable examples, including a minimal product-wrapper example crate that shows how downstream teams should inject native commands and product defaults.
