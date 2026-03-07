# LDAP Reference

This document describes the LDAP-facing behavior used by the LDAP command
provider.

## Core Behavior

- Anonymous bind is allowed by default when no credentials are provided.
- `ldap user` and `ldap netgroup` are the baseline commands documented here.
- Commands return list-of-row shaped output.

## Config Keys

All keys are optional:

- `ldap.url` (string) — LDAP server URL.
- `ldap.base_dn` (string) — base search DN.
- `ldap.bind_dn` (string) — bind DN for authenticated bind.
- `ldap.bind_password` (secret) — bind password.
- `ldap.anonymous` (bool, default true) — allow anonymous bind.

Compatibility aliases:

- `ldap.bind_user` -> `ldap.bind_dn`
- `ldap.domain_controlers` (legacy) -> derive `base_dn` if no base is set

## Command Contracts

If `ldap user` is called without a uid, it defaults to the active `user.name`
from config or `-u/--user`.


**ldap user <uid>**

- Returns a list of rows (even if one row).
- Fields should match the documented command contract and integration fixtures.

**ldap netgroup <name>**

- Returns a list of rows (even if one row).
- Fields should match the documented command contract and integration fixtures.

## Error Mapping

- Connection failures -> `ConfigError` if URL missing, or `NetworkError` if unreachable.
- Search yields no results -> empty list, not an error.
- Invalid DN or auth failure -> `AuthError`.

## Test Fixture Expectations

For tests and fixtures:

- Use fixtures for `user` and `netgroup` responses.
- Deterministic ordering of list fields.
- Always return list-of-rows.

## Common Extensions

- `ldap host`, `ldap org`, `ldap automount`, `ldap filegroup`.
- Attribute filtering and custom LDAP filters.
- Auth flows with token caching.
