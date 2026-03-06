# Quality Lift Plan

This document turns the current architecture review into a living cleanup plan.
It is meant to be updated as work lands. Check boxes in the same PR that makes
the change.

## Goal

Build a Rust CLI architecture that is cleaner than `osprov-cli`, not just a
parity port of its behavior and debt.

The rewrite should keep the good parts:

- explicit module boundaries
- typed config and output models
- snapshot-style state
- testable render and dispatch seams

The rewrite should not carry over the bad parts:

- hidden resolver magic
- permissive DSL behavior that masks mistakes
- UI code that decides runtime behavior from ambient process state
- compatibility shims that silently lose information

## Four Cross-Cutting Quality Lifts

### 1. Make UI Pure And Runtime-Resolved

- [ ] Move TTY, env, width, locale, and color/unicode probing out of `osp-ui`
      and into runtime/bootstrap code.
- [x] Make `osp-ui` render only from explicit `RenderSettings` /
      `ResolvedRenderSettings`.
- [ ] Keep `osp-ui` free of command semantics, plugin identity, auth state, and
      dispatch policy.
- [x] Add tests that prove the same input document renders the same way without
      reading process-global state.

Definition of done:
`osp-ui` becomes a pure formatting/render crate, not a runtime policy crate.

### 2. Make DSL Strict By Default

- [x] Keep one parser and one stage model.
- [x] Unknown verbs error by default.
- [x] Malformed quotes and malformed pipelines error by default.
- [ ] Put any Python-compat fallbacks behind explicit compatibility flags or
      isolated translation layers.
- [x] Make ambiguous key resolution explicit and testable.

Definition of done:
the DSL stops "helping" by guessing when input is invalid.

### 3. Make `OutputResult` The Only Non-Lossy DSL/UI Boundary

- [x] Remove APIs that silently downgrade grouped output to flat rows or empty
      vectors.
- [ ] Standardize command, DSL, and rendering boundaries on
      `osp_core::output_model::OutputResult`.
- [ ] Keep rows as a convenience input only where there is no information loss.
- [ ] Ensure copy/group/meta flags survive end-to-end.

Definition of done:
no valid pipeline result can be erased by passing through the "simple" API.

### 4. Keep Config/Core Explicit But Smaller

- [ ] Keep strict schema, explicit layers, snapshot replacement, and revision
      tracking.
- [ ] Remove legacy `context` carryover from core resolution once migration
      permits it.
- [ ] Trim mechanical scaffolding where a simpler typed representation will do.
- [ ] Keep network/auth logic out of config resolution.
- [ ] Prefer visible, local complexity over hidden dynamic behavior.

Definition of done:
config/core stay explicit and auditable without growing into boilerplate-heavy
ceremony.

## Workspace Plan

## `osp-core`

Role:
Own the smallest stable shared data model for rows, grouped output, render
metadata, runtime enums, and plugin protocol primitives.

Checklist:

- [ ] Keep `Row`, `OutputItems`, `OutputMeta`, and `OutputResult` minimal and
      stable.
- [ ] Do not add policy, parsing, or rendering decisions here.
- [ ] Tighten docs around what metadata must survive end-to-end.
- [ ] Reject convenience helpers that flatten or discard grouped output.

## `osp-config`

Role:
Own loading, precedence, validation, interpolation, explain, and persistence.

Checklist:

- [ ] Preserve one-pass profile selection followed by one-pass resolution.
- [ ] Remove legacy `context` compatibility from the resolved core model when
      callers no longer depend on it.
- [ ] Review `RuntimeDefaults` and replace hand-maintained boilerplate with a
      simpler typed construction path where practical.
- [ ] Keep secrets handling strict and explicit.
- [ ] Keep unknown-key rejection strict except for approved extension
      namespaces.
- [ ] Keep config free of alias parsing, auth refresh, and API client logic.

## `osp-dsl`

Role:
Own parsing, stage typing, and pipeline execution semantics.

Checklist:

- [x] Remove silent fallback from unknown verbs to quick search.
- [x] Remove naive fallback parsing for malformed quoted pipelines.
- [x] Make the parser and execution contracts strict by default.
- [ ] Keep compatibility shims isolated and opt-in if they are still needed.
- [x] Make `OutputResult` the primary execution result surface.
- [x] Remove or redesign lossy helpers that return `Vec<Row>` only.
- [x] Make grouping/refinement semantics explicit: flat bucket model, preserved
      regroup aggregates, and clear errors for ambiguous or structured keys.
- [ ] Document stage classes clearly: row-preserving, grouping, aggregating,
      external-process, copy/meta-affecting.

## `osp-ports`

Role:
Own business-facing traits and domain contracts.

Checklist:

- [ ] Keep traits narrow and use-case driven.
- [ ] Do not leak clap/CLI/UI/renderer concerns into port interfaces.
- [ ] Keep shared parsing helpers here only if they are truly domain contracts,
      not CLI conveniences.
- [ ] Review trait return types to ensure they can participate in the
      `OutputResult` boundary without ad-hoc conversions.

## `osp-api`

Role:
Own concrete adapters for ports.

Checklist:

- [ ] Keep adapters dumb and transport-focused.
- [ ] Do not let API adapters perform CLI parsing or output shaping.
- [ ] Ensure adapter outputs preserve enough structure for non-lossy
      `OutputResult` conversion.
- [ ] Keep auth/session refresh policy outside raw transport adapters unless it
      is an adapter responsibility by design.

## `osp-services`

Role:
Own use-case orchestration and command-level business workflows.

Checklist:

- [x] Stop using DSL entrypoints that can lose grouped output.
- [x] Make services return `OutputResult` where pipelines can change result
      shape.
- [ ] Keep command parsing small and explicit.
- [ ] Avoid embedding compatibility hacks that belong in CLI translation layers.
- [ ] Keep service logic independent of render mode and terminal heuristics.

## `osp-ui`

Role:
Own structural document building, formatting selection, and rendering.

Checklist:

- [ ] Move process/env probing out of this crate.
- [x] Accept fully resolved runtime settings from callers.
- [ ] Keep auto-format selection pure and payload-based.
- [ ] Keep JSON/table/mreg/value rendering free of command-specific branches.
- [ ] Audit clipboard and message formatting paths for hidden runtime policy.
- [x] Add contract tests for deterministic rendering under explicit settings.

## `osp-completion`

Role:
Own completion structures and completion-specific algorithms.

Checklist:

- [ ] Ensure completion reuses the same parser/token rules as the CLI/DSL where
      possible.
- [ ] Remove schema drift between completion metadata and real command syntax.
- [ ] Keep completion nodes typed; avoid magic keys and implicit side channels.
- [ ] Keep dynamic completion augmentation isolated and cache-aware.

## `osp-repl`

Role:
Own interactive loop, history expansion, completion plumbing, and shell state.

Checklist:

- [ ] Keep REPL runtime state separate from config, auth, and UI policy.
- [ ] Reuse the same strict parser and dispatcher semantics as one-shot mode.
- [ ] Avoid reintroducing hidden mutable state that changes command semantics.
- [ ] Keep history expansion explicit and well tested.
- [ ] Ensure REPL shells do not bypass the `OutputResult` rendering contract.

## `osp-cli`

Role:
Own process bootstrap, clap parsing, runtime context, dispatch policy, plugin
ownership, and integration of the other crates.

Checklist:

- [ ] Keep runtime probing and environment interpretation here, not in `osp-ui`.
- [ ] Keep flags in one parser, not in decorator-like wrappers.
- [ ] Remove remaining command grammar hacks where possible instead of
      normalizing them later.
- [ ] Make alias expansion a parser concern only.
- [ ] Enforce the strict DSL defaults from this document.
- [ ] Keep `AppState` as a thin integration root, not a new monolith.
- [x] Audit direct row-based helper paths and move them to `OutputResult` where
      shape can vary.

## Review Cadence

For each crate-level cleanup:

- [ ] Add or update contract tests first.
- [ ] Remove one source of hidden behavior.
- [ ] Simplify the public boundary, not just the implementation.
- [ ] Update this file by checking off the completed items.

## Success Criteria

The plan is working if:

- invalid DSL input fails clearly instead of guessing
- grouped results cannot disappear through helper APIs
- rendering is deterministic from explicit inputs
- config remains strict without becoming ceremony-heavy
- crate boundaries are visible in code, not only in docs
