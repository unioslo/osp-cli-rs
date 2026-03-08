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
DEFAULT_OUT_DIR = REPO_ROOT / "foundation"
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
        "--run-cov",
        action="store_true",
        help="Run `cargo llvm-cov --lib --all-features` in the generated crate.",
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


def rename_snapshot_filename(filename: str, source_crate_name: str, foundation_crate_name: str) -> str:
    rust_ident = source_crate_name.replace("-", "_")
    if filename.startswith(f"{rust_ident}__"):
        return f"{foundation_crate_name.replace('-', '_')}__{filename}"
    return filename


def copy_support_files(crate: CrateInfo, dest_root: Path, foundation_crate_name: str) -> None:
    for source in crate.src_dir.rglob("*"):
        if not source.is_file() or source.suffix == ".rs":
            continue
        relative = source.relative_to(crate.src_dir)
        if source.suffix == ".snap":
            relative = relative.with_name(
                rename_snapshot_filename(relative.name, crate.package_name, foundation_crate_name)
            )
        dest = dest_root / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)


def copy_crate_sources(out_src: Path, crates: list[CrateInfo], foundation_crate_name: str) -> None:
    for crate in crates:
        dest_root = out_src / crate.module_name
        copy_support_files(crate, dest_root, foundation_crate_name)
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
    hidden_modules = "\n".join(f"#[doc(hidden)]\npub mod {crate.module_name};" for crate in crates)
    return textwrap.dedent(
        f"""\
        //! `osp-cli-foundation` is the single-crate staging area for the future root
        //! `src/` layout.
        //!
        //! The internal implementation still mirrors the old workspace split under
        //! `osp_*` module names, but consumers should start preferring the stable
        //! top-level modules exported here:
        //!
        //! - [`app`] for the main CLI entrypoints and stateful host surface
        //! - [`config`] for configuration types and resolution
        //! - [`core`] for shared output, row, and runtime types
        //! - [`dsl`] for pipeline parsing and execution
        //! - [`ui`] for rendering and message formatting
        //! - [`repl`] for REPL engine types
        //! - [`completion`] for command/completion tree types
        //! - [`api`], [`ports`], and [`services`] for the service/client layer
        //!
        //! The `osp_*` modules remain public for now as compatibility shims while the
        //! crate is still transitioning away from the generated mirror layout.
        //
        // Generated by scripts/build-foundation-crate.py.
        // Edit the workspace crates first, then regenerate this prototype.

        {hidden_modules}

        pub mod app {{
            //! Main host-facing entrypoints and stateful runtime surfaces.

            pub use crate::osp_cli::{{
                App, AppBuilder, AppRunner, BufferedUiSink, StdIoUiSink, UiSink, run_from,
                run_process, run_process_with_sink,
            }};
        }}

        pub mod runtime {{
            //! Host runtime, session, and launch-state types used to embed the CLI/REPL.

            pub use crate::osp_cli::state::{{
                AppClients, AppRuntime, AppSession, AppState, AuthState, ConfigState,
                DebugTimingBadge, DebugTimingState, LastFailure, LaunchContext, ReplScopeFrame,
                ReplScopeStack, RuntimeContext, TerminalKind, UiState,
            }};
        }}

        pub mod config {{
            //! Configuration loading, resolution, schema, and persistence.

            pub mod schema {{
                pub use crate::osp_config::{{
                    ActiveProfileSource, BootstrapConfigExplain, BootstrapKeySpec,
                    BootstrapPhase, BootstrapScopeRule, BootstrapValueRule, ConfigLayer,
                    ConfigSchema, ExplainInterpolation, ExplainInterpolationStep, LayerEntry,
                    ResolveOptions, ResolvedValue, SchemaEntry, SchemaValueType,
                }};
            }}

            pub mod load {{
                pub use crate::osp_config::{{
                    ChainedLoader, ConfigLoader, EnvSecretsLoader, EnvVarLoader, LoadedLayers,
                    LoaderPipeline, SecretsTomlLoader, StaticLayerLoader, TomlFileLoader,
                }};
            }}

            pub mod resolve {{
                pub use crate::osp_config::{{
                    ConfigExplain, ConfigResolver, ExplainCandidate, ExplainLayer, ResolvedConfig,
                }};
            }}

            pub mod runtime {{
                pub use crate::osp_config::{{
                    RuntimeConfig, RuntimeConfigPaths, RuntimeDefaults, RuntimeLoadOptions,
                    DEFAULT_DEBUG_LEVEL, DEFAULT_LOG_FILE_ENABLED, DEFAULT_LOG_FILE_LEVEL,
                    DEFAULT_PROFILE_NAME, DEFAULT_REPL_HISTORY_DEDUPE,
                    DEFAULT_REPL_HISTORY_ENABLED, DEFAULT_REPL_HISTORY_MAX_ENTRIES,
                    DEFAULT_REPL_HISTORY_PROFILE_SCOPED, DEFAULT_REPL_INTRO_STYLE,
                    DEFAULT_SESSION_CACHE_MAX_RESULTS, DEFAULT_UI_CHROME_FRAME,
                    DEFAULT_UI_COLUMN_WEIGHT, DEFAULT_UI_GRID_PADDING, DEFAULT_UI_HELP_LAYOUT,
                    DEFAULT_UI_INDENT, DEFAULT_UI_MARGIN, DEFAULT_UI_MEDIUM_LIST_MAX,
                    DEFAULT_UI_MESSAGES_LAYOUT, DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH,
                    DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO, DEFAULT_UI_PRESENTATION,
                    DEFAULT_UI_SHORT_LIST_MAX, DEFAULT_UI_TABLE_BORDER,
                    DEFAULT_UI_TABLE_OVERFLOW, DEFAULT_UI_WIDTH,
                }};
            }}

            pub mod store {{
                pub use crate::osp_config::{{
                    TomlEditResult, secret_file_mode, set_scoped_value_in_toml,
                    unset_scoped_value_in_toml,
                }};
            }}

            pub use crate::osp_config::{{
                ConfigError, ConfigSource, ConfigValue, Scope, SecretValue, bootstrap_key_spec,
                build_runtime_pipeline, default_cache_root_dir, default_config_root_dir,
                default_state_root_dir, is_alias_key, is_bootstrap_only_key,
                validate_bootstrap_value, validate_key_scope,
            }};
        }}

        pub mod core {{
            //! Shared output, row, runtime, and plugin protocol types.

            pub mod output {{
                pub use crate::osp_core::output::*;
            }}

            pub mod output_model {{
                pub use crate::osp_core::output_model::*;
            }}

            pub mod plugin {{
                pub use crate::osp_core::plugin::*;
            }}

            pub mod row {{
                pub use crate::osp_core::row::*;
            }}

            pub mod runtime {{
                pub use crate::osp_core::runtime::*;
            }}

            pub use crate::osp_core::row::Row;
            pub use crate::osp_core::runtime::{{RuntimeHints, RuntimeTerminalKind, UiVerbosity}};
        }}

        pub mod dsl {{
            //! DSL parsing, stage metadata, and pipeline execution.

            pub mod eval {{
                pub use crate::osp_dsl::eval::*;
            }}

            pub mod model {{
                pub use crate::osp_dsl::model::*;
            }}

            pub mod parse {{
                pub use crate::osp_dsl::parse::*;
            }}

            pub mod stages {{
                pub use crate::osp_dsl::stages::*;
            }}

            pub mod verbs {{
                pub use crate::osp_dsl::verbs::*;
            }}

            pub use crate::osp_dsl::{{
                Pipeline, VerbInfo, VerbStreaming, apply_output_pipeline, apply_pipeline,
                execute_pipeline, execute_pipeline_streaming, is_registered_explicit_verb,
                parse_pipeline, registered_verbs, render_streaming_badge, verb_info,
            }};
        }}

        pub mod ports {{
            //! Port traits and data-shaping helpers used by services and APIs.

            pub use crate::osp_ports::{{LdapDirectory, apply_filter_and_projection, parse_attributes}};
        }}

        pub mod api {{
            //! Higher-level API/client adapters built on the shared ports layer.

            pub use crate::osp_api::MockLdapClient;
        }}

        pub mod services {{
            //! Service-style command execution helpers over config, DSL, and ports.

            pub use crate::osp_services::{{
                ParsedCommand, ServiceContext, execute_command, execute_line, parse_repl_command,
            }};
        }}

        pub mod ui {{
            //! Rendering, layout, document, and message formatting surfaces.

            pub mod chrome {{
                pub use crate::osp_ui::chrome::*;
            }}

            pub mod clipboard {{
                pub use crate::osp_ui::clipboard::*;
            }}

            pub mod document {{
                pub use crate::osp_ui::document::*;
            }}

            pub mod format {{
                pub use crate::osp_ui::format::{{
                    MessageContent, MessageFormatter, MessageKind, MessageOptions, MessageRules,
                    build_help_document,
                }};
            }}

            pub mod interactive {{
                pub use crate::osp_ui::interactive::*;
            }}

            pub mod messages {{
                pub use crate::osp_ui::messages::*;
            }}

            pub mod style {{
                pub use crate::osp_ui::style::*;
            }}

            pub mod theme {{
                pub use crate::osp_ui::theme::*;
            }}

            pub use crate::osp_ui::{{
                CodeBlock, Document, Interactive, InteractiveResult, InteractiveRuntime, JsonBlock,
                LineBlock, LinePart, MregBlock, MregEntry, MregRow, MregValue, PanelBlock,
                PanelRules, RenderBackend, RenderRuntime, RenderSettings,
                ResolvedRenderSettings, Spinner, StyleOverrides, TableAlign, TableBlock,
                TableBorderStyle, TableOverflow, TableStyle, ValueBlock, copy_output_to_clipboard,
                copy_rows_to_clipboard, line_from_inline, parts_from_inline, render_document,
                render_document_for_copy, render_inline, render_output, render_output_for_copy,
                render_rows, render_rows_for_copy,
            }};
        }}

        pub mod completion {{
            //! Completion tree, engine, and suggestion model types.

            pub use crate::osp_completion::{{
                ArgNode, CommandLine, CommandLineParser, CommandSpec, CompletionAnalysis,
                CompletionContext, CompletionEngine, CompletionNode, CompletionTree,
                CompletionTreeBuilder, ConfigKeySpec, ContextScope, CursorState, FlagNode,
                FlagOccurrence, MatchKind, ParsedLine, QuoteStyle, Suggestion, SuggestionEngine,
                SuggestionEntry, SuggestionOutput, TailItem, TokenSpan, ValueType,
            }};
        }}

        pub mod repl {{
            //! REPL engine and prompt/history types.

            pub use crate::osp_repl::{{
                CompletionDebug, CompletionDebugFrame, CompletionDebugMatch,
                CompletionDebugOptions, DebugStep, HighlightDebugSpan, HistoryConfig,
                HistoryEntry, HistoryShellContext, LineProjection, LineProjector,
                OspHistoryStore, PromptRightRenderer, ReplAppearance, ReplInputMode,
                ReplLineResult, ReplPrompt, ReplReloadKind, ReplRunConfig, ReplRunResult,
                SharedHistory, color_from_style_spec, debug_completion, debug_completion_steps,
                debug_highlight, default_pipe_verbs, expand_history, run_repl,
            }};
        }}

        pub mod cli {{
            //! CLI-specific helpers still owned by the host layer.

            pub use crate::osp_cli::{{Cli, pipeline}};
        }}

        pub mod prelude {{
            //! Small convenience surface for embedding the app without importing the full module tree.

            pub use crate::app::{{App, AppBuilder, AppRunner, run_from, run_process}};
            pub use crate::core::output::{{ColorMode, OutputFormat, RenderMode, UnicodeMode}};
            pub use crate::runtime::{{AppState, RuntimeContext, UiState}};
            pub use crate::ui::RenderSettings;
        }}

        pub use crate::app::{{App, AppBuilder, AppRunner, run_from, run_process}};

        #[cfg(test)]
        mod tests {{
            use crate::core::output::OutputFormat;

            #[test]
            fn stable_top_level_surface_exposes_primary_entrypoints_and_types_unit() {{
                let _run_from = |args: Vec<&str>| crate::app::run_from::<Vec<&str>, &str>(args);
                let _run_process =
                    |args: Vec<&str>| crate::app::run_process::<Vec<&str>, &str>(args);
                let mut sink = crate::app::BufferedUiSink::default();
                let _builder = crate::app::AppBuilder::new().build();
                let _runner = crate::app::AppBuilder::new().build_with_sink(&mut sink);
                let _cli_type: Option<crate::cli::Cli> = None;
                let _row: crate::core::Row = Default::default();
                let _resolver: Option<crate::config::resolve::ConfigResolver> = None;
                let _completion: Option<crate::completion::CompletionEngine> = None;
                let _prompt: Option<crate::repl::ReplPrompt> = None;
                let _ldap: Option<crate::api::MockLdapClient> = None;
                let _runtime: Option<crate::runtime::AppRuntime> = None;
                let _format = OutputFormat::Json;
                let _settings = crate::ui::RenderSettings::test_plain(OutputFormat::Table);
            }}

            #[test]
            fn legacy_osp_namespaces_still_exist_during_transition_unit() {{
                let _settings = crate::osp_ui::RenderSettings::test_plain(OutputFormat::Table);
                let _format = crate::osp_core::output::OutputFormat::Json;
                let _cli_type: Option<crate::osp_cli::Cli> = None;
            }}
        }}
        """
    )


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
        "# Generated by scripts/build-foundation-crate.py.",
        "# Edit the workspace crates first, then regenerate this prototype.",
        "",
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
        "[workspace]",
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
    copy_crate_sources(out_src, crates, package_name)
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

    run(
        ["cargo", "generate-lockfile", "--manifest-path", str(out_dir / "Cargo.toml")],
        cwd=out_dir,
    )


def reset_output_dir(out_dir: Path) -> None:
    if not out_dir.exists():
        out_dir.mkdir(parents=True, exist_ok=True)
        return

    # Preserve Cargo build artifacts for the tracked foundation crate so repeated
    # regen/check cycles stay fast and do not race on a disappearing target dir.
    for child in out_dir.iterdir():
        if child.name == "target":
            continue
        if child.is_dir():
            shutil.rmtree(child)
        else:
            child.unlink()


def main() -> None:
    args = parse_args()
    out_dir = args.out_dir.resolve()

    if not args.keep_existing:
        reset_output_dir(out_dir)
    else:
        out_dir.mkdir(parents=True, exist_ok=True)

    write_generated_crate(out_dir, args.name)
    print(f"Generated crate at {out_dir}")

    if args.run_check:
        run(["cargo", "check", "--all-features"], cwd=out_dir)
    if args.run_tests:
        run(["cargo", "test", "--lib", "--all-features"], cwd=out_dir)
    if args.run_cov:
        run(["cargo", "llvm-cov", "--lib", "--all-features"], cwd=out_dir)
    if args.run_package:
        run(["cargo", "package", "--allow-dirty"], cwd=out_dir)


if __name__ == "__main__":
    main()
