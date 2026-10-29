# Output and Invocation Flags

These flags affect one command at a time and work the same way in the CLI and
in the REPL.

The useful split is:

- format flags change the output shape
- `--source-record` selects the source record before display shaping
- render flags change how that shape is drawn
- verbosity/debug flags change side-channel detail, not the command data itself

Use invocation flags when you want a one-off answer. Use config when you want a
new default.

## Source Record

`--source-record` uses the decoded source record when an integration supplies one
alongside its display document. Rendering, `--json`, pipes and quick search all
use that record, with generic rendering instead of the display document's
columns, format hint or display rules. Without a separate source, the command's
normal data and presentation stay unchanged.

```text
osp COMMAND --source-record --json
COMMAND --source-record | P field
last --source-record --raw --json
```

The REPL saves both documents and the command's selection. `last --source-record`
selects the saved source before replaying the original pipeline. Add `--raw` to
skip that pipeline; new stages still run on the source. Plain `last` retains the
original command's selection, and replay never changes the saved result.

## Output Format

Canonical form:

- `--format {auto,guide,json,csv,tsv,table,md,mreg,value}`

Convenience aliases:

- `--guide`
- `--json`
- `--csv`
- `--tsv`
- `--table`
- `--value`
- `--md`
- `--mreg`

Examples:

```bash
osp plugins list --format table
osp plugins list --json
osp help --format auto
```

```text
plugins list --format table
plugins list --json
help --format auto
```

Format guide:

- `guide`
  - human documents
- `csv` / `tsv`
  - comma-separated / tab-separated rows
- `json`
  - machine-readable payload
- `table`
  - compare many rows
- `mreg`
  - explicit selection renders records as key/value cards, including flat lists
- `value`
  - extract values, one entry per line, preserving original timestamp values
- `md`
  - markdown-friendly output
  - semantic help/intro/reference output
- `auto`
  - let `osp` choose from the result shape

Explicit invocation formats (including `--format auto`) win, followed by an
exact `ui.command_formats.<canonical.command.path>` default, then `ui.format`. One
ordinary record then selects a classic MREG card before human command/plugin
recommendations; remaining shape inference comes last. Machine recommendations
retain their export format. Help/intro documents keep guide layout by default unless
an invocation explicitly requests another format. This applies equally to
JSON, CSV and TSV; machine exports bypass human display shaping.

Automatic TTY tables become MREG cards only when planned cards show more values
per rendered line than the fitted table; ties keep tables.
`ui.table.too_wide=hide` keeps the fitted table and combined hidden/shortened fit summary.
Explicit `--table` and configured `ui.format=table` retain tables. See
[UI.md](UI.md#tables-and-mreg-layout) for all fallback conditions.

If you are unsure, start with `auto` and only force a format when the default
shape is not what you need.

## Rendering

Rendering flags:

- `--render-mode {plain,rich,auto}`
- `--color {auto,always,never}`
- `--no-color`
- `--unicode {auto,always,never}`
- `--ascii`

These change how output is rendered, not the underlying command result.

## Automatic paging

Tall human-readable output from a one-shot command uses `$PAGER` when stdout is
a terminal. The default command is `less -R`. JSON, redirected output, REPL
output, and progress messages are never automatically paged. Explicit `| LESS`
pages any format, including short output; without a terminal it prints normally.

Disable paging persistently with:

```bash
osp config set ui.pager never --save
```

Set it back to the default with `ui.pager auto`.

## Verbosity and Debug

These are also invocation-local:

- `-v/-vv/-vvv`
- `-q/-qq`
- `-d/-dd/-ddd`

See [LOGGING.md](LOGGING.md) for command evidence, error wording and debug levels.

```bash
osp plugins list -v
osp config get ui.verbosity -vv
osp config get ui.verbosity -v -dd
osp plugins list -dd
osp plugins list --json -q
```

## Placement Rules

Invocation flags may appear anywhere before `--`.

These are all valid:

```bash
osp plugins list --json
osp --json plugins list
osp plugins --json list
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
osp plugins list --json
```

Which only affects that invocation.

## Practical Recipes

Force plain JSON for shell scripts:

```bash
osp --format json --render-mode plain plugins list
```

Keep the normal format but suppress color:

```bash
osp --no-color plugins list
```

Render help as a document rather than rows:

```bash
osp help --format auto
```

## Stream Separation

Machine-readable output stays on `stdout`, including failures when JSON output
is in effect. Failures use one envelope:

```json
{"ok":false,"error":{"code":"command_failed","message":"...","details":null},"exit_code":1,"messages":[],"data":null}
```

`error.code` is a stable machine code: `usage_error` for invalid syntax,
`command_error` for process/dispatch errors, or `command_failed` for command
results without a backend code. Backend codes and structured `details` are
preserved. `exit_code` is the process status, `messages` contains visible
command messages, and `data` contains any failure payload. Unavailable details
and data are `null`; absent messages are an empty array.

Messages, warnings, and debug logs stay on `stderr`.

Use `--csv` or `--tsv` for delimited records, with CRLF record endings and
RFC 4180 quoting (double embedded quotes; quote delimiters and newlines).
The header follows column order after `| P`; nested arrays and objects are
compact JSON, and null/missing cells are empty. These formats emit no ANSI
escapes and never use a pager.
