# Contributing

## Engineering approach

Keep rules, mappings, and invariants in one place. Other code should use them
there instead of making the same decision independently. Similar-looking code
can remain separate when it represents different rules.

Choose the simplest design that handles current needs. Add an abstraction when
it removes repeated decisions or supports a real variation. File size alone is
not a reason to split a module: a split should make a likely behavior change
easier to understand and implement. Prefer small, behavior-preserving refactors
to broad rewrites.

Test stable behavior through integration and contract checks. Keep the
end-to-end suite focused, and retain local tests for invariants and failure
paths that broader checks do not exercise.

## Local Tooling

This repo expects `just` for the documented developer commands.

Install it with:

```bash
cargo install just --locked
python3 scripts/confidence.py --install-tools full
```

If `just` is still not found afterwards, make sure `~/.cargo/bin` is on your
`PATH`.

Native `x86_64-unknown-linux-gnu` builds use `lld`, including the UiO product.
Install the linker before running checks; on Ubuntu/Debian:

```bash
sudo apt-get install lld
```

The confidence runner preflights a lane's tooling before builds, including the
active compiler's `llvm-cov` and `llvm-profdata` binaries for coverage. Its
installer ensures `llvm-tools-preview` and pinned audit/coverage helpers; CI
uses that same installer. Other native targets
keep their platform linker.

## Docs Layout

Keep current behavior, contributor guidance, and architecture contracts in
`docs/`. Delete obsolete reviews and superseded plans; Git preserves history.
Temporary planning belongs in `docs/plans/` and must not become a second owner
for live contracts.

## Commit Message Contract

This repository enforces commit messages through `.githooks/commit-msg` and
installs the `.gitmessage` template.

Normal commits must use:

`<type>(<scope>): <Subject>`

- Allowed `type`:
  - `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `style`, `ci`, `build`, `perf`, `revert`
- `scope` is optional. If present, it must start lowercase and may contain
  lowercase letters, digits, `.`, `_`, `/`, and `-`
- `Subject` must start with uppercase, must not end with a period, and must be
  72 characters or fewer
- the line immediately after the subject must be blank before the body starts
- ordinary commits must include a real body
- body lines must wrap at 80 columns or fewer
- do not embed literal `\n` sequences in a single `git commit -m ...`; use the
  editor or multiple `-m` flags instead
- Git-generated `Merge ...`, `Revert ...`, and autosquash `fixup! ...` /
  `squash! ...` subjects are allowed exceptions

Use the installed template:

```text
<type>(<scope>): <Subject>

Why:

What:

Verification:

Refs:
```

Fill `Why`, `What`, and `Verification` with real content; empty headings alone
fail the hook.

Examples:

- `feat(cli): Add profile positional dispatch`
- `fix(config): Normalize profile and terminal keys`
- `ci(test): Pin Rust 1.94 and tighten test guardrails`

## Hook Setup

Run once per clone:

```bash
./scripts/install-git-hooks.sh
```

This sets:

- `core.hooksPath=.githooks`
- `commit.template=.gitmessage`

The commit hook is in `.githooks/commit-msg`.

The pre-commit hook checks staged public docs and runs the static lane. The
pre-push hook runs the local `pre-push` lane. These are contributor conveniences:
they inspect local state, not necessarily the refs or commits being pushed.
CI on the submitted revision remains authoritative.

## Verification

Start with the smallest existing check that exercises the changed behavior; see
[docs/TESTING.md](docs/TESTING.md). Do not add regression tests or remove existing
tests. Run the local lane before a coordinated change is handed off:

```bash
python3 scripts/confidence.py local
```

`scripts/confidence.py` owns the exact command lists. Use its `--list` output
and the selected lane's printed checks instead of maintaining another table.
Run one check with `--check NAME`. Generic checks (`fmt`, `clippy`, `test`,
`build`, `audit`, `metadata`, and `fmt-fix`) accept `--cwd` for a product checkout;
framework lanes stay anchored here. Single-check labels go to stderr so native
stdout, including metadata JSON, remains usable by callers.
`full` adds the existing unit, doctest, terminal, wrapper-example, advisory,
and coverage checks. Its instrumented run executes the covered test targets
once; the faster local lanes remain available for iteration. CI runs `full`
on pull requests and pushes to `main`, with Miri in a separate pinned-nightly
job. The library's example wrapper is checked as an independent crate; root
`--all-targets` alone cannot check its manifest.

Full confidence also builds the release binary and checks its startup/size
budget. The startup limit has a generous fixed floor to avoid millisecond-scale
runner noise. `just startup-baseline` captures an intentional baseline after
`just build`; review the measured receipt rather than updating it to silence a
failure.

The UiO product is a separate repository. Its paired workflow checks the exact
framework revision alongside the product, then runs product checks. A green
framework lane proves the framework contracts, not the product's integrations,
authentication, or live service behavior. Record both revisions for coordinated
changes and distributable builds.

Verification commands that resolve dependencies use `--locked`; intentional
updates to manifests and lockfiles belong in a separate reviewed change.

## Coverage

Coverage is a diagnostic backstop, not a request to accumulate tests or raise
a number after every change. `scripts/coverage.py` owns enforcement and
`.coverage-baseline.json` owns the numeric policy. Review uncovered behavior
and the existing tests before changing tests or policy.

```bash
just cov-gate
just cov-gate-fast
```

CI supplies `COVERAGE_BASE` explicitly so changed-file coverage compares the
submitted revision with its intended base even in detached checkouts. A local
fast report is an approximation and prints its comparison basis. An unavailable
base must not silently turn a changed-file check into a successful empty check.

The overall hard floor is 85%. Changed files below 85% produce explicit review
warnings, not whole-file blockers: unchanged paths and downstream-only APIs can
otherwise force overlapping local tests. Review which public behavior is untested.
Aim for higher coverage through useful contract and integration workflows;
the floor is a minimum, not a target. Keep it fixed unless explicitly reviewed.

Missing entries still fail. The non-executable `src/lib.rs` facade has one
explicit digest-pinned exemption; any byte change requires renewed review.
Files that appear with executable lines are evaluated normally. `pre-push`
runs the local checks without repeating instrumented targets; use the full
lane for authoritative coverage and `cov-gate-fast` for an optional report.

Change the stored baseline only when coverage scope or policy changes
intentionally, after reviewing the full report. `just cov-baseline` is a
maintenance tool, not a routine step or a way to bypass a failed gate.

## Releases

The release workflow checks out the requested tag, validates package/version
agreement and completed notes, runs `full`, dry-runs locked publication, and
publishes the crate through trusted publishing plus its GitHub release notes.
Packaged cross-platform binaries are currently outside that release flow.

Prepare a version with the existing helper:

```bash
just bump patch "Summarize the release"
```

It updates the root manifest/lockfile and prepares the matching changelog and
`docs/releases/vX.Y.Z.md`. Finish their placeholders and review the change.
Before tagging the final clean revision, rehearse the release:

```bash
just release-check
just release-dry
just release-sign
```

`release-check` runs release metadata validation, full confidence, and
`cargo publish --dry-run --locked`. The tag helper checks metadata, worktree
cleanliness, and local/remote tag absence, then creates and pushes an annotated
or signed tag; it does not rerun the full build/test rehearsal. Release CI
validates the tagged revision again. Do not infer release readiness from a
successful local hook or an archive built with verification disabled.
