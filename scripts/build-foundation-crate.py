#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import textwrap
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import tomllib


REPO_ROOT = Path(__file__).resolve().parents[1]
WORKSPACE_TOML = REPO_ROOT / "Cargo.toml"
DEFAULT_OUT_DIR = REPO_ROOT / "dist" / "osp-cli-foundation"
CRATE_ORDER = [
    "osp-core",
    "osp-config",
    "osp-dsl",
    "osp-ports",
    "osp-api",
    "osp-services",
    "osp-ui",
    "osp-completion",
    "osp-repl",
    "osp-cli",
]


@dataclass(frozen=True)
class CrateInfo:
    package_name: str
    module_name: str
    manifest_path: Path
    crate_root: Path
    src_dir: Path
    dependencies: dict[str, Any]
    dev_dependencies: dict[str, Any]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Generate a single publishable osp-cli-foundation crate from the current workspace."
        )
    )
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=DEFAULT_OUT_DIR,
        help=f"Destination directory for the generated crate (default: {DEFAULT_OUT_DIR})",
    )
    parser.add_argument(
        "--name",
        default="osp-cli-foundation",
        help="Package name for the generated crate.",
    )
    parser.add_argument(
        "--run-check",
        action="store_true",
        help="Run `cargo check --all-features` in the generated crate.",
    )
    parser.add_argument(
        "--run-tests",
        action="store_true",
        help="Run `cargo test --lib --all-features` in the generated crate.",
    )
    parser.add_argument(
        "--run-package",
        action="store_true",
        help="Run `cargo package --allow-dirty` in the generated crate.",
    )
    parser.add_argument(
        "--keep-existing",
        action="store_true",
        help="Do not delete the output directory before regenerating it.",
    )
    return parser.parse_args()


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def run(command: list[str], cwd: Path) -> None:
    subprocess.run(command, cwd=cwd, check=True)


def run_capture(command: list[str], cwd: Path) -> str:
    result = subprocess.run(command, cwd=cwd, check=True, text=True, capture_output=True)
    return result.stdout.strip()


def load_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def workspace_config() -> tuple[dict[str, Any], dict[str, Any]]:
    payload = load_toml(WORKSPACE_TOML)
    workspace = payload["workspace"]
    return workspace["package"], workspace["dependencies"]


def crate_module_name(package_name: str) -> str:
    return package_name.replace("-", "_")


def load_crates() -> list[CrateInfo]:
    crates: list[CrateInfo] = []
    for package_name in CRATE_ORDER:
        crate_root = REPO_ROOT / "crates" / package_name
        manifest_path = crate_root / "Cargo.toml"
        payload = load_toml(manifest_path)
        package = payload["package"]
        if package["name"] != package_name:
            fail(f"unexpected package name in {manifest_path}: {package['name']}")
        crates.append(
            CrateInfo(
                package_name=package_name,
                module_name=crate_module_name(package_name),
                manifest_path=manifest_path,
                crate_root=crate_root,
                src_dir=crate_root / "src",
                dependencies=payload.get("dependencies", {}),
                dev_dependencies=payload.get("dev-dependencies", {}),
            )
        )
    return crates


def normalize_dep_spec(name: str, spec: Any, workspace_deps: dict[str, Any]) -> dict[str, Any]:
    if isinstance(spec, str):
        return {"version": spec}
    if not isinstance(spec, dict):
        fail(f"unsupported dependency specification for {name!r}: {spec!r}")
    if spec.get("workspace") is True:
        workspace_spec = workspace_deps.get(name)
        if workspace_spec is None:
            fail(f"workspace dependency {name!r} was not found in workspace dependencies")
        return normalize_dep_spec(name, workspace_spec, workspace_deps)

    result = dict(spec)
    result.pop("path", None)
    result.pop("workspace", None)
    return result


def merge_dep_specs(existing: dict[str, Any] | None, incoming: dict[str, Any]) -> dict[str, Any]:
    if existing is None:
        return dict(incoming)

    result = dict(existing)

    if "version" in incoming:
        if "version" in result and result["version"] != incoming["version"]:
            fail(f"conflicting versions for dependency: {result['version']} vs {incoming['version']}")
        result["version"] = incoming["version"]

    result["features"] = sorted(
        set(result.get("features", [])) | set(incoming.get("features", []))
    )
    if not result["features"]:
        result.pop("features", None)

    if incoming.get("default-features") is False or result.get("default-features") is False:
        result["default-features"] = False
    else:
        result.pop("default-features", None)

    if incoming.get("optional") is False or result.get("optional") is False:
        result["optional"] = False
    elif incoming.get("optional") is True and result.get("optional") is True:
        result["optional"] = True
    else:
        result.pop("optional", None)

    for key in ("package",):
        if key in incoming:
            if key in result and result[key] != incoming[key]:
                fail(f"conflicting {key} values for dependency: {result[key]} vs {incoming[key]}")
            result[key] = incoming[key]

    return result


def merged_dependencies(crates: list[CrateInfo], workspace_deps: dict[str, Any]) -> tuple[dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    internal = {crate.package_name for crate in crates}
    deps: dict[str, dict[str, Any]] = {}
    dev_deps: dict[str, dict[str, Any]] = {}

    for crate in crates:
        for section_name, source, target in (
            ("dependencies", crate.dependencies, deps),
            ("dev-dependencies", crate.dev_dependencies, dev_deps),
        ):
            for name, spec in source.items():
                if name in internal:
                    continue
                normalized = normalize_dep_spec(name, spec, workspace_deps)
                target[name] = merge_dep_specs(target.get(name), normalized)

    return deps, dev_deps


def toml_value(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        return repr(value)
    if isinstance(value, str):
        escaped = value.replace("\\", "\\\\").replace('"', '\\"')
        return f'"{escaped}"'
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(item) for item in value) + "]"
    raise TypeError(f"unsupported TOML value: {value!r}")


def render_dependency_table(deps: dict[str, dict[str, Any]]) -> str:
    lines: list[str] = []
    for name in sorted(deps):
        spec = deps[name]
        if set(spec.keys()) == {"version"}:
            lines.append(f'{name} = {toml_value(spec["version"])}')
            continue

        parts = []
        if "version" in spec:
            parts.append(f'version = {toml_value(spec["version"])}')
        for key in ("default-features", "features", "optional", "package"):
            if key in spec:
                parts.append(f"{key} = {toml_value(spec[key])}")
        lines.append(f"{name} = {{ " + ", ".join(parts) + " }")
    return "\n".join(lines)


def rewrite_source(content: str, crate: CrateInfo, crates: list[CrateInfo]) -> str:
    root_marker = "__OSP_FOUNDATION_ROOT__::"
    rewritten = content
    for other in crates:
        module_name = other.module_name
        rewritten = re.sub(
            rf"(?<!crate::)(?<![A-Za-z0-9_]){module_name}::",
            f"{root_marker}{module_name}::",
            rewritten,
        )
    rewritten = re.sub(r"\bcrate::", f"crate::{crate.module_name}::", rewritten)
    rewritten = rewritten.replace(root_marker, "crate::")
    return rewritten


def write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def copy_crate_sources(out_src: Path, crates: list[CrateInfo]) -> None:
    for crate in crates:
        dest_root = out_src / crate.module_name
        for source in crate.src_dir.rglob("*.rs"):
            relative = source.relative_to(crate.src_dir)
            if relative.name == "main.rs":
                continue
            if relative.name == "lib.rs":
                relative = relative.with_name("mod.rs")
            dest = dest_root / relative
            rewritten = rewrite_source(source.read_text(), crate, crates)
            write_text(dest, rewritten)

        if crate.package_name == "osp-cli":
            mod_path = dest_root / "mod.rs"
            mod_path.write_text(
                mod_path.read_text() + "\n\npub use crate::row;\n"
            )


def render_foundation_lib(crates: list[CrateInfo]) -> str:
    module_lines = [f"pub mod {crate.module_name};" for crate in crates]
    exports = textwrap.dedent(
        """
        pub use crate::osp_cli::{Cli, run_from, run_process};
        pub use crate::osp_cli::state;
        pub use crate::osp_cli::pipeline;
        """
    ).strip()
    return "\n".join(module_lines) + "\n\n" + exports + "\n"


def render_binary(crate_name: str) -> str:
    rust_ident = crate_name.replace("-", "_")
    return textwrap.dedent(
        f"""\
        fn main() {{
            std::process::exit({rust_ident}::run_process(std::env::args_os()));
        }}
        """
    )


def render_cargo_toml(
    package_name: str,
    version: str,
    edition: str,
    dependencies: dict[str, dict[str, Any]],
    dev_dependencies: dict[str, dict[str, Any]],
) -> str:
    try:
        repository = run_capture(["git", "config", "--get", "remote.origin.url"], cwd=REPO_ROOT)
    except subprocess.CalledProcessError:
        repository = ""

    dependencies = dict(dependencies)
    dev_dependencies = dict(dev_dependencies)

    # The generated crate needs clap-enabled plugin surface to match the current app.
    dependencies["clap"] = merge_dep_specs(
        dependencies.get("clap"),
        {"version": "4.5", "default-features": False, "features": ["derive", "std", "help", "usage", "error-context", "suggestions", "color"]},
    )

    sections = [
        "[package]",
        f'name = "{package_name}"',
        f'version = "{version}"',
        f'edition = "{edition}"',
        'description = "Generated single-crate foundation package for osp-cli"',
        'license-file = "LICENSE"',
        'readme = "README.md"',
        *( [f'repository = "{repository}"'] if repository else [] ),
        "",
        "[features]",
        'default = ["clap"]',
        "clap = []",
        "",
        "[[bin]]",
        'name = "osp"',
        'path = "src/bin/osp.rs"',
        "",
        "[dependencies]",
        render_dependency_table(dependencies),
    ]
    if dev_dependencies:
        sections.extend(
            [
                "",
                "[dev-dependencies]",
                render_dependency_table(dev_dependencies),
            ]
        )
    sections.append("")
    return "\n".join(section for section in sections if section is not None)


def write_generated_crate(out_dir: Path, package_name: str) -> None:
    workspace_package, workspace_deps = workspace_config()
    crates = load_crates()
    dependencies, dev_dependencies = merged_dependencies(crates, workspace_deps)

    out_src = out_dir / "src"
    copy_crate_sources(out_src, crates)
    write_text(out_src / "lib.rs", render_foundation_lib(crates))
    write_text(out_src / "bin" / "osp.rs", render_binary(package_name))

    cargo_toml = render_cargo_toml(
        package_name=package_name,
        version=workspace_package["version"],
        edition=workspace_package["edition"],
        dependencies=dependencies,
        dev_dependencies=dev_dependencies,
    )
    write_text(out_dir / "Cargo.toml", cargo_toml)

    for doc_name in ("README.md", "LICENSE"):
        source = REPO_ROOT / doc_name
        if source.exists():
            shutil.copy2(source, out_dir / doc_name)


def main() -> None:
    args = parse_args()
    out_dir = args.out_dir.resolve()

    if out_dir.exists() and not args.keep_existing:
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    write_generated_crate(out_dir, args.name)
    print(f"Generated crate at {out_dir}")

    if args.run_check:
        run(["cargo", "check", "--all-features"], cwd=out_dir)
    if args.run_tests:
        run(["cargo", "test", "--lib", "--all-features"], cwd=out_dir)
    if args.run_package:
        run(["cargo", "package", "--allow-dirty"], cwd=out_dir)


if __name__ == "__main__":
    main()
