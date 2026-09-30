# Testing and verification

Choose the smallest existing check that exercises the changed behavior.
Run broader checks when the change affects several components or a focused
check leaves an interaction unverified.

Do not add regression tests, including for bug fixes, or remove existing tests
unless explicitly requested. Verify with existing integration, contract,
end-to-end and architecture checks, static checks, and focused manual execution.
Adjust existing tests when the public contract changes. Avoid assertions about
implementation details or behavior already covered by another test.

## Choose a test target

| Existing target | Covers |
| --- | --- |
| `contracts` | Spawned CLI behavior, help, output, exit codes and configuration |
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

When checking saved configuration or machine-readable output, parse it and
assert meaningful values; counts or field presence alone can pass even when
the content is wrong.

Linux contracts include one real native-keyring lifecycle. It requires
`dbus-run-session`, `dbus-daemon`, `gnome-keyring-daemon` and `gdbus`; on
Debian/Ubuntu these come from `dbus-daemon`, `gnome-keyring` and
`libglib2.0-bin`. CI installs them explicitly, and confidence test/contract/
coverage checks fail their prerequisite preflight before compilation when a
tool is missing. Build and publication checks do not require these services.
The runner never installs system packages implicitly.

The contract starts a private D-Bus session and unlocks a throwaway GNOME
keyring under isolated HOME/XDG roots. It executes the real CLI to persist,
reload, redact, reveal, preview, rotate and remove a scoped credential, then
verifies the ordinary-config fallback and the value-free `0600` index.
It preserves `LLVM_PROFILE_FILE` for instrumented child processes. This proves
the Linux Secret Service boundary; it does not verify macOS Keychain, Windows
Credential Manager or availability of a user's desktop keyring. A denied
local socket bind is an environment failure, and missing tools do not skip the
contract.

## Run existing checks

The confidence runner defines command lists, environment isolation and which
checks each lane includes. See [CONTRIBUTING.md](../CONTRIBUTING.md#verification)
for tooling, hooks, CI, coverage and release procedures.

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

Publication rehearsal uses `--check publish-dry-run` and its own
`target/publish-dry-run` directory. Sharing the normal build directory can
leave Cargo fingerprints pointing at `target/package/...` source, allowing a
process test to execute an older packaged binary. If a current source edit
does not rebuild the CLI, inspect the package paths in `target/debug/.fingerprint`.
Run `cargo clean --package osp-cli`, then rebuild and rerun the affected check.
This removes the crate's artifacts while retaining dependency builds.

## Snapshots and public examples

Use `cargo insta review` to review intentional output changes. Keep snapshots
with the tests that exercise the behavior: spawned stdout/stderr in contracts,
terminal flows in e2e, local layout invariants in existing unit checks.

Doctests teach small, stable public entrypoints and copyable usage. The curated
`.public-api-examples.txt` baseline requires complete, nonempty runnable Rust
blocks; the existing doctest target verifies they compile and execute. Do not
inflate examples with private fixtures or state assertions to raise coverage.

## Reporting results

Report which existing checks passed, how you exercised the requested behavior,
and what remains unverified. Full CI checks the revision submitted for merge;
a local hook checks the working tree and may not match the revision pushed.
