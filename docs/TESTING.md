# Testing and verification

Start from the behavior being changed and choose the smallest existing check
that crosses its real ownership boundary. One observable promise has one primary
owner; running every tier is not a prerequisite for every edit.

Do not add regression tests, including for bug fixes, or remove existing tests
unless explicitly requested. Verify with existing integration, contract,
end-to-end and architecture checks, static checks, and focused manual execution.
A test adjustment should describe a changed stable contract, not pin a rejected
implementation or add another copy of a promise already owned elsewhere.

## Choose a boundary

| Existing target | Owns |
| --- | --- |
| `contracts` | Spawned CLI behavior, help, output, exit codes and configuration surfaces |
| `integration` | In-process flows across configuration, dispatch, plugins, guides and DSL evaluation |
| `e2e` | Real process/PTY behavior, prompts, completion and multi-command REPL state |
| `architecture` | Active stable/Miri workflow and toolchain alignment |
| `unit` and `src/**/tests*` | Local invariants, parser edges and awkward failure paths |

Prefer contracts or integration for stable behavior. Use existing PTY checks
when terminal behavior matters; a string-render check cannot prove prompt
redraw or live progress. Existing unit tests remain useful when they explain
an invariant the outer contract does not cover.

Read the target fixtures before running a suite. Contract and integration lanes
isolate HOME/XDG roots and ambient `OSP_*` values. Live service requests,
authentication and writes need separate scoped execution evidence; local
fixtures do not establish those effects.

## Run existing checks

The confidence runner owns command lists, environment isolation and lane
omissions. [CONTRIBUTING.md](../CONTRIBUTING.md#verification) owns tooling,
hooks, CI, coverage and release procedures.

```bash
python3 scripts/confidence.py --list
python3 scripts/confidence.py local
```

For a focused existing target:

```bash
python3 scripts/confidence.py --check architecture
python3 scripts/confidence.py --check contracts
python3 scripts/confidence.py --check integration
python3 scripts/confidence.py --check e2e
```

Run the selected check once while code and dependencies remain unchanged.
Broaden validation when a failure, dependency change or unresolved interaction
requires it. Confirm tests actually executed and distinguish environment/tooling
failures from behavior failures. Report commands, results and skipped effects;
test counts alone do not establish readiness.

## Snapshots and public examples

Use `cargo insta review` to review intentional output changes. Keep snapshots
at the boundary that owns the behavior: spawned stdout/stderr in contracts,
terminal flows in e2e, local layout invariants in existing unit checks.

Doctests teach small, stable public entrypoints and copyable usage. The curated
`.public-api-examples.txt` baseline requires complete, nonempty runnable Rust
blocks; the existing doctest target verifies they compile and execute. Do not
inflate examples with private fixtures or state assertions to raise coverage.

## Completion evidence

A change is complete when its owning existing checks pass and the requested
behavior has been executed at the appropriate boundary. Preserve existing
tests, avoid overlapping promises, and record any verification limit. Full CI
is the merge check; a local hook is a convenience and does not establish which
revision or ref was pushed.
