# DSL Reference

`osp` commands can be followed by a pipe-based DSL for filtering and reshaping
results.

## Basic Shape

```text
command ... | STAGE ... | STAGE ...
```

Examples:

```bash
osp ldap user alice | P uid mail
osp ldap users | F active=true | S uid | L 10
```

## Common Stages

- `P`: project fields
- `F`: filter rows
- `S`: sort
- `G`: group
- `A`: aggregate
- `L`: limit
- `Z`: collapse grouped output
- `C`: count shorthand
- `VAL` or `VALUE`: extract values
- `JQ`: run a JSON transform
- `Y`: mark output for clipboard copy

Bare quick-search stages are also supported.

## Examples

Project fields:

```text
| P uid mail
```

Filter:

```text
| F uid=alice
| F active=true
| F created_at>=2024-01-01
```

Sort and limit:

```text
| S uid | L 20
```

Group and aggregate:

```text
| G department | A count
```

Value extraction:

```text
| VALUE uid
```

## Paths

Paths support:

- dotted keys
- array selectors such as `[]` and `[idx]`
- nested access such as `interfaces[].mac`

List values use any-match semantics for filters by default.

## Parsing Rules

- shell-style quoting and escaping are supported
- `|` starts a new DSL stage
- malformed quoted pipelines are errors
- unknown verb-shaped stages are errors

Bare search text still works as quick search, but mistyped stage verbs are not
silently treated as searches.

## Output Pipeline

The DSL runs before final formatting. That means the same command can be
rendered differently after the data has already been filtered or projected.

## Streaming

The DSL now keeps flat row pipelines on a streaming path where the stage
semantics allow it.

Stages that typically stream:

- `F`
- `P`
- `VAL` / `VALUE`
- `Y`
- `U`
- `L` when used as a normal head limit such as `| L 20`

Stages that materialize the current payload:

- `S`
- `G`
- `A`
- `C`
- `Z`
- `JQ`
- quick-search style stages such as bare search text, `V`, and `K`

Use `| H` in the REPL to see the current verb list and `| H <verb>` for per-verb
streaming notes.

## Clipboard

The `Y` stage marks the final output for clipboard copy behavior when supported
by the current environment.
