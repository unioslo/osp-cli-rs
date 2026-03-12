# LDAP Reference

This document covers the minimal LDAP-shaped command surface that exists in the
upstream crate and its embeddable service layer.

It is intentionally narrow. It does not define a site-specific LDAP provider
configuration contract, bind strategy, or downstream command vocabulary.

## What This Document Actually Covers

Stable upstream baseline:

- `ldap user`
- `ldap netgroup`
- row-shaped output
- optional lightweight `--filter` and `--attributes` behavior in the small
  service layer

Not covered here:

- downstream/provider-specific LDAP connection config
- site-specific schema extensions
- extra commands such as `ldap host` or `ldap org`
- auth/bind behavior owned by a downstream plugin or distribution

If you are looking for the small embeddable API rather than the full host, the
owning code is the service/port layer in
[`src/services.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/services.rs)
and
[`src/ports.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/ports.rs).

## Command Shape

Baseline commands:

```text
ldap user [uid] [--filter SPEC] [--attributes ATTRS]
ldap netgroup <name> [--filter SPEC] [--attributes ATTRS]
```

The important shape is simple:

- input is command-like text
- output is always list-of-rows
- trailing DSL stages may further reshape the rows afterward

## `ldap user`

`ldap user` looks up one logical user subject and returns zero or more rows.

If `uid` is omitted in the small service layer, the command falls back to the
active user identity from config or `-u/--user`.

Examples:

```bash
osp ldap user alice
osp ldap user alice --json
```

```text
ldap user alice | P uid mail
ldap user alice --cache | P uid
```

## `ldap netgroup`

`ldap netgroup` looks up one logical netgroup subject and returns zero or more
rows.

Examples:

```bash
osp ldap netgroup ops
```

```text
ldap netgroup ops | P cn members
```

## Filter And Attribute Behavior

The small embeddable service surface supports two lightweight modifiers:

- `--filter`
  - simple row filtering
- `--attributes`
  - comma-separated projection list

Examples:

```text
ldap user alice --attributes uid,mail
ldap netgroup ops --filter cn=ops
```

These are intentionally lightweight helpers, not a promise of full LDAP query
language parity.

## Output Contract

The upstream expectation is boring:

- results are row-shaped
- zero matches return an empty row list, not a special success shape
- attribute projection keeps only the requested keys
- DSL stages can further filter, sort, limit, or extract values afterward

That is why these compose naturally:

```text
ldap user alice --attributes uid,mail | P uid
ldap netgroup ops | VALUE cn
```

## Testing And Fixtures

For examples, doctests, and unit tests, the repo uses a deterministic in-memory
LDAP double:

- [`src/ports/mock.rs`](/home/oistes/git/github.uio.no/osp/osp-cli-rust/src/ports/mock.rs)

The fixture is intentionally small and predictable so tests can focus on:

- wildcard lookup behavior
- row filtering
- attribute projection
- empty-result handling

If a downstream plugin owns richer LDAP behavior, that belongs in the plugin's
own tests and docs rather than in this upstream file.
