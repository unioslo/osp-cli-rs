# Output and Invocation Flags

These flags affect one command at a time. They work the same way in the CLI
and in the REPL.

## Output Format

Canonical form:

- `--format {json,table,mreg,value,md,auto}`

Convenience aliases:

- `--json`
- `--table`
- `--value`
- `--md`
- `--mreg`

Examples:

```bash
osp ldap user alice --format table
osp ldap user alice --json
```

```text
ldap user alice --format table
ldap user alice --json
```

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

## Stream Separation

Machine-readable output stays on `stdout`.

Messages, warnings, and debug logs stay on `stderr`.
