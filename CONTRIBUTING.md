# Contributing

## Engineering Philosophy

This repo is trying to stay simple to reason about, not merely easy to extend
with one more layer.

In practice that means a few things.

We want one clear owner for each important piece of knowledge. If a rule,
mapping, or invariant matters, it should live in one place and other code
should call into that owner instead of quietly rebuilding the same decision.
That is what we mean by:

- centralize facts
- localize effects
- constrain reachability
- remove duplicate decisions

This is also why some files in this repo are intentionally large. A large file
is fine when it is the one place that owns a concept. Splitting code just to
make files smaller is usually a loss if it scatters the truth across multiple
modules.

We try hard to separate "looks easy" from "is actually simple." Hiding
complexity behind a helper, abstraction, trait, or layer can make one callsite
feel nicer while making the system harder to understand and change. We are not
against abstractions, but we want them to earn their keep.

The default bias is:

- choose the boring design over the clever one
- start with the simplest working shape
- add abstractions only for real, current duplication or variation
- prefer duplicated code over duplicated truth
- do small, behavior-preserving refactors instead of big rewrites

A useful rule of thumb for reviews and refactors:

- merge code when the same knowledge is defined twice
- do not merge code just because it looks similar

Said another way: do not DRY out the shape, DRY out the knowledge.

Tests follow the same philosophy. We prefer tests at stable boundaries, keep
the end-to-end suite small and high-signal, and avoid stacks of local tests
that all prove the same thing with slightly different setup.

## Local Tooling

This repo expects `just` for the documented developer commands.

Install it with:

```bash
cargo install just --locked
```

If `just` is still not found afterwards, make sure `~/.cargo/bin` is on your
`PATH`.

## Docs Layout

Keep committed docs user-facing.

- product behavior, usage, contributor-facing architecture, and reference docs belong in `docs/`
- `docs/plans/` is the freezone for planning notes, reviews, migration sequencing, and temporary working material
- keep freezone material under `docs/plans/` instead of mixing it into the user-facing guides

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

The pre-commit hook intentionally stays fast:

- `public-docs.py --staged`
- `confidence.py static`

The pre-push hook runs `confidence.py pre-push`.

For a behavior-first local confidence pass, use:

```bash
python3 scripts/confidence.py local
```

That runs the fast static checks first, then the contract and integration
behavior suites, with the repo-wide public docs contract up front.

For the local merge-guard approximation, use:

```bash
python3 scripts/confidence.py pre-push
```

That adds the fast changed-file coverage guardrail on top of the local lane.

## CI And Releases

GitHub Actions runs two separate lanes:

- `Verify`
  Runs on `pull_request` and pushes to `main`.
  It runs on the pinned Rust `1.94.0` toolchain from `rust-toolchain.toml`.
  It enforces:
  - `python3 ./scripts/confidence.py full`

- `Release`
  Runs when a `v*` tag is pushed.
  It reruns the full confidence lane on Rust `1.94.0`, then dry-runs publish
  and publishes the crate and release notes.
  Cross-platform packaged binaries are currently disabled while that release
  story is on hold.

The release workflow publishes both:

- the `osp-cli` crate to crates.io through trusted publishing
- the GitHub release entry and release notes

Release artifacts are built from the root single-crate package.

Release tags are expected to match the root package version exactly. For
example, if `Cargo.toml` says `0.1.0`, the release tag must be `v0.1.0`.

Typical release flow:

```bash
just bump patch "Summarize the release"
git commit --all
git push origin main
just release-check
just release-dry
just release-sign
```

Use the installed commit template when writing the release commit so the hook
sees a real body.

## Version Bumps

Use the bump helper to advance the package version and create the next release
notes stub:

```bash
just bump patch
just bump patch "Summarize the release"
just bump 1.4.8 "Summarize the release"
just bump minor
just bump major
just bump-dry patch "Preview the next release"
```

This updates:

- root `Cargo.toml` package version
- root package entries in `Cargo.lock`
- `docs/releases/vX.Y.Z.md` if it does not already exist
- `CHANGELOG.md` with a matching version section if it does not already exist

The generated release notes file intentionally contains `TODO` markers. The
generated changelog section also contains placeholders. The release workflow
refuses to publish while those placeholders remain.

## Local Release Rehearsal

Before tagging, run:

```bash
just release-check
```

That enforces the same release prerequisites locally:

- release notes exist for the current package version
- `CHANGELOG.md` has a finished section for the current package version
- the full confidence lane passes
- `cargo publish --dry-run --locked` passes

To create and push the release tag safely, use:

```bash
just release
just release-sign
```

Those helpers:

- re-run release readiness checks
- refuse to create a tag if it already exists locally or on `origin`
- create an annotated tag by default, or a signed tag with `release-sign`

## Coverage Gate

This repository keeps a checked-in coverage baseline in
`.coverage-baseline.json`.

The enforced policy is:

- full root-package line coverage must not drop below the baseline
- changed Rust source files under `src/` must stay at or above `85%`
- tiny files under `20` executable lines are skipped for the per-file rule

Run it manually with:

```bash
just cov-gate
```

For the local pre-push approximation, run:

```bash
just cov-gate-fast
```

Or get the raw workspace summary with:

```bash
just cov
```

Update the stored baseline intentionally with:

```bash
just cov-baseline
```

Why this runs on `pre-push` instead of `pre-commit`:

- a warm full release-path `cargo llvm-cov --all-features` run was about one
  minute locally on March 7, 2026
- even the smaller fast gate still reruns an instrumented test slice, so it is
  materially slower than the staged docs check plus static lane

So the pragmatic policy is:

- keep `pre-commit` fast with `public-docs.py --staged` plus
  `confidence.py static`
- enforce a changed-file coverage approximation through `confidence.py pre-push`
- enforce the full confidence lane plus full coverage in CI and release checks

The local `pre-push` path is intentionally approximate:

- it runs the local behavior-first lane before coverage
- it reruns instrumented `lib`, `bin`, `contracts`, and `integration` targets
- it uses the union of branch, index, worktree, and untracked source changes
- it enforces the changed-file floor, but leaves the full baseline check to CI

## Updating The Baseline

`.coverage-baseline.json` is a policy file, not an automatically refreshed
artifact.

Update it only when:

- coverage improved in a way you want to lock in
- a larger test batch landed and you want that new floor enforced
- the coverage scope changed on purpose and you are resetting the floor

Do not update it just because the gate failed once.

Recommended workflow:

1. Run `just cov` or `just cov-gate` and confirm the new number is real.
2. Run `just cov-baseline` to rewrite the stored overall floor.
3. Review the diff to `.coverage-baseline.json`.
4. Commit that change deliberately, ideally in a coverage-focused commit.
