# MVP: LDAP in REPL (With DSL, No Login)

This is the first working version. It only needs LDAP in the REPL with DSL
support and visual parity with osprov-cli. Authentication is deferred.

## Scope

**In scope**

- REPL mode only.
- `ldap user <uid>` command.
- `ldap netgroup <name>` command.
- DSL pipes after LDAP output.
- Rich output with the same look and auto-format behavior as osprov-cli.
- Anonymous LDAP bind (no login flow).

**Out of scope**

- Auth/login, tokens, sudo.
- Other command groups (mreg, nh, orch, config, etc.).
- Non-LDAP APIs.

## CLI Examples (Parity Targets)

These are the parity commands to match behavior:

- `osp -u oistes ldap user oistes`
- `osp -u oistes ldap user oistes --json`
- `osp -u oistes ldap netgroup ucore`
- `osp -u oistes ldap netgroup ucore --json`

REPL equivalents:

- `ldap user oistes`
- `ldap user oistes | P uid,cn`
- `ldap netgroup ucore | P members | V`

## Output Expectations

## Cross-check Against osprov-cli

Use the existing Python CLI to validate output shape and formatting:

- `./osprov-cli/.venv/bin/osp -u oistes ldap user oistes`
- `./osprov-cli/.venv/bin/osp -u oistes ldap user oistes --json`
- `./osprov-cli/.venv/bin/osp -u oistes ldap netgroup ucore`
- `./osprov-cli/.venv/bin/osp -u oistes ldap netgroup ucore --json`

These commands are the baseline for visual parity.


- Output is always a list of rows (even when only one row is returned).
- Auto format rules must match osprov-cli:
  - Single row -> MREG-style layout.
  - Multiple rows -> table.
  - `--json` -> JSON array.

The user expects the output to look as nice as the Python version.


## Global Flags (MVP)

- `-u/--user <name>` sets the active identity for the session.
- If `ldap user` is called without a positional uid, it should default to
  the active user name.

## LDAP Command Surface (MVP)

**ldap user**

- Arguments:
  - `uid` (positional)
- Options:
  - `--json` (legacy) or `--format json`
  - `--format`, `--mode`, `--color`, `--unicode` (global formatting)

**ldap netgroup**

- Arguments:
  - `name` (positional)
- Options:
  - `--json` (legacy) or `--format json`
  - `--format`, `--mode`, `--color`, `--unicode`

We can add filtering/attributes later.

## LDAP Result Shapes (Mock Contract)

The mock client should return these keys at minimum.

**ldap user** row keys:

- `uid` (string)
- `cn` (string)
- `objectClass` (list)
- `uidNumber` (string or int)
- `gidNumber` (string or int)
- `homeDirectory` (string)
- `loginShell` (string)
- `eduPersonAffiliation` (list)
- `uioAffiliation` (string)
- `uioPrimaryAffiliation` (string)
- `netgroups` (list)
- `filegroups` (list)

**ldap netgroup** row keys:

- `cn` (string)
- `description` (string)
- `objectClass` (list)
- `members` (list)

## Mocking Strategy

- Use an in-memory LDAP client for MVP.
- Load fixture data from static JSON or Rust structs.
- Keep deterministic ordering (sort list fields if needed).

## Minimal Test Matrix (TDD)

- REPL accepts `ldap user oistes` and prints a single-row MREG layout.
- REPL accepts `ldap netgroup ucore` and prints a single-row MREG layout.
- DSL works: `ldap user oistes | P uid,cn` only shows those fields.
- JSON output works: `--format json` returns a JSON list.
- `--mode plain` forces plain rendering.

## Notes

- Anonymous bind is allowed; do not block commands on missing credentials.
- Avoid implementing full config/secret systems before MVP.
