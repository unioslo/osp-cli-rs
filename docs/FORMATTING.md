# Output and Invocation Flags

These flags affect one command at a time. They work the same way in the CLI
and in the REPL.

The useful split is:

- format flags change the output shape
- render flags change how that shape is drawn
- verbosity/debug flags change side-channel detail, not the command data itself

Use invocation flags when you want a one-off answer. Use config when you want a
new default.

## Output Format

Canonical form:

- `--format {auto,guide,json,table,mreg,value,md}`

Convenience aliases:

- `--guide`
- `--json`
- `--table`
- `--value`
- `--md`
- `--mreg`

Examples:

```bash
osp ldap user alice --format table
osp ldap user alice --json
osp help --guide
```

```text
ldap user alice --format table
ldap user alice --json
help --guide
```

Format guide:

- `json`
  - machine-readable payload
- `table`
  - compare many rows
- `mreg`
  - scan one object in key/value style
- `value`
  - extract one compact field/value surface
- `md`
  - markdown-friendly output
- `guide`
  - semantic help/intro/reference output
- `auto`
  - let `osp` choose from the result shape

If you are unsure, start with `auto` and only force a format when the default
shape is not what you need.

## Rendering

Rendering flags:

- `--mode {plain,rich,auto}`
- `--color {auto,always,never}`
- `--no-color`
- `--unicode {auto,always,never}`
- `--ascii`

These change how output is rendered, not the underlying command result.

## Verbosity and Debug

These are also invocation-local:

- `-v/-vv`
- `-q/-qq`
- `-d/-dd/-ddd`

`-v/-q` control user-facing detail. `-d` controls developer diagnostics on
`stderr`.

Examples:

```bash
osp ldap user alice -v
osp ldap user alice -dd
osp ldap user alice --json -q
```

## Provider Selection

Use `--plugin-provider <plugin-id>` when multiple plugins provide the same
command and you want to choose the provider for one invocation.

## REPL Cache

`--cache` is supported only in the REPL.

It reuses a successful external command result and reapplies the current pipe
and output rendering. This is useful when the backend is slow and you want to
run multiple pipelines against the same response.

## Placement Rules

Invocation flags may appear anywhere before `--`.

These are all valid:

```bash
osp ldap user alice --json
osp --json ldap user alice
osp ldap --json user alice
```

After `--`, remaining tokens are passed through literally.

## Important Rules

- invocation flags affect only the current command
- they do not write into config
- persistent defaults belong in config
- conflicting format aliases are errors

Example default:

```bash
osp config set ui.format json --save
```

That sets the default. It is different from:

```bash
osp ldap user alice --json
```

Which only affects that invocation.

## Practical Recipes

Force plain JSON for shell scripts:

```bash
osp --format json --mode plain ldap user alice
```

Keep the normal format but suppress color:

```bash
osp --no-color ldap user alice
```

Ask for help/guide output in guide form:

```bash
osp help --guide
```

## Stream Separation

Machine-readable output stays on `stdout`.

Messages, warnings, and debug logs stay on `stderr`.
