# Releases

Each tagged release must have matching `docs/releases/vX.Y.Z.md` release notes.

The release workflow uses that file as the GitHub release body.
It also publishes the crate to crates.io through the
`crates-io-release` GitHub environment using trusted publishing.
Releases contain the source crate and GitHub release notes; no prebuilt binaries
are distributed. Install the crate with `cargo install osp-cli --locked`.

Verification records the checked-out commit once, and publication checks out that
same SHA. The release tag must still identify that commit before publication.
If the crate version already exists, its packaged `.cargo_vcs_info.json` must
identify the same clean root-package commit before publishing the GitHub notes.

Release coverage compares against the nearest reachable version tag before the
release commit, falling back to its parent. An initial root commit uses the
coverage gate's empty-tree comparison. Full confidence includes a dependency
audit; CI installs pinned `cargo-audit` 0.22.2 before running the lane.

Rules:

- one file per released version
- file name must match the tag exactly
- `CHANGELOG.md` must contain a matching version section with a nonempty body
- finish the changelog date and remove all `TODO` markers before publishing
- the `crates-io-release` environment must stay configured in GitHub and crates.io

Useful commands:

```bash
just bump patch "Summarize the release"
just release-check
just release-dry
just release-sign
```
