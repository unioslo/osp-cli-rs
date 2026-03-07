# Design Roadmap

This document is for design questions that are still open, but important
enough that we should stop improvising.

It is intentionally more concrete than `docs/ARCHITECTURE.md`.

- `docs/ARCHITECTURE.md` says what boundaries we currently claim.
- this file says what we recommend doing next, in what order, and what we
  should explicitly avoid

## Executive Summary

Recommended direction for the next stretch of work:

1. keep narrowing `AppState` only at already-stable seams
2. do not grow `osp-services` yet
3. do not try to deduplicate all tree/path walkers at once
4. keep `merge_orch_os_tokens()` as a compatibility shim for now, but do not
   let more domain-specific parsing pile up beside it

If nothing changes, this is the default plan:

- next 1-3 PRs:
  - narrow one more builtin command family away from full `&mut AppState`
  - add a comment near `merge_orch_os_tokens()` that explicitly calls it a
    domain-specific compatibility shim
  - add one small note where duplicated walkers are intentional
- next architectural review point:
  - only revisit `osp-services` after a second real caller wants the same
    service-style execution path

## What We Should Do Next

### 1. `AppState` Narrowing

#### Recommendation

Continue narrowing away from full `&mut AppState`, but only in places where the
smaller dependency surface is already obvious from the code.

#### Good next targets

- builtin command families in:
  - `crates/osp-cli/src/cli/commands/config.rs`
  - `crates/osp-cli/src/cli/commands/doctor.rs`
  - `crates/osp-cli/src/cli/commands/history.rs`
- read-only app helpers in:
  - `crates/osp-cli/src/app/config_explain.rs`
  - `crates/osp-cli/src/repl/surface.rs`
  - `crates/osp-cli/src/repl/presentation.rs`

#### Concrete rule

Use this rule before changing a signature:

- if a helper touches one mutable sub-state and at most one or two read-only
  dependencies, narrow it
- if a function is still true orchestration, keep `AppState`

#### What this should look like

Prefer small concrete bundles like:

- `ConfigCommandContext`
- `DoctorCommandContext`
- `HistoryCommandContext`

Not:

- traits for "something that has config and UI"
- ten independent parameters
- a second god object with a nicer name

#### What not to do

- do not rewrite every command signature in one pass
- do not move `run()` or `run_repl_command()` off `AppState`
- do not narrow signatures if it forces more cloning than the current code

#### Decision

This is approved for incremental work now.

## 2. Should `osp-services` Grow?

### Recommendation

No, not yet.

`osp-services` should stay small until there is a clear reusable use-case layer
that is wanted by more than one caller.

### What it owns today

Today `osp-services` is a small service-style command executor around
LDAP-oriented flows. It is not the main command runtime.

### What it should not absorb now

Do not move these into `osp-services` just to make `osp-cli` smaller:

- clap parsing
- REPL lifecycle
- plugin discovery/dispatch
- output rendering
- terminal/runtime hints
- session-layer/bootstrap orchestration

### Concrete trigger for growth

Grow `osp-services` only if all of these are true:

1. the logic is business/use-case logic, not terminal behavior
2. the same execution path is wanted by at least two callers
3. the code does not need direct REPL or rendering state

### Candidate future moves

These are plausible later, not now:

- backbone-owned domain command execution that uses ports and returns rows
- reusable orchestration around non-plugin service commands

### Decision

Keep `osp-services` small in the next phase of work.

## 3. Tree-Walk And Path-Traversal Duplication

This question exists in at least two places:

- completion traversal in `crates/osp-completion/src/engine.rs` and
  `crates/osp-completion/src/suggest.rs`
- DSL traversal in `crates/osp-dsl/src/eval/resolve.rs`

### Recommendation

Do not open a broad "deduplicate the walkers" project.

Instead:

1. identify the exact rule that is duplicated
2. share only that rule if it is truly identical
3. leave the rest duplicated if the payloads or call sites differ

### Concrete policy

Share the code only if all of these are true:

- the semantic rule is identical
- the tests for the two call sites should stay identical
- the shared helper is simpler to read than the two copies

Do not share if:

- the two walkers carry different payloads
- the two walkers have different failure/reporting behavior
- the shared abstraction needs callbacks or generic traits just to stay alive

### Examples

Good shared extraction:

- slice-index calculation
- one exact tree-navigation primitive
- one exact "active item" filter

Bad shared extraction:

- a generic visitor framework
- a traversal trait hierarchy
- one helper that returns different shapes through callback closures

### Recommended next step

Do not refactor here immediately. First add one explicit code comment where the
duplication is intentional.

Best candidates:

- completion traversal helpers in `engine.rs` / `suggest.rs`
- pair-carrying traversal in `resolve.rs`

### Decision

No broad dedup refactor. Share primitives only when drift is proven.

## 4. Parser / Domain Boundary: `merge_orch_os_tokens()`

### Current problem

`crates/osp-cli/src/pipeline.rs` contains orchestrator-specific normalization:

- `orch provision --os alma 9`
- becomes `orch provision --os alma9`

That behavior may be useful, but it is not generic pipeline parsing.

### Recommendation

Keep it for now as a compatibility shim. Do not build around it.

### Concrete next step

In the next parser touch:

1. add a short comment above `merge_orch_os_tokens()` saying it is a
   domain-specific compatibility rule
2. keep the existing tests that lock current behavior

### When it should move

Move it out of the generic parser only if one of these becomes true:

- more command-specific rewrites appear in `pipeline.rs`
- orchestrator commands gain a clearer parse/normalize owner
- the same normalization is needed outside CLI pipeline parsing

### Where it should move later

Preferred future homes, in order:

1. command-owner normalization
2. service-layer parser for orchestrator command execution
3. plugin-side normalization, if orchestrator verbs become fully plugin-owned

### What not to do

- do not invent a generic normalization framework for one special case
- do not move the logic twice
- do not remove it without replacement while the user-facing behavior still
  matters

### Decision

Keep as shim now. Treat new parser/domain quirks as a signal that ownership
needs to move.

## Phased Roadmap

### Phase A: Safe Local Moves

Do now:

- narrow one more builtin command family away from `AppState`
- comment `merge_orch_os_tokens()` as a compatibility shim
- add one intentional-duplication note for a walker pair

Do not do now:

- new crate
- broad traversal dedup
- parser framework

### Phase B: Re-check Pressure

Revisit after a few more PRs.

Questions to ask:

- are the same state bundles recurring?
- did another parser quirk appear?
- did a duplicated walker need the same fix twice?
- did `osp-cli` gain more business logic instead of orchestration?

### Phase C: Boundary Promotion

Only do this if the pressure is real.

- promote repeated small state bundles into named contexts
- move stable service-style execution into `osp-services`
- move command-specific normalization to the command owner

## Decision Table

| Topic | Recommendation | Next Action | Trigger To Revisit |
|---|---|---|---|
| `AppState` narrowing | yes, incrementally | narrow one more builtin command family | repeated small state bundles appear |
| `osp-services` growth | not yet | leave crate boundary alone | same service execution wanted by 2+ callers |
| walker duplication | share primitives only | add comments/tests, no broad refactor | same semantic fix lands twice |
| `merge_orch_os_tokens()` | keep as shim | document it where it lives | more domain-specific parser rewrites appear |

## Anti-Goals

These are explicitly not recommended next steps:

- "remove `AppState`"
- "move things into `osp-services` so the architecture looks cleaner"
- "deduplicate all walkers"
- "build a generic parser normalization framework"
