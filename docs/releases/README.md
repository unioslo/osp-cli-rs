# Releases

Each tagged release must have a matching release notes file:

- `docs/releases/v1.4.5.md`
- `docs/releases/v1.4.6.md`

The release workflow uses that file as the GitHub release body.

Rules:

- one file per released version
- file name must match the tag exactly
- `CHANGELOG.md` must contain a matching version section
- remove all `TODO` markers before publishing

Useful commands:

```bash
just bump patch "Summarize the release"
just release-check
just release-dry
just release-sign
```
