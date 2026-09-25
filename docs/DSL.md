# DSL guide

The pipe DSL transforms command output before rendering. Every stage operates
on canonical rows. Human headings use the same field paths as filters; JSON
shows all available fields. The command vocabulary remains owned by each
product; DSL stages do not rename commands or infer service facts.

## One-shot commands

Quote shell pipes so your shell passes them to `osp`:

```bash
osp theme list '|' F id=nord '|' C
osp --json theme list '|' P id '|' S id desc
osp 'theme list | F id=nord | C'
osp theme list '|' H F
osp theme list '|' 'F id=nord' '|' C
```

The same explicit `|` syntax works on built-in, native product and external
plugin commands that support structured output. In the REPL, write ordinary
pipes. `H` displays DSL help before running the command.
CLI help output also accepts stages, for example `osp --help '|' C`.
Shell-completion scripts and version text are not structured command output;
they reject pipe stages explicitly.

For a product task list whose JSON exposes `id` and `status`:

```bash
osp --json orch task list '|' F status=running '|' C
osp orch task list '|' P id,status,created_at '|' S -created_at
```

Use the actual raw field path returned by your command. `status` is not an
alias for `status.name`: use the nested path only when it exists in raw JSON.
A human-only `STALE` suffix does not change the canonical status or imply a
terminal state. Filter the raw `stale` boolean to select stale tasks.

## The row boundary

- An array supplies one row per element. Object elements retain their fields;
  scalar elements become `{value: ...}` rows.
- An object supplies one row, with nested objects and lists intact.
- A command can declare a collection field such as `items` through `row_path`.
  Its elements become rows once, before the pipeline. Filters do not see the
  wrapper's pagination fields.
- Unstaged JSON preserves an explicitly supplied raw document. Applying a
  pipeline produces a JSON array of the resulting rows, including `[]` and
  singleton arrays. Wrapper totals and cursors are discarded; they no longer
  describe the transformed result.
- Structured help declares itself as a guide. Its entries and text lines
  become rows for a pipeline; section headings remain presentation metadata.
  Filtering and limiting operate on those entries, not on a singleton help
  envelope. Human guide rendering restores surviving content into its sections.
  Shape-changing stages, including field cleanup (`?`), instead display the
  resulting rows without retaining the original guide layout.

`F`, bare search, `K`, `V`, and existence searches retain complete matching
rows. They do not recursively remove members from arrays inside a row. For
example, filtering a row by `members[].uid=alice` retains that whole row.
To work on individual members, explicitly extract/unroll them first:

```text
P members[] | F uid=alice | C
U members | F members.uid=alice
```

`P members[]` emits member fields as rows; `U members` keeps parent fields
and replaces the list field with one member per row. `VALUE members[].uid`
emits plain `{value: ...}` rows in document order, preserving duplicates from
distinct addresses.

## Syntax and verbs

| Stage | Syntax and meaning | Example |
| --- | --- | --- |
| Bare search | Case-insensitive text search across keys and values; retains rows | `running` |
| `F` | `F field OP value`; operators `=`, `==`, `!=`, `>`, `>=`, `<`, `<=`, `~` | `F status=running` |
| `P` | `P field[,field...] [!field...]`; keep/drop fields | `P id,status` |
| `S` | `S field [asc\|desc] [AS num\|str\|ip] ...` | `S created_at desc id` |
| `G` | `G field [AS alias] [field ...]`; partition by group keys | `G provider` |
| `A` | `A count\|sum\|avg\|min\|max [field] [AS alias]` | `A sum memory AS total` |
| `L` | `L count [offset]`; negative count selects the tail | `L 10 20`, `L -5` |
| `Z` | Collapse group headers/aggregates into summary rows | `G provider \| A count \| Z` |
| `C` | Count the current selection | `F stale=true \| C` |
| `Y` | Mark output for copying | `P id \| Y` |
| `H` | `H [verb]`; show syntax and examples | `H S` |
| `V` | Value-only quick search | `V running` |
| `K` | Key-only quick search | `K requester` |
| `?` | No operand: remove null, empty string and empty array fields | `?` |
| `?field` | Keep rows with a truthy field; `!?field` selects absence | `?requester` |
| `U` | `U field`; unroll a list while retaining parent data | `U contacts` |
| `JQ` | Run a quoted jq expression | `JQ '.[] \| .id'` |
| `VALUE` / `VAL` | Extract selected fields as value rows; no operand extracts all | `VALUE status.name` |

The existing `VAL` spelling remains supported. No new verb synonyms are added.
Verbs are case-insensitive. An unknown single-letter verb is an error;
ordinary unregistered words are quick-search text.

## Matching and selectors

Quick search supports these prefixes, with or without intervening spaces:

| Prefix | Meaning |
| --- | --- |
| `!text` | Exclude matching rows |
| `=text` | Case-insensitive equality |
| `==text` | Case-sensitive equality |
| `!=text` | Negated case-insensitive equality |
| `%text` | Fuzzy text matching |
| `?field` / `!?field` | Truthy presence / absence |

`K` and `V` restrict the search to keys or values. `K !=field` selects rows
having a key unequal to that name. All whitespace variants of `!=` have the
same interpretation; predicate `F field != value` also permits attached forms.
`F field text` means a contains comparison; `F field=value` means equality.
Array-valued fields match if an element satisfies the predicate.

A bare field selector can match descendant keys. Dotted paths navigate nested
objects; when a named segment reaches an array it visits each member. Thus
`members.uid` and `members[].uid` select the same leaves. Use `[0]`, `[-1]`,
`[1:3]` or `[]` for indexed, tail, sliced or full traversal. Missing branches
contribute no values. Structural paths do not match a literal dotted key inside
an unrelated nested object.

Projection keepers and droppers resolve against original addresses before
compaction. A selected null remains null. Fanout columns use their leaf name;
selecting two fanouts with the same name is an ambiguity error. Static parent
fields repeat alongside projected member rows.

## Sorting, dates and quoting

`S -created_at` and `S created_at desc` mean descending order; the existing
`!created_at` spelling also works. Cast and direction may appear in either
order, for example `S size desc AS num`. A missing sort field in a nonempty
selection is an error. Empty input sorts successfully. Missing values sort
last; numeric-looking values sort numerically under automatic casting.

Ordered filters recognize RFC3339 timestamps and timezone-naive dates/times.
Naive values use the machine's local timezone. Ambiguous or nonexistent local
DST times are rejected with guidance to supply an offset. Prefer explicit
RFC3339 offsets in scripts:

```text
F created_at >= 2026-09-25T10:00:00+02:00
F created_at >= '2026-09-25 10:00:00'
```

Human output formats RFC3339 timestamps in local time with an explicit UTC
offset. Numeric timestamps are formatted only when the producer declares that
field as Unix seconds; arbitrary numbers are not guessed to be dates. Raw JSON
keeps the original values. Shape-changing stages clear producer formatting
hints so an alias cannot inherit an unrelated timestamp format.

Quote regex alternation inside a textual pipeline:

```bash
osp "theme list | F id ~ 'nord|dracula' | P id"
osp theme list '|' F id '~' 'nord|dracula' '|' P id
```

An unquoted `|` is always a pipeline boundary. Ambiguous regex-to-text-search
transitions fail with quoting guidance; use an explicit verb after an unquoted
regex, or quote the regex. Quote whole jq expressions too:

```bash
osp theme list '|' JQ '.[] | .id'
```

JQ receives an array of canonical rows, including singleton objects. Use
`.[0].field` or `.[] | .field`. Scalar results become `value` rows. For grouped
input, JQ receives each `{groups, aggregates, rows}` envelope; `.rows` accesses
members. Returning a replacement envelope updates that group; other results
replace its member rows.

## Groups and empty selections

`G` keeps group keys, aggregate values and member rows distinct. Row operations
such as `P`, `U`, `VALUE`, quick search and clean run within each partition.
`F` tests matching header/aggregate fields first; otherwise it filters members
and removes empty groups. `S` and `L` order/limit groups. `A` adds one aggregate
per group; `Z` emits keys and aggregates as ordinary rows. Previously computed
aggregates remain snapshots until explicitly recomputed.

`C` emits one `{count: N}` row for ungrouped input and one key/count summary per
surviving group. If no groups survive, it emits `{count: 0}` without invented
group keys. `A count` follows the same empty-selection rule. Empty numeric
sum/average are zero; minimum/maximum are null. `G` alone over empty input
produces no groups.

## Human tables

Null and empty arrays render blank; lists render as readable comma-separated
values. Producer column order expresses priority. Narrow terminal tables omit
trailing low-priority columns before clipping the first remaining column.
Declared columns remain stable for zero, one or many rows. Use `--json` for
complete machine data, or `P` to choose the fields needed for your task.
The explicit `table_overflow=none` setting disables width fitting.
