# CLI and Profile Parsing

The CLI uses a positional profile name. Profile replaces the old context
concept.

## Supported forms

- `osp` starts REPL in the default profile.
- `osp <profile>` starts REPL in that profile if it exists.
- `osp <profile> <command...>` runs one-shot in that profile.
- `osp <command...>` runs one-shot in default profile.

## Deterministic parsing rule

1. Read argv after the binary name.
2. If argv is empty, start REPL using default profile.
3. If argv[0] matches a known profile name, set profile = argv[0] and
   drop argv[0] from the args list.
4. If the remaining args list is empty, start REPL.
5. Otherwise, run one-shot command with the selected profile.

## Known profiles

Known profiles come from the config layer and are resolved without any
network calls. No API should be contacted to decide whether a profile
exists.
Profile names are case-insensitive and normalized to lowercase.

## Edge cases

- If a profile name collides with a command name, the profile wins only if it
  is defined in config. Otherwise the token is treated as a command.
- `--profile` is optional. If provided, it overrides positional profile.
  The first positional token is then treated as command input.

## Contract tests

At minimum:

- `osp` -> REPL default profile.
- `osp <profile>` -> REPL with that profile.
- `osp <profile> <cmd>` -> one-shot with that profile.
- `osp <cmd>` -> one-shot default profile.
- `osp <unknown>` -> one-shot default profile with command `<unknown>`.
