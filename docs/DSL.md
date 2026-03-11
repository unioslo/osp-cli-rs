# DSL Guide

`osp` commands can be followed by a small pipe DSL for filtering, reshaping,
grouping, and extracting structured output.

## Basic Shape

```text
command ... | STAGE ... | STAGE ...
```

Examples:

```bash
osp ldap user alice | P uid mail
osp ldap users | F active=true | S !uid | L 10
osp help | VALUE commands[].name
```

## Mental Model

- Bare text like `doctor` is quick search over keys and values.
- Path-shaped selectors like `commands[].name` are strict paths.
- Selector verbs try to preserve structure when they can.
- Collection verbs intentionally reshape rows or groups.
- The DSL runs before final rendering, so the same pipeline can be shown as a
  table, JSON, markdown, or plain values afterward.

In practice:

- `name` is permissive.
- `metadata.owner` means that exact path.
- `members[].uid` means fan out the `members` array and read each `uid`.

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

Example:

```bash
osp ldap users | F active=true | P uid dept amount | S !amount AS num | L 2
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
