# DSL execution contract and limits

The DSL executes over canonical rows and explicit group metadata. Commands own
normalization of service envelopes into rows; rendering preferences do not
select a second evaluator. See [DSL.md](DSL.md) for operator syntax and
[src/dsl/engine.rs](../src/dsl/engine.rs) for the public execution boundary.

## Current ownership

- Parsing retains stage classification and source text; compilation owns verb
  plans before execution.
- `RowSet` is the sole execution substrate. Ungrouped input has one partition,
  including empty input; grouping creates named partitions.
- Group keys and aggregates are metadata separate from member rows. Service
  fields named `groups`, `aggregates` or `rows` do not establish group identity.
- Commands may retain a raw document for unstaged rendering. A staged result
  describes transformed rows, with stale service totals/cursors removed.
- Guide output can retain its layout through shape-preserving stages. Structural
  changes fall back to ordinary transformed output.
- Human display rules and render recommendations are separate from canonical
  values used by predicates and transformations.

One execution substrate does not make every verb interchangeable. Grouped
filtering distinguishes group headers from member rows, and grouped `JQ` runs
per partition. These are current contracts, not evidence that a whole-result
query and a per-group query have identical scope.

## Precision and scope limits

Numeric comparison and aggregation use floating point, so large JSON integers
may lose precision. Timestamp comparisons use whole-second resolution.
Do not promise exact large-integer accounting or subsecond ordering.

Addressed quick expressions can prune nested structure while ordinary quick
search filters complete records. Presence, null and truthiness also remain
separate concerns. Use the documented examples and existing public pipeline
contracts for these distinctions; a common substrate alone cannot prove
semantic equivalence.

A bare literal field selector prefers a root field, including its nested leaves.
It falls back to descendant keys only when that root field is absent. Opt-in
fuzzy key search keeps its broader descendant search. Use an addressed path
when the intended location must be explicit.

## Maintenance

Change a semantic rule at its owning parser, selector or verb and inspect all
callers. Verify through existing `apply_output_pipeline` integration contracts
and the relevant CLI/terminal checks; follow [TESTING.md](TESTING.md).
Do not restore a document evaluator, compatibility aliases or a second execution
model to satisfy obsolete review proposals. Extend scope or precision only for
a concrete operator requirement, updating callers and the public contract in
the same change.
