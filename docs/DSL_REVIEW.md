# DSL implementation review and future proposals

This review follows the executor, selector resolution, individual verbs, and
the existing integration contracts. It distinguishes implementation defects
from language choices that existing tests deliberately preserve. It is a
record of the development fixes committed on 2026-09-14, followed by optional
future proposals. The fixes below are implemented; the V2 architecture section
is not an implementation plan or a prerequisite for committing development
work. Tests were not rerun during this cleanup.

## Assessment

Keep the substantive DSL. Addressed selection, implicit array traversal,
structure-preserving projection, grouping, and an in-process jq escape hatch
are useful operational capabilities. The main architectural cost is that rows,
documents, and groups sometimes assign different meanings to the same verb.

The best simplification is one owner for each semantic decision. Flattening
everything into rows would lose useful document behavior; adding more generic
execution machinery would not resolve the existing ambiguities.

## Defects addressed in this change

| Contract | Previous behavior | Correction |
| --- | --- | --- |
| Presentation does not define evaluation | Mreg/Markdown recommendations promoted one nested row to a document | Only explicit document identity selects document execution |
| Canonical payload has one owner during evaluation | Executor cloned and synchronized document JSON after stages | Keep one payload and its document kind; materialize output at the end |
| Group filtering evaluates the addressed field | Negation succeeded against missing group-header/aggregate fields and retained unrelated members | Resolve header ownership first, otherwise filter member rows |
| Predicates consume their input | `F uid=alice AND active=true` ignored the second predicate | Reject trailing text and suggest chaining filters |
| Missing sort keys stay last | Descending order moved them first | Reverse comparisons only when both keys exist |
| Sorting has a consistent order | Pair-dependent numeric/text fallback admitted comparison cycles | Classify numeric, IP, and text values consistently |
| Scalar-array sorting honors its options | Direction and casts were ignored | Use the first sort key's direction and cast |
| VALUE selector order is stable | Rows were row-major; documents were selector-major | Both use selector-major addressed extraction |
| Bare VALUE extracts object fields | A document containing scalar fields remained unchanged | Emit value rows |
| jq evaluates the current input | Empty rows skipped evaluation, so `JQ 'length'` could not produce zero | Evaluate the empty array |
| Scoped quick search retains complete members | `V doctor` could discard a matched record's other fields | Scope only changes matching, not member preservation |
| Aggregates have a complete grammar | Trailing arguments were ignored; argumentless sum acted like count | Require fields for non-count functions and consume all arguments |
| Invalid timestamps do not become valid comparisons | Impossible dates and malformed seconds were accepted | Validate calendar days and explicit seconds |
| Collection operations preserve unrelated data | Limiting, sorting, grouping and aggregation pruned unrelated nulls and empty containers | Keep those values; explicit cleaning owns removal |
| Service records are not DSL groups | Matching field names triggered group decoding | Ingest services as rows; semantic execution enables group decoding only after grouping |

New regression coverage lives at the `apply_output_pipeline` integration
interface in `tests/integration/dsl/regressions.rs`, with presentation invariance
in the existing semantic integration suite. Existing unit fixtures were
adjusted where they encoded implicit document promotion.

## Remaining language inconsistencies

These need an explicit semantic decision and migration examples, not an
unannounced rewrite of existing tested behavior.

### Negation has two effects

Ordinary quick negation filters records, but negated structural quick can
delete fields. Existing addressed integration tests require that deletion.
The claim that quick search only filters is therefore too broad.

Keep this convenience: `!query` excludes matches and `!path.to.field` removes
the addressed structure. Do not force users to learn a separate projection
verb for ordinary exclusion. Document these two contexts with examples; any
future change needs evidence that it improves real operator tasks.

### Collection scope is implicit

On documents, collection verbs discover nested collections. `L` slices arrays
it reaches; other verbs use heuristics to recognize row/group collections.
The operator has no explicit way to select the collection receiving a stage.

Collection operations now preserve unrelated branches, including nulls and
empty containers. Keep implicit scope as the normal workflow. A future explicit
scope is only justified by a concrete multi-collection task that cannot be
expressed conveniently; it must not become a prerequisite for basic queries.

### Groups need explicit identity

Service ingestion and ordinary semantic collection stages now treat objects
as records regardless of `groups`, `aggregates`, and `rows` field names.
Explicit `G` enables group decoding; pipeline continuations retain that state,
and arbitrary document `JQ` output resets it. The trusted decoder rejects
extra fields rather than dropping them. This is stage-level provenance, not
a fully typed identity for every nested collection; a future payload model
should retain per-collection identity if mixed structural rewrites require it.

Grouped `JQ` currently runs separately on each group and retains group
envelopes; it does not see the entire result once. Grouped `VALUE` preserves
envelopes while document `VALUE` emits flat value rows. Both need explicit
scope/shape contracts. Prefer whole-payload `JQ`; offer per-group execution
only through an explicit scope when it has a concrete use case.

### Presence is different from truthiness

Missing, null, false, zero, and empty collections are not interchangeable.
Truthiness is useful, but calling it existence makes queries surprising.
Give presence and truthiness distinct documented operations and test their
behavior across rows, documents, and groups.

### Numeric and timestamp precision need limits

Numeric comparison and aggregation currently use floating point. Large JSON
integers can lose precision. Timestamp comparison has whole-second resolution,
so fractional seconds do not distinguish otherwise equal instants. V2 should
specify numeric promotion and timestamp precision before choosing a shared
comparison implementation. Exact integers should remain exact.

## V2 architecture

1. Carry one canonical payload with explicit document/row/group identity.
   Rendering recommendations remain separate and cannot select semantics.
2. Keep one addressed selector resolver. Filtering retains complete matching
   records, projection chooses fields, extraction emits values, and collection
   scope chooses where sorting/limiting/grouping operate.
3. Compile a strict stage representation that retains source text and token
   locations. Report stage number, offending expression, and a useful remedy.
   The lexer already has spans; preserve them instead of reconstructing context.
4. Define ordering, presence, truthiness, and empty-input behavior once, then
   make verbs consume those rules where their semantics actually agree.
5. Test through the public pipeline interface with a compact matrix of rows,
   documents, groups, empty/singleton input, nulls, and mixed types. Remove
   test-only alternate execution paths: `value::apply_stage` currently gives
   some unit tests different quick semantics from production execution.

Start v2 with executable examples for scope, negation, grouping, and empty
input. Keep today's concise verbs as aliases where meanings remain unchanged.
Do not add a second full evaluator or a general-purpose scripting runtime.
