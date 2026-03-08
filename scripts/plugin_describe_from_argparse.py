#!/usr/bin/env python3
"""Best-effort argparse -> osp DescribeV1 helper.

This is intentionally a convenience script, not a compatibility contract.
If it drifts from modern argparse behavior, treat the output as a starter
document and adjust it manually.
"""

from __future__ import annotations

import argparse
import importlib
import json
import sys
from typing import Any


def load_attr(spec: str) -> Any:
    module_name, sep, attr_name = spec.partition(":")
    if not sep:
        raise SystemExit("expected MODULE:ATTR")
    module = importlib.import_module(module_name)
    return getattr(module, attr_name)


def suggestion_payload(choices: Any) -> list[dict[str, str]]:
    if not choices:
        return []
    return [{"value": str(choice)} for choice in choices]


def convert_action(action: argparse.Action) -> tuple[str, dict[str, Any]] | dict[str, Any] | None:
    if isinstance(action, argparse._HelpAction):
        return None

    if action.option_strings:
        long_flags = [flag for flag in action.option_strings if flag.startswith("--")]
        key = long_flags[0] if long_flags else action.option_strings[0]
        flag_only = action.nargs == 0
        return key, {
            "about": action.help if action.help not in (None, argparse.SUPPRESS) else None,
            "flag_only": flag_only,
            "multi": action.nargs in ("*", "+") or isinstance(action, argparse._AppendAction),
            "value_type": "Path" if action.type is argparse.FileType else None,
            "suggestions": suggestion_payload(getattr(action, "choices", None)),
        }

    return {
        "name": action.dest if action.dest not in (None, argparse.SUPPRESS) else None,
        "about": action.help if action.help not in (None, argparse.SUPPRESS) else None,
        "multi": action.nargs in ("*", "+"),
        "value_type": "Path" if action.type is argparse.FileType else None,
        "suggestions": suggestion_payload(getattr(action, "choices", None)),
    }


def convert_parser(name: str, parser: argparse.ArgumentParser) -> dict[str, Any]:
    command = {
        "name": name,
        "about": parser.description or "",
        "args": [],
        "flags": {},
        "subcommands": [],
    }

    subparsers = None
    for action in parser._actions:
        if isinstance(action, argparse._SubParsersAction):
            subparsers = action
            continue
        converted = convert_action(action)
        if converted is None:
            continue
        if isinstance(converted, tuple):
            key, payload = converted
            command["flags"][key] = payload
        else:
            command["args"].append(converted)

    if subparsers is not None:
        for subcommand_name, subparser in sorted(subparsers.choices.items()):
            command["subcommands"].append(convert_parser(subcommand_name, subparser))

    return command


def build_describe(parser: argparse.ArgumentParser, plugin_id: str, plugin_version: str) -> dict[str, Any]:
    prog = (parser.prog or plugin_id).split()[0]
    top_name = prog.removeprefix("osp-")
    return {
        "protocol_version": 1,
        "plugin_id": plugin_id,
        "plugin_version": plugin_version,
        "min_osp_version": None,
        "commands": [convert_parser(top_name, parser)],
    }


def main() -> int:
    cli = argparse.ArgumentParser(description="Convert an argparse parser into a DescribeV1 skeleton.")
    cli.add_argument("parser_ref", help="Python reference in MODULE:ATTR form")
    cli.add_argument("--plugin-id", required=True)
    cli.add_argument("--plugin-version", default="0.1.0")
    args = cli.parse_args()

    parser = load_attr(args.parser_ref)
    if callable(parser) and not isinstance(parser, argparse.ArgumentParser):
        parser = parser()
    if not isinstance(parser, argparse.ArgumentParser):
        raise SystemExit("target is not an argparse.ArgumentParser")

    json.dump(build_describe(parser, args.plugin_id, args.plugin_version), sys.stdout, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
