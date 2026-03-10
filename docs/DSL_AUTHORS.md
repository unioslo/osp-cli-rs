# DSL Author Notes

This file is for people changing the DSL implementation, not just using it.

The short version is simple:

- Keep syntax meaning stable.
- Keep selector verbs and collection verbs separate.
- Preserve document envelopes first, compact later.
- Treat surprising shape changes as bugs unless the verb is explicitly
  transforming.

## Selector Law

The DSL is easiest to reason about when these rules hold:

- Bare token syntax means permissive descendant matching.
- Path syntax means strict path semantics.
- The same selector should resolve the same addresses across `P`, `F`, quick,
  `?`, and `VALUE`.
- Selector verbs preserve and rebuild structure whenever possible.
- Collection verbs reshape rows or groups on purpose.

Concrete examples:

- `name` is permissive.
- `commands[].name` is exact path traversal.
- `commands[0].name` is exact indexed traversal.
- `metadata.owner` must not silently fall back to a descendant flat-key match.

If a change makes one of those rules fuzzy again, assume it is a regression
until proven otherwise.

## Verb Families

Selector-engine verbs:

- bare quick
- `F`
- `P`
- `V`
- `K`
- `?`
- `VALUE`
- `VAL`
- `U`

Collection-engine verbs:

- `S`
- `G`
- `A`
- `C`
- `Z`
- `L`
- `JQ`

Meta or side-effect verbs:

- `H`
- `Y`

The architectural seam matters. Selector verbs are about addressed matches and
structural rebuild. Collection verbs are about row/group operations. Mixing the
two models inside every verb is how semantic drift comes back.

## Intentional Divergences

Not every verb returns the same shape, but the differences should be deliberate.

- Bare quick on a multi-row set behaves like a row filter.
- Bare quick on a single semantic document narrows to matching branches while
  keeping owning envelopes.
- `V` and `K` only narrow the quick-search scope. They do not change selector
  resolution rules.
- `VALUE` is transforming. It keeps canonical JSON on the semantic path, but
  targeted leaves become `{value: ...}` rows.
- `U` duplicates the nearest owning record once per array member. It is not a
  disguised projection.
- Group-preserving row verbs operate on each group's member rows and leave group
  headers and aggregates intact.

If a divergence is user-visible but not documented here or in the user guide,
it probably is not intentional enough yet.

## Structural Rebuild Rules

These rules are the high-risk area.

- Preserve first, compact after.
- Real `null` is user data and must survive.
- Sparse holes are an internal rebuild detail and must never leak.
- Mixed keepers and droppers must be resolved against original addresses, not
  against already-compacted output.
- Overlapping keepers must merge by address, not by post-compaction position.
- Relative addressed selectors must rebuild the branch they actually matched.

When these rules break, users get wrong-subtree bugs, dropped `null` values, or
surprising aliasing in rebuilt output.

## Row And Value Contracts

Row and semantic execution do not always look identical, but the selector
surface still has to stay coherent.

- Path selectors must mean the same path in `F`, `P`, `VALUE`, and path quick.
- Quoted term parsing must stay shared across `P`, `VAL`, and `VALUE`.
- Row-mode fanout projection must not silently alias colliding labels.
- `VALUE` should keep sibling field identity when extracting multiple leaves
  from the same object.
- Grouped pipelines must preserve group metadata when applying row-oriented
  stages.

## Code Map

Start here when changing semantics:

- `src/dsl/parse/path.rs`
  Path parsing and the structural-token classification rule.
- `src/dsl/verbs/selector.rs`
  Selector-engine split between strict path matching and permissive descendant
  matching.
- `src/dsl/eval/resolve.rs`
  Addressed resolution, path traversal, descendant matching, flat-key hints,
  negative indexes, and slices.
- `src/dsl/verbs/json.rs`
  Structural rebuild, sparse array handling, compaction, and envelope
  preservation.
- `src/dsl/verbs/project.rs`
  Mixed keepers/droppers, row projection, dynamic column behavior, and fanout
  label handling.
- `src/dsl/verbs/values.rs`
  `VALUE` transform rules and shape stability.
- `src/dsl/engine.rs`
  Substrate transitions: row stream, materialized rows/groups, semantic JSON.
- `src/dsl/verb_info.rs`
  Registered verb metadata and help-facing descriptions.

## Regression Matrix

When changing selector semantics, cover at least these cases:

- The same selector in `P`, `F`, quick, `?`, and `VALUE`.
- Bare token versus dotted path versus indexed path.
- Relative paths on nested semantic documents.
- Fanout, slices, negative indexes, and overlapping keepers.
- Literal `null` in arrays and objects.
- Mixed keepers and droppers on the same path family.
- Grouped row stages preserving `groups` and `aggregates`.
- `VALUE` extracting sibling leaves from the same object.
- Row fanout label collisions.
- Collapse and count behavior after grouped pipelines.

Useful existing test files:

- `src/dsl/contract_tests.rs`
- `src/dsl/value.rs`
- `tests/integration/dsl_ported.rs`

If you fix a semantic bug, write the failing regression first and keep it near
the behavior it protects.

## Author Checklist

Before merging a DSL change, ask:

- Did this make a selector surface stricter or fuzzier?
- If it became fuzzier, is that really intended?
- Does the same selector still mean the same thing across selector verbs?
- Did we preserve real `null` and avoid leaking sparse rebuild details?
- Did grouped row stages keep their metadata?
- Did we document the user-visible behavior in `docs/DSL.md`?
- Did we update tests at a stable boundary instead of only the smallest helper?

If the answer to the last two is no, the change is not finished.
