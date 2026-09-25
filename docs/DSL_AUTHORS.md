# DSL author notes

The execution model is `RowSet`: partitions of rows with group keys and
aggregates kept separately. Ungrouped data has one partition even when empty.
The public `OutputItems::Rows/Groups` variants are presentation boundaries;
they are converted once on entry and once on exit. They must not select
parallel implementations of verbs.

## Ownership and normalization

- `cli/rows/output.rs` unwraps producer-declared collections once. Human column
  hints never rename, remove or convert canonical values. A raw document can
  survive unstaged JSON rendering, but no staged service result retains its
  stale pagination/total envelope.
- `GuideView` owns guide-to-row normalization and reconstruction. Explicitly
  declared guides expose entries/text as rows and keep section/layout metadata
  outside execution. Ordinary service fields never implicitly identify guides.
- `dsl/engine.rs` compiles and dispatches each stage once over `RowSet`.
- Verbs own algorithms over rows/partitions. Group keys and aggregates are
  deliberately metadata, not duplicate fields inserted into members.
- `ui/lower.rs` applies human-only producer field projection and timestamp
  hints. `unix_timestamp_columns` identifies Unix seconds without knowing
  product field names. `display_rules` supports literal string suffixes gated
  by a true boolean (`SuffixWhenTrue`) and blanking gated by another field's
  non-null presence (`BlankWhenPresent`). Conditions always read the original
  row; missing target fields are not synthesized. JSON rendering preserves raw
  values, and filters use canonical values before these display changes.

A stage that changes shape clears curated columns, alignment, numeric timestamp
hints, conditional display rules and renderer recommendations. Guide layout
survives only row-preserving stages; cleanup (`?`), projection, aggregation and
JQ expose their resulting row shape.

## Selector contract

All inputs use the same selector resolver. Bare keys search descendants;
structural paths use addressed traversal. Named segments descend through arrays,
so `a.b` and `a[].b` resolve equivalent leaves. Indexed/sliced selectors retain
original addresses until projection is complete. Missing branches contribute
nothing; selected null and duplicate values are retained.

Filtering retains complete canonical rows. It does not switch to recursive
member pruning when an object has a raw JSON sidecar. Callers that want to
filter members first expose them as rows with a declared collection boundary,
`P collection[]`, or `U collection`.

`P` resolves all keepers/droppers against original addresses, then rebuilds and
compacts sparse arrays. Dynamic projection labels must be unambiguous. `VALUE`
extracts leaves as flat value rows and never reconstructs service envelopes.

## Group semantics

- `G` partitions rows; regrouping operates on existing partitions.
- Row stages map the same algorithm over partitions and preserve metadata.
- `F` tests group headers when its selector resolves there, otherwise filters
  member rows and drops empty groups.
- `S` and `L` operate on group headers/partitions.
- `A` stores aggregate results separately; existing aggregates are snapshots,
  not silently recomputed when later member filters run.
- `C` emits key/count summaries and `Z` emits keys/aggregates.
- Empty `C` and empty `A count` produce one zero summary even after all groups
  have disappeared. They do not invent a group key. Empty `G` remains empty.
- JQ receives the row array, or each explicit group envelope. There is one jaq
  evaluator; its result is normalized at that explicit boundary.

## Parsing and verification

The lexer shares one quote/escape scanner for stage splitting and tokenization.
Quote regex/JQ pipes. One-shot argv separates literal pipe tokens before command
parsing so built-ins and product/plugin commands share operand handling.

Use the existing library, contract, integration and end-to-end suites. Existing
JSON fixtures must enter through the actual command adapter and renderer;
fixture helpers must not reintroduce the removed document executor. Preserve
all existing test functions/coverage when updating intended contracts. Do not
add regression tests. Compile and run checks on the internal builder.

See [DSL.md](DSL.md) for the user contract and [CONFIG.md](CONFIG.md) for explicit
profile selection. Product command vocabulary and server-owned facts belong to
the product/API owner, not to DSL aliases or renderer inference.
