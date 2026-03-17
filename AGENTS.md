Centralize facts. Localize effects. Constrain reachability. Remove duplicate decisions.

## Engineering Heuristics (99 Bottles + Grug)

### Core principle
- Prefer understandable code over speculative flexibility.
- Complexity is a cost; introduce it only with a concrete, current need.

### Default decision rules
- Start with the simplest working design ("shameless green").
- Do not add abstractions "for later" without a real change request.
- If unsure between clever and boring, choose boring.
- Prefer 80/20 solutions that deliver most value with less moving parts.
- Optimize for local clarity and operability, not architectural novelty.

### Abstraction policy
- Add an abstraction only when duplication or variation is proven in real code.
- Require at least one concrete caller/use case before introducing generic APIs.
- Keep interfaces narrow; hide complexity behind small boundaries.
- Delay factoring until stable cut points emerge.

### Merge policy
- Merge when:
  the same rule is defined in multiple places
  the same policy is being decided more than once
  the same data mapping is repeated
  the same invariant must stay synchronized
  there is one clear owner for the shared concept
- Do not merge when:
  the code is only textually similar
  the shared function would need flags or modes
  the abstraction would accept many loosely related parameters
  the merged version would know too much about multiple callers
  the two cases are likely to evolve differently
  the abstraction makes control flow harder to see
  the abstraction reduces duplication but increases coupling
- Don't DRY out the shape; DRY out the knowledge.
- Prefer duplicated code to duplicated truth.
- This is the heart of it.

### Refactoring policy
- Keep refactors small, reversible, and behavior-preserving.
- Preserve system behavior at every step; avoid "big bang" rewrites.
- Apply Chesterton's Fence: understand why existing code exists before replacing it.
- Prefer incremental improvements over broad redesign.

### Testing policy
- Prefer integration tests at stable boundaries.
- Keep a small, high-signal end-to-end suite.
- Use unit tests where they add speed and focus, not by dogma.
- For bugs: write a failing regression test first, then fix.
- Hold doctests to a higher bar than ordinary tests.
- Add a doctest only when it teaches a real public contract or a copyable
  usage pattern.
- Keep doctests small, stable, and reader-oriented; if an example needs heavy
  fixtures, many branches, or internal state assertions, it should usually be
  a unit or integration test instead.
- Use doctests for public API behavior, import-path correctness, and simple
  end-to-end examples.
- Use unit tests for edge cases, private helpers, branch-heavy logic, state
  machines, and awkward failure paths.

### Avoid low-signal tests and docs
- Do not add many tests that only vary setup while proving the same promise;
  merge them into one table-driven test or promote the promise to
  integration/contract/e2e.
- Once an outer-boundary test owns a user-visible promise, delete overlapping
  local tests and keep only the local invariant tests that still add signal.
- Do not use doctests as hidden coverage storage; if the example is heavy,
  branchy, fixture-driven, or about internal state, it should be a unit or
  integration test instead.
- Do not add comments or docstrings that restate names or narrate syntax;
  document why the code exists, what it owns, key invariants, and important
  limits.

### Comments and docstrings
- Comments explain why a choice exists, not what the code syntax does.
- Document tradeoffs, invariants, failure modes, and operational constraints.
- If code is self-evident, remove noise comments.
- Hold public docs to a high standard: explain why the API/module exists, what
  purpose it serves, how it works at a high level, and what it is allowed to
  depend on or own.

### Dependency and tooling policy
- No new libraries by default.
- Add dependencies only for clear, repeated pain with measurable gain.
- Prefer standard library and existing project components first.
- Invest in debugging/logging quality before adding framework complexity.

### PR acceptance checklist
- Is this simpler than the previous version?
- Does this reduce or cap complexity?
- Are tests aligned with stable behavior?
- Are operational failure modes visible in logs/metrics?
- Could this be done with fewer abstractions?

### Commit message contract (enforced in this repo)
- The installed hook enforces: `<type>(<scope>): <Subject>`
- Allowed `type`: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`,
  `style`, `ci`, `build`, `perf`, `revert`
- `scope` is optional; if present it must be lowercase and may use
  letters/digits plus `.`, `_`, `/`, and `-`
- `Subject` must start with uppercase, must not end with a period, and the
  full subject line must be 72 characters or fewer
- The line immediately after the subject must be blank before the body starts
- Bodies are required for ordinary commits; placeholder headings alone do not
  satisfy the hook
- Body lines must wrap at 80 columns
- Do not embed literal `\n` sequences in a single `-m`; use the editor or
  multiple `-m` flags instead
- Git-generated `Merge ...`, `Revert ...`, and autosquash `fixup! ...` /
  `squash! ...` subjects are allowed exceptions

Examples:
- `feat(cli): Add profile positional dispatch`
- `fix(config): Normalize profile and terminal keys`
- `ci(test): Pin Rust 1.94 and tighten test guardrails`

Commit template:

`<type>(<scope>): <Subject>`

`<blank line>`

`Why:`
`What:`
`Verification:`
`Refs:`

When filling the template, write real content under `Why`, `What`, and
`Verification`; do not leave them as empty headings.
