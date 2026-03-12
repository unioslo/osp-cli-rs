# DSL Guide

`osp` commands can be followed by a small pipe DSL for filtering, reshaping,
grouping, and extracting structured output.

This document is about practical use, not parser internals.

The DSL is most useful when the command already gives you roughly the right
data and you just need to ask a smaller question:

- keep only matching rows
- keep only the fields you care about
- sort or limit the result
- extract one field as plain values
- inspect help/guide output without writing a special command

The point is to move small, local output shaping to the client side instead of
adding another command flag for every little reporting need.

## Broad-Strokes Flow

```text
command output
  ↓
optional DSL pipeline
  ↓
smaller / reordered / reshaped structured output
  ↓
normal format selection and rendering
```

The last line matters: the DSL runs before rendering. You can use the same
pipeline and still ask for `json`, `table`, `md`, `mreg`, or `value` output.

## Five-IQ Recipes

Keep matching rows:

```text
| F active=true
```

Keep only a few fields:

```text
| P uid mail
```

Sort rows:

```text
| S uid
```

Take the first few:

```text
| L 10
```

Turn one field into a simple value list:

```text
| VALUE uid
```

If you only remember five things about the DSL, remember those.

## Basic Shape

```text
command ... | STAGE ... | STAGE ...
```

Examples:

```bash
osp ldap user alice | P uid mail
osp plugins commands | P name provider about | S name | L 10
osp help | VALUE commands[].name
```

## Mental Model

- Row-shaped commands return similar objects, such as users, hosts, or plugin
  command rows.
- Document-shaped commands return semantic structures, such as help and intro
  output.
- Selector-style stages try to preserve structure when they can.
- Collection-style stages intentionally reshape row/group data.
- Bare text like `doctor` is quick search over keys and values.
- Path-shaped selectors like `commands[].name` are strict paths.
- The DSL runs before final rendering, so the same pipeline can be shown as a
  table, JSON, markdown, or plain values afterward.

In practice:

- `name` is permissive.
- `metadata.owner` means that exact path.
- `members[].uid` means fan out the `members` array and read each `uid`.

## Choosing The Smallest Useful Stage

Use the dumbest stage that answers your question:

| Need | Stage |
|---|---|
| "show me rows/documents mentioning this thing" | bare quick search |
| "keep only rows matching a condition" | `F` |
| "keep only these fields" | `P` |
| "sort the result" | `S` |
| "show fewer rows" | `L` |
| "group before rendering" | `G` |
| "I only want the values of one field" | `VALUE` |

That keeps pipelines readable. If a pipeline becomes clever, it usually becomes
hard to trust.

## Example Inputs

The examples below reuse two small inputs so you can compare stages directly.

Row-shaped input:

```json
[
  {
    "uid": "alice",
    "dept": "ops",
    "active": true,
    "amount": 120,
    "roles": ["eng", "ops"],
    "interfaces": [
      {"mac": "aa:bb", "speed": 1000},
      {"mac": "cc:dd", "speed": 100}
    ]
  },
  {
    "uid": "bob",
    "dept": "eng",
    "active": false,
    "amount": 80,
    "roles": ["eng"],
    "interfaces": [
      {"mac": "aa:bb", "speed": 1000}
    ]
  },
  {
    "uid": "carol",
    "dept": "ops",
    "active": true,
    "amount": 90,
    "roles": ["ops"],
    "interfaces": []
  }
]
```

Guide-shaped input:

```json
{
  "usage": ["osp help [topic]"],
  "commands": [
    {"name": "help", "short_help": "Show command overview"},
    {"name": "doctor", "short_help": "Run diagnostics"},
    {"name": "theme", "short_help": "Manage themes"}
  ]
}
```

## Chaining With Pipes

Pipelines are read left to right.

Example on the row-shaped input above:

```text
| F active=true | P uid dept amount | S !amount | L 2
```

Using the row input above, the result is:

```json
[
  {"uid": "alice", "dept": "ops", "amount": 120},
  {"uid": "carol", "dept": "ops", "amount": 90}
]
```

Another example on structured help output:

```bash
osp help | P commands[].name | VALUE name
```

Result:

```json
[
  {"value": "help"},
  {"value": "doctor"},
  {"value": "theme"}
]
```

## Row Data Vs Structured Documents

Many commands produce ordinary row sets. Those behave like a table even before
you render them.

Some commands, especially help/guide surfaces, produce structured documents.
The DSL still works on those, but selector-style stages are more important
because the useful thing is often nested.

That is why these both make sense:

```bash
osp plugins commands | P name provider
osp help | P commands[].name | VALUE name
```

Same pipeline language, different output shape, same idea: keep only the part
you actually need.

## Verb Examples

### Bare Quick Search

Pipeline:

```text
| ops
```

Input:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "bob", "dept": "eng"},
  {"uid": "carol", "dept": "ops"}
]
```

Output:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "carol", "dept": "ops"}
]
```

On a structured document, bare quick keeps the matching parent object instead
of flattening everything:

Pipeline:

```text
| doctor
```

Input:

```json
{
  "commands": [
    {"name": "help", "short_help": "Show command overview"},
    {"name": "doctor", "short_help": "Run diagnostics"}
  ]
}
```

Output:

```json
{
  "commands": [
    {"name": "doctor", "short_help": "Run diagnostics"}
  ]
}
```

### `F` Filter Rows Or Structure

Pipeline:

```text
| F active=true
```

Input:

```json
[
  {"uid": "alice", "active": true},
  {"uid": "bob", "active": false},
  {"uid": "carol", "active": true}
]
```

Output:

```json
[
  {"uid": "alice", "active": true},
  {"uid": "carol", "active": true}
]
```

Path filters work on structured documents too:

Pipeline:

```text
| F commands[].name=doctor
```

Input:

```json
{
  "commands": [
    {"name": "help", "short_help": "Show command overview"},
    {"name": "doctor", "short_help": "Run diagnostics"}
  ]
}
```

Output:

```json
{
  "commands": [
    {"name": "doctor", "short_help": "Run diagnostics"}
  ]
}
```

Supported comparison operators:

- `=` or `==`
- `!=`
- `>`
- `>=`
- `<`
- `<=`
- `~` for regex

Examples:

```text
| F uid=alice
| F amount>=100
| F uid ~ ^a
| F ?mail
```

### `P` Project Fields

Pipeline:

```text
| P uid dept
```

Input:

```json
[
  {"uid": "alice", "dept": "ops", "amount": 120},
  {"uid": "bob", "dept": "eng", "amount": 80}
]
```

Output:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "bob", "dept": "eng"}
]
```

Exact path projection:

Pipeline:

```text
| P commands[].name
```

Input:

```json
{
  "usage": ["osp help [topic]"],
  "commands": [
    {"name": "help", "short_help": "Show command overview"},
    {"name": "doctor", "short_help": "Run diagnostics"}
  ]
}
```

Output:

```json
{
  "commands": [
    {"name": "help"},
    {"name": "doctor"}
  ]
}
```

Droppers can remove fields from the kept result:

```text
| P uid dept amount !amount
```

### `S` Sort

Pipeline:

```text
| S !amount AS num
```

Input:

```json
[
  {"uid": "alice", "amount": 120},
  {"uid": "bob", "amount": 80},
  {"uid": "carol", "amount": 90}
]
```

Output:

```json
[
  {"uid": "alice", "amount": 120},
  {"uid": "carol", "amount": 90},
  {"uid": "bob", "amount": 80}
]
```

Notes:

- Prefix a key with `!` for descending order.
- `AS num`, `AS str`, and `AS ip` force a cast.
- Missing values sort last.

### `G` Group

Pipeline:

```text
| G dept
```

Input:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "bob", "dept": "eng"},
  {"uid": "carol", "dept": "ops"}
]
```

Output, shown in the grouped JSON shape:

```json
[
  {
    "groups": {"dept": "ops"},
    "aggregates": {},
    "rows": [
      {"uid": "alice", "dept": "ops"},
      {"uid": "carol", "dept": "ops"}
    ]
  },
  {
    "groups": {"dept": "eng"},
    "aggregates": {},
    "rows": [
      {"uid": "bob", "dept": "eng"}
    ]
  }
]
```

Fanout grouping is allowed:

```text
| G roles[]
```

Aliasing is allowed:

```text
| G dept AS department
```

### `A` Aggregate

Pipeline:

```text
| G dept | A sum(amount) AS total
```

Input:

```json
[
  {"uid": "alice", "dept": "ops", "amount": 120},
  {"uid": "bob", "dept": "eng", "amount": 80},
  {"uid": "carol", "dept": "ops", "amount": 90}
]
```

Output, again shown in grouped JSON shape:

```json
[
  {
    "groups": {"dept": "ops"},
    "aggregates": {"total": 210.0},
    "rows": [
      {"uid": "alice", "dept": "ops", "amount": 120},
      {"uid": "carol", "dept": "ops", "amount": 90}
    ]
  },
  {
    "groups": {"dept": "eng"},
    "aggregates": {"total": 80.0},
    "rows": [
      {"uid": "bob", "dept": "eng", "amount": 80}
    ]
  }
]
```

Supported aggregate functions:

- `count`
- `sum(field)`
- `avg(field)`
- `min(field)`
- `max(field)`

### `L` Limit

Pipeline:

```text
| L 2
```

Input:

```json
[
  {"uid": "alice"},
  {"uid": "bob"},
  {"uid": "carol"}
]
```

Output:

```json
[
  {"uid": "alice"},
  {"uid": "bob"}
]
```

Offset form:

```text
| L 2 1
```

Result:

```json
[
  {"uid": "bob"},
  {"uid": "carol"}
]
```

### `Z` Collapse Grouped Output

Pipeline:

```text
| G dept | A count AS count | Z
```

Input:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "bob", "dept": "eng"},
  {"uid": "carol", "dept": "ops"}
]
```

Output:

```json
[
  {"dept": "ops", "count": 2},
  {"dept": "eng", "count": 1}
]
```

`Z` only works after grouped output exists.

### `C` Count

Pipeline:

```text
| C
```

Input:

```json
[
  {"uid": "alice"},
  {"uid": "bob"},
  {"uid": "carol"}
]
```

Output:

```json
[
  {"count": 3}
]
```

On grouped input, `C` produces one summary row per group:

Pipeline:

```text
| G dept | C
```

Output:

```json
[
  {"dept": "ops", "count": 2},
  {"dept": "eng", "count": 1}
]
```

### `Y` Mark Output For Copy

Pipeline:

```text
| Y
```

Input:

```json
[
  {"uid": "alice"},
  {"uid": "bob"}
]
```

Visible output:

```json
[
  {"uid": "alice"},
  {"uid": "bob"}
]
```

`Y` does not change the data. It marks the final rendered output for clipboard
copy when the current environment supports it.

### `H` Show DSL Help

`H` is a help stage rather than a data stage.

Pipeline:

```text
| H
```

Example output:

```text
F       Filter rows
P       Project columns
S       Sort rows
G       Group rows
...
```

Per-verb help:

```text
| H F
| H VALUE
```

### `V` Value-Only Quick Search

Pipeline:

```text
| V ops
```

Input:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "bob", "dept": "eng"},
  {"uid": "carol", "dept": "ops"}
]
```

Output:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "carol", "dept": "ops"}
]
```

`V` only searches values, not keys.

### `K` Key-Only Quick Search

Pipeline:

```text
| K uid
```

Input:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"name": "bob", "dept": "eng"}
]
```

Output:

```json
[
  {"uid": "alice", "dept": "ops"}
]
```

`K` only searches keys, not values.

### `?` Clean Or Exists Filter

With no argument, `?` removes empty values and drops empty rows.

Pipeline:

```text
| ?
```

Input:

```json
[
  {"uid": "alice", "mail": "", "tags": [], "note": null},
  {"mail": "", "tags": [], "note": null}
]
```

Output:

```json
[
  {"uid": "alice"}
]
```

With a selector, `?` becomes an exists filter:

Pipeline:

```text
| ? uid
```

Input:

```json
[
  {"uid": "alice"},
  {"mail": "bob@example.org"}
]
```

Output:

```json
[
  {"uid": "alice"}
]
```

### `U` Unroll A List Field

Pipeline:

```text
| U interfaces
```

Input:

```json
[
  {
    "uid": "alice",
    "interfaces": [
      {"mac": "aa:bb", "speed": 1000},
      {"mac": "cc:dd", "speed": 100}
    ]
  },
  {
    "uid": "bob",
    "interfaces": [
      {"mac": "aa:bb", "speed": 1000}
    ]
  }
]
```

Output:

```json
[
  {"uid": "alice", "interfaces": {"mac": "aa:bb", "speed": 1000}},
  {"uid": "alice", "interfaces": {"mac": "cc:dd", "speed": 100}},
  {"uid": "bob", "interfaces": {"mac": "aa:bb", "speed": 1000}}
]
```

Common follow-up:

```text
| U interfaces | P uid mac speed | G mac | A count AS count
```

### `JQ` Run A jq Expression

Pipeline:

```text
| JQ 'map({uid, dept})'
```

Input:

```json
[
  {"uid": "alice", "dept": "ops", "amount": 120},
  {"uid": "bob", "dept": "eng", "amount": 80}
]
```

Output:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "bob", "dept": "eng"}
]
```

`JQ` sees the full current payload, not one row at a time.
It is implemented in-process with
[jaq](https://github.com/01mf02/jaq), so treat it as jq-like rather than a
bit-for-bit promise of external `jq`.

### `VAL` / `VALUE` Extract Values

`VAL` and `VALUE` are aliases.

Pipeline:

```text
| VALUE uid
```

Input:

```json
[
  {"uid": "alice", "dept": "ops"},
  {"uid": "bob", "dept": "eng"}
]
```

Output:

```json
[
  {"value": "alice"},
  {"value": "bob"}
]
```

Path extraction:

Pipeline:

```text
| VALUE commands[].name
```

Input:

```json
{
  "commands": [
    {"name": "help", "short_help": "Show command overview"},
    {"name": "doctor", "short_help": "Run diagnostics"}
  ]
}
```

Output:

```json
[
  {"value": "help"},
  {"value": "doctor"}
]
```

## Selectors And Paths

Quoted term lists behave the same in `P`, `VAL`, and `VALUE`:

```text
| P "display,name" "team ops"
| VALUE "display,name"
```

Path syntax supports:

- dotted fields like `metadata.owner`
- fanout like `members[]`
- indexes like `members[0]`
- negative indexes like `members[-1]`
- slices like `members[:2]`

Important rule:

- Bare tokens are permissive descendant selectors.
- Dotted or indexed selectors are strict path selectors.

That means `owner` and `metadata.owner` are intentionally different surfaces.

## Parsing Rules

- `|` starts a new stage
- commas and whitespace both separate terms in `P` and `VALUE`
- quotes keep embedded commas or spaces together
- malformed quoting is an error
- mistyped verb-like stages are errors, not silent quick search

Examples:

```text
| P uid,mail
| P "display,name" 'team ops'
| F note="a=b>=c"
```

## Streaming Notes

Stages that usually stream on flat rows:

- `F`
- `P`
- `VALUE`
- `VAL`
- `Y`
- `U`
- bare quick search
- `V`
- `K`
- `?`
- `L` in ordinary head-limit form like `| L 20`

Stages that materialize the current payload:

- `S`
- `G`
- `A`
- `C`
- `Z`
- `JQ`

Use `| H` in the REPL to see the current verb list and `| H <verb>` for
per-verb notes.
