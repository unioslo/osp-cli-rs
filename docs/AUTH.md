# Auth And Command Policy

`osp-cli` now has a generic auth and command-policy mechanism in the core
crate.

This document is about the upstream mechanism, not any site-specific auth
model.

## Scope

Upstream owns:

- generic command-policy types and evaluation
- generic plugin auth metadata in `--describe`
- generic visibility vs runnability handling
- generic UI hints derived from policy metadata

Upstream does not own:

- netgroup lookup
- LDAP or IdP integration
- site-specific capability vocabularies
- site-specific command authorization decisions

Those belong in downstream distributions such as `osp-cli-uio`.

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

## Downstream Use

A downstream distribution should:

1. build its own auth/session model
2. derive capabilities from its own identity sources
3. populate `CommandPolicyContext`
4. register or augment builtin and plugin command policies
5. rely on upstream evaluation for visibility and runnability

That keeps the core reusable while still letting downstream products make auth
first-class.
