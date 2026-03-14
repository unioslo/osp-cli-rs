# Scripts

This directory contains small operational helpers for `osp-cli-rust`.

The folder is intentionally flat for now. The current rule is simple:

- keep scripts grouped by workflow domain
- prefer a few explicit entrypoints over many overlapping helpers
- keep policy scripts boring and reviewable
- do not turn this directory into a generic utilities bucket

## Quality And Confidence

- `confidence.py`
  Runs the named local confidence lanes used by hooks, humans, and CI.
- `coverage.py`
  Owns the repository coverage gate and coverage utility commands.
- `public-docs.py`
  Enforces public Rustdoc coverage and feature-gate wording in staged or
  repo-wide mode.
- `check-contract-env.sh`
  Guards the contract-test environment assumptions.

## Release

- `release.py`
  Owns the release workflow subcommands: `bump`, `check`, and `tag`.

## Plugins

- `plugin_describe_from_click.py`
  Helper for describing Click-based plugins.
- `plugin_describe_from_argparse.py`
  Helper for describing argparse-based plugins.

## Setup

- `install-git-hooks.sh`
  Configures the repository git hook path and commit template.

## Notes

- If a script starts owning a distinct workflow, give it a clear top-level name.
- If one workflow grows multiple tightly related helpers, prefer consolidating
  them behind one explicit script with subcommands.
- If this directory grows much larger, split it by domain deliberately instead
  of letting the flat layout decay into clutter.
