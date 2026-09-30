# Release verification boundaries

Passing local checks establishes the exercised library and product contracts.
It does not establish installation, live authentication, authorization or
production service behavior. [CONTRIBUTING.md](../CONTRIBUTING.md#releases)
owns release commands; this page owns the limits of that evidence.

Before claiming a supported product release, record the exact framework and UiO
product revisions, successful CI checks, supported target and built artifact.
The core publishes a crate and release notes; prebuilt cross-platform binaries
are outside its current release flow. The product's paired checks must identify
the framework revision used, rather than infer it from a package version alone.

Use approved non-production services for live authentication and read/write
verification. Check Kerberos expiry/renewal, keyring availability, failed login,
profile/origin switching and service authorization. Scope writes to a disposable
object and verify ambiguous outcomes before retrying.

Verify cancellation during and after task-follow sessions, including a quiet
stream. A process-wide signal handler or network deadline alone does not prove
prompt interruption or restoration of normal process behavior.

Run a current dependency advisory/license review in a network-enabled release
environment. Check diagnostics using synthetic credentials; arbitrary remote
error text is not proof of redaction. HTTP request deadlines do not by themselves
establish DNS resolver deadlines. LDAP response-inactivity limits are not overall
deadlines for continuously streaming results.

The DSL operates on canonical rows and explicit groups, with raw documents for
unstaged presentation. Floating-point numeric operations and whole-second
timestamp comparisons have precision limits; see [DSL_REVIEW.md](DSL_REVIEW.md).
Do not infer equivalent scope or precision for every verb from shared execution.

Report observed commands/results and any skipped effects. An archive built
with verification disabled, a test count, or an unlinked earlier success is not
a receipt for the release being evaluated.
