# Auth And Command Policy

`osp-cli` now has a generic auth and command-policy mechanism in the core
crate.

This document is about the upstream mechanism, not any site-specific auth
model.

Read this file when you are integrating auth or command visibility into a
downstream distribution. If you are just trying to understand why one command
is hidden or blocked at runtime, start with:

- `osp plugins commands`
- `osp plugins doctor`
- the command/provider docs for your downstream distribution

## Scope

Upstream owns:

- generic command-policy types and evaluation
- generic plugin auth metadata in `--describe`
- generic visibility vs runnability handling
- generic UI hints derived from policy metadata

Upstream does not own:

- netgroup lookup
- directory or IdP integration
- site-specific capability vocabularies
- site-specific command authorization decisions

Those belong in downstream distributions such as `osp-cli-uio`.

## Why This Exists

Upstream needs one boring, reusable way to answer two product questions:

- should this command be shown to the user?
- should this command be runnable right now?

That answer must work consistently across:

- command dispatch
- help and command listings
- completion
- REPL overviews

The point is not to define your site's auth model. The point is to keep command
UX from inventing five different visibility rules in five different places.

## Core Model

The upstream split is:

- `AuthState`: host-owned runtime auth surface
- `CommandPolicyContext`: policy-facing projection of host auth state
- `CommandPolicyRegistry`: command-path keyed policy registry
- `CommandAccess`: evaluated result with visibility and runnability

`AuthState` owns the host's live auth/runtime view. `CommandPolicyContext` is
the reduced, evaluation-oriented input derived from it for command policy
decisions.

This keeps credential acquisition, authorization normalization, and command UX
policy as separate concerns.

## What Users Actually Experience

The user-facing effect is simple:

- some commands are visible and runnable
- some commands are visible but blocked
- some commands are hidden entirely

Visible-but-blocked is important. It lets the product show that a command
exists without pretending the current session is allowed to run it.

## Visibility And Runnability

`osp-cli` distinguishes:

- visibility: should the command appear in help, completion, or listings?
- runnability: should execution be allowed right now?

A command can be visible but not runnable, for example when it requires
authentication or capabilities that are not currently present.

The host uses the same access result for:

- command dispatch
- plugin command listings
- completion
- REPL overview and completion surfaces

Command-policy evaluation governs product visibility and general runnability,
but backend and resource-level authorization remains authoritative.

Rule of thumb:

- command policy decides whether the command surface should expose the command
- backend authorization still decides whether the actual operation is allowed

Those are related, but not the same job.

## Plugin Metadata

Plugins can attach auth metadata to `DescribeCommandV1`:

```json
{
  "name": "approve",
  "auth": {
    "visibility": "capability_gated",
    "required_capabilities": ["orch.approval.decide"],
    "feature_flags": ["orch"]
  }
}
```

This metadata is generic. Upstream carries and evaluates it, but does not
define what `orch.approval.decide` means.

## UI Hints

Plugin listings and REPL summaries show compact auth hints such as:

- `[auth]`
- `[cap: orch.approval.decide]`
- `[cap: orch.approval.decide; feature: orch]`

These are meant to improve discovery and operator debugging without embedding
downstream-specific policy logic in the upstream core.

Those hints are intentionally small. They are there to help the operator
understand "why is this gated?" without turning upstream into a site-specific
policy encyclopedia.

## Downstream Use

A downstream distribution should:

1. build its own auth/session model
2. derive capabilities from its own identity sources
3. populate `CommandPolicyContext`
4. register or augment builtin and plugin command policies
5. rely on upstream evaluation for visibility and runnability

That keeps the core reusable while still letting downstream products make auth
first-class.
