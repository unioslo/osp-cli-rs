# LDAP Client (MVP and Beyond)

This document defines the LDAP client behavior for the Rust rewrite, with a
focus on the MVP (`ldap user`, `ldap netgroup`) and anonymous bind.

## MVP Requirements

- Anonymous bind is allowed and should be the default when no credentials are
  provided.
- Only two commands are required: `ldap user` and `ldap netgroup`.
- The client should be mockable for tests; no real LDAP network access is
  required for MVP tests.

## Config Keys

MVP keys (all optional):

- `ldap.url` (string) — LDAP server URL.
- `ldap.base_dn` (string) — base search DN.
- `ldap.bind_dn` (string) — bind DN for authenticated bind.
- `ldap.bind_password` (secret) — bind password.
- `ldap.anonymous` (bool, default true) — allow anonymous bind.

Compatibility aliases (optional):

- `ldap.bind_user` -> `ldap.bind_dn`
- `ldap.domain_controlers` (legacy) -> derive `base_dn` if no base is set

## Command Contracts

If `ldap user` is called without a uid, default to the active `user.name` from config or `-u/--user`.


**ldap user <uid>**

- Returns a list of rows (even if one row).
- Fields should match the MVP contract in docs/MVP_LDAP_REPL.md.

**ldap netgroup <name>**

- Returns a list of rows (even if one row).
- Fields should match the MVP contract in docs/MVP_LDAP_REPL.md.

## Error Mapping

- Connection failures -> `ConfigError` if URL missing, or `NetworkError` if unreachable.
- Search yields no results -> empty list, not an error.
- Invalid DN or auth failure -> `AuthError` (but MVP should avoid auth).

## Mock Client (MVP)

For tests, a simple in-memory LDAP client is acceptable:

- Use fixtures for `user` and `netgroup` responses.
- Deterministic ordering of list fields.
- Always return list-of-rows.

## Later Extensions

- `ldap host`, `ldap org`, `ldap automount`, `ldap filegroup`.
- Attribute filtering and custom LDAP filters.
- Auth flows with token caching.
