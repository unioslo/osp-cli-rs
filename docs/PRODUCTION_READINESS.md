# Production readiness

Status: development follow-up record, updated 2026-09-14. These items do not
block development commits or pushes. They matter before claiming production
readiness. No local tests, builds, or live-service checks were run during the
September 14 cleanup.

## Implemented development changes

- DSL collection operations retain unrelated nulls and empty containers.
- Service field names cannot implicitly turn records into DSL groups; explicit
  grouping survives pipeline continuations. Ordinary quick syntax is unchanged.
- UiO HTTP clients have total request deadlines. Siteadmin rejects incomplete
  response bodies and unexpected redirects; failed writes explain that the
  server may already have applied the operation and are not blindly retried.
- LDAP has connection and response-inactivity timeouts. Numeric TOML timeout
  settings now take effect instead of being silently ignored.
- Manual release tags are passed through the environment and validated, never
  interpolated as shell source.

## Before a production release

1. **Cancellation:** orchestration installs a process-wide SIGINT handler that
   outlives task-following. A quiet event stream can also delay cancellation
   until the next read/deadline. Fix signal ownership and add a subprocess
   contract covering cancellation during and after a follow session. Simply
   unregistering signal-hook does not restore the previous OS disposition.
2. **Live authentication:** validate Kerberos ticket expiry/renewal, keyring
   availability, failed login, profile/origin switching, and read/write
   authorization against approved non-production UiO services. Use a disposable
   object for write verification and explicitly confirm ambiguous outcomes.
3. **Distribution:** the core release workflow's binary artifact job is
   disabled. Select supported targets and verify install/upgrade from built
   artifacts. The unpublished UiO product uses a sibling path dependency; develop and
   build the two checkouts together. Record the exact pair of repository
   revisions when producing a distributable artifact. No published-version
   fallback or compatibility layer is maintained during this cutover.
4. **Security evidence:** run a fresh dependency advisory/license review in a
   network-enabled release environment. Verify diagnostics with synthetic
   credentials; arbitrary remote error bodies are not universally redacted.
   Bound DNS lookup behavior separately: HTTP deadlines alone do not prove a
   resolver deadline. LDAP search uses a per-next-result inactivity timeout,
   not an overall deadline for a continuously streaming result set.
5. **DSL limits:** numeric comparisons/aggregations use floating point and
   timestamps have whole-second comparison precision. Do not promise exact
   large-integer accounting or subsecond comparisons. Group identity inside
   documents is stage-level, not per-node; mixed structural rewrites need
   further contracts before claiming full row/document/group equivalence.

## Verification boundary

Prefer public pipeline, native-command, real-binary, and local-server contracts.
Run the existing full suite only after implementation/static review. No release,
installation into a user's environment, or live-service mutation is implied by
passing local tests. Record the exact release revisions and CI results when
closing these gates.

An earlier draft reported local checks on 2026-09-05: 1,275 core tests,
23 selected UiO boundary tests, 146 UiO library tests, Clippy, release metadata,
and an offline archive using `--allow-dirty --no-verify`. The draft did not
identify exact tested commits or preserve a linked run receipt. Keep those
numbers as historical notes only; they do not validate the September 14
commits. Future verification records must include both repository revisions
and the commands/results actually observed.
