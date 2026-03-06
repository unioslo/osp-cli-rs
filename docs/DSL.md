# DSL (Rust Plan, Based on osprov-cli)

This document summarizes how the current osprov-cli DSL works and proposes a
cleaner, more expressive Rust design that preserves user-facing behavior.

## Current Rust DSL Structure (Implemented Foundation)

The `osp-dsl` crate now has an explicit module layout to avoid the old
single-file implementation:

```text
crates/osp-dsl/src/
  lib.rs
  model.rs
  parse/
    mod.rs
    lexer.rs
    pipeline.rs
    key_spec.rs
    path.rs
    quick.rs
  eval/
    mod.rs
    context.rs
    engine.rs
    errors.rs
    flatten.rs
    matchers.rs
  stages/
    mod.rs
    common.rs
    filter.rs
    project.rs
    values.rs
    quick.rs
    copy.rs
```

### Lexer Contract (Now in place)

- Pipeline splitter: `split_pipeline(...)` in `parse/lexer.rs`
  - splits on `|` while respecting quotes and escapes
  - returns stage segments with byte spans
- Stage tokenizer: `tokenize_stage(...)` in `parse/lexer.rs`
  - shell-like tokenization
  - operator token splitting (`<= >= == != < > =`)
  - preserves prefix forms (`==foo`, `!?bar`, `!foo`, `?foo`) as single tokens
  - returns tokens with byte spans

### Runtime Behavior (Current)

- `parse_pipeline(...)` still returns `{ command, stages }` for compatibility
  with `osp-cli`/`osp-services`, but malformed quoted pipelines now fail
  instead of being naively re-split.
- `apply_pipeline(...)` supports existing working verbs: `P`, `F`, `G`, `A`,
  `S`, `C`, `L`, `Z`, quick search (`term`, `K term`, `V term`), plus
  explicit `VAL`/`VALUE` extraction.
- `apply_pipeline(...)` now returns `OutputResult`, and
  `apply_output_pipeline(...)` applies stages to existing row or grouped output
  without flattening it first.
- `Y` copy-hint stage is wired in the engine metadata path.
- Unknown verb-shaped stages now error by default. Bare quick-search stages
  still work, but mistyped DSL verbs such as `| R foo` no longer silently turn
  into quick search.

### Parity Verification (Current)

- Ported regression tests from Python DSL coverage for current Rust-implemented
  behavior (`F` + quick + parser edge cases), placed under:
  - `crates/osp-dsl/tests/ported_python_filter_and_quick.rs`
- Added a Python-reference parity test for parser/tokenizer behavior:
  - `crates/osp-dsl/tests/python_parser_parity.rs`
  - Compares Rust parser/tokenizer output against `osprov-cli` reference
    behavior when Python reference env is available.

## Current DSL Summary (osprov-cli)

### Pipeline model

- The DSL is a pipe-based language: `cmd ... | F ... | S ... | L ...`
- Parsing now rejects malformed quoted pipelines.
- Bare quick-search stages remain available, but unrecognized verb-shaped
  stages no longer fall back to quick search.

### Data model

- Input data is flattened into a single row per item.
- Non-ASCII separators are used:
  - `DICT_SEP = "🦒"` between dict keys
  - `LIST_SEP = "🐧"` between list indices
- Output is `ExecResult { items, meta }`.
- Grouping returns `GroupObject { _groups, _aggregates, _rows }`.

### Core stages

- `F` filter with operators and key prefixes.
- `S` sort by key.
- `G` group by key(s).
- `A` aggregate.
- `P` project keys.
- `L` limit.
- `C` count macro.
- `Z` collapse groups.
- `Y` mark output for clipboard.
- `JQ` JSON transform.
- Quick search stage for bare terms.

### Path semantics

Paths support:

- dotted keys
- list selectors `[]`, `[idx]`, `[start:stop]`, `[::step]`
- fuzzy key matching on segments

List values are “any-match” by default in filters.

### Context and key resolution

The context builds a `key_index` from flattened rows. Matching uses:

- exact matching
- suffix matching by segment
- fuzzy matching when multiple candidates exist

### Notable complexity

- Emoji separators leak into user-visible data and tests.
- Quick search and filter implement overlapping semantics.
- Path parsing and key resolution are split across multiple helpers.
- Streaming is mixed with materialized stages without a clear boundary.

## Goals for a Better Rust DSL

- Same UX with fewer moving parts.
- Deterministic grammar with typed AST.
- Consistent key resolution across all stages.
- Explicit streaming vs materializing stages.
- ASCII-first path syntax by default, with compatibility support.

## Proposed Rust DSL Design

### 1. Grammar and AST

Use a small grammar with a typed AST. Example EBNF:

```
pipeline   := stage ("|" stage)*
stage      := verb spec?
verb       := IDENT | SINGLE_LETTER
spec       := token+

filter     := expr
expr       := term (("&&" | "||") term)*
term       := "!" term
          | "(" expr ")"
          | predicate
predicate  := exists | compare | quick
exists     := "?" path | "!?" path | "exists(" path ")" | "missing(" path ")"
compare    := path op value
op         := "=" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "~"
quick      := value
```

This allows optional Boolean expressions while keeping the legacy syntax.

### 2. Data model

Keep two representations:

- `Row` with nested `serde_json::Value`.
- `KeyIndex` with flattened paths for matching.

Flattening is used only for indexing and matching, not as the primary
data representation.

### 3. Key resolution

Introduce a single `KeyResolver`:

- Builds an index of path segments.
- Supports exact, suffix, and fuzzy matching.
- Errors on ambiguous matches unless a flag allows “best match.”

This avoids duplicated resolution logic across `F`, `P`, `G`, `A`, `S`.

### 4. Stage interface

Stages implement a unified trait:

```
trait Stage {
  fn kind(&self) -> StageKind;
  fn apply(&self, stream: BoxStream<RowOrGroup>) -> BoxStream<RowOrGroup>;
}
```

`StageKind` is one of:

- `Streaming`: can run row-by-row.
- `Materializing`: needs all rows (e.g., sort, group, aggregate).

The pipeline engine chooses when to collect.

### 5. Compatibility layer

Legacy shorthand remains supported:

- Bare terms are treated as `Quick` stages.
- `?path` and `!?path` are rewritten to `exists()` or `missing()`.
- `C` is a macro for `A count | Z`.
- Legacy separators are accepted but not emitted.

### 6. Expressiveness improvements

These are optional, but feasible in Rust:

- Boolean expressions inside `F`.
- `A` supports `as` for aliasing: `A sum(cpu) as cpu_total`.
- `S` supports multiple keys with per-key ordering: `S name,created_at:desc`.
- `P` supports `+` and `-` for explicit include/exclude.
- `U` for unroll lists, replacing implicit flattening.

## Proposed DSL Stages (Rust)

Baseline parity with osprov-cli:

- `F` filter.
- `S` sort.
- `G` group.
- `A` aggregate.
- `P` project.
- `L` limit.
- `C` count macro.
- `Z` collapse.
- `Y` mark for clipboard copy.
- `JQ` JSON transform.
- `H` help.
- `Q` quick search (internal alias for bare terms).

## Parsing and Tokenization

Use a proper tokenizer instead of ad-hoc `shlex`.

Recommended approach:

- `logos` for tokenization.
- `chumsky` or `pest` for parsing.

This yields:

- consistent error messages with positions
- safe handling of quotes and escapes
- easy extension for new syntax

## Execution Pipeline

1. Parse pipeline into AST.
2. Build `KeyResolver` from data rows.
3. Compile stages using the resolver.
4. Execute with streaming or materialization as needed.
5. Return `ExecResult { items, meta }`.

`ExecMeta` should include:

- `grouped`
- `wants_copy`
- `key_index`
- `warnings` for ambiguous keys or lossy coercions

## Non-ASCII Separators

The Rust DSL should default to ASCII separators:

- Dict paths: `.` (e.g., `user.name`)
- List selectors: `[i]`, `[i:j]`, `[]`

For compatibility:

- Accept `🦒` and `🐧` separators if present.
- Normalize them to ASCII during parsing.

## Suggested Rust Crates

- Parser: `logos` + `chumsky` or `pest`.
- JSON: `serde`, `serde_json`.
- Wildcards: `wildmatch` or `globset`.
- Regex: `regex`.
- Unicode: `unicode-width` for alignment in DSL help.

## Test Plan

Start with behavior-first tests:

1. Quick search parity with bare terms.
2. Filter semantics with `?`, `!?`, `=`, `==`.
3. Path selectors with list indexing and slicing.
4. `C` macro and `Z` collapse behavior.
5. Compatibility with non-ASCII separators.

## Migration Strategy

Phase 1:

- Parser and AST with a minimal stage set.
- Implement `F`, `P`, `S`, `L`.

Phase 2:

- Add `G`, `A`, `C`, `Z`.
- Add `Y`, `JQ`.

Phase 3:

- Add Boolean expressions in `F`.
- Add compatibility with non-ASCII separators.
