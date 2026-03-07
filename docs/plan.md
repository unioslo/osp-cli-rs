# osp-cli-rust Review Plan

## 1. Boundaries and layering (Score: 9/10)
**Review:** Excellent separation via independent workspace crates (`osp-core`, `osp-config`, `osp-ui`, `osp-dsl`). Business objects map into `Rows` rather than directly printing text, keeping the domain completely agnostic to terminal rendering.  
*Deep Dive:* `osp-core/src/output.rs` explicitly isolates UI decisions into discrete semantic enums (`OutputFormat`, `RenderMode`, `ColorMode`) which keeps the transport boundary tightly contained.
**Actionable Suggestion:** The boundary is almost perfect, though command handlers still interact directly with `UiState::message_verbosity`. Consider hiding the UI entirely behind an event channel or common interface to finalize decoupling.

## 2. Command architecture (Score: 8/10)
**Review:** Commands like `plugins` and `config` are correctly structured, taking parsed arguments and emitting a discrete output list or failure. The use of context structs (`PluginsCommandContext<'a>`) successfully isolates state.
**Actionable Suggestion:** There is still minor boilerplate in commands mapping logic outcomes into `Rows`. Generalize a return trait (e.g., `IntoRows`) so handlers only need to return their structured generic models.

## 3. REPL integration (Score: 4/10)
**Review:** **Major architecture smell.** As defined in the review criteria, `plugins.rs` (and seemingly others) has two entirely separate implementations for each command path: `run_plugins_command` (returning `CliCommandResult`) and `run_plugins_repl_command` (returning `ReplCommandOutput`). Both methods map internal states identically but return different wraps.  
*Deep Dive:* `osp-repl/src/lib.rs` shows a very healthy utilization of `reedline` for prompting and a smart `SubmissionResult` abstraction. However, the dispatch layer forces `repl/dispatch.rs` and `app/dispatch.rs` to maintain independent wiring for the exact same plugin and builtin components.
**Actionable Suggestion:** Unify execution. Commands should only return a single `Result<CommandOutput>` format. The CLI driver or REPL loop driver should handle wrapping it in `CliCommandResult` or `ReplCommandOutput` specifically at the system entrypoint.

## 4. State and context management (Score: 9/10)
**Review:** Clean, explicit, and structured safely. Uses `AppRuntime`/`AppSession` with no nested `Arc<Mutex>` horrors or global singletons. Clear division between `ConfigState`, `UiState`, and `AuthState`. 
*Deep Dive:* `osp-config/src/resolver.rs` proves the integrity of this model: there is a rigorous two-step resolution (`resolve_maps_for_frame`) that completely isolates identifying raw config winners from interpolating them. 
**Actionable Suggestion:** Prevent `AppSession` from becoming a dumping ground for disparate states in the future by logically grouping related transient state pieces early. 

## 5. Error model (Score: 8/10)
**Review:** The overarching use of `miette` integrates diagnostics and rich console reporting beautifully. The executable entry point turns `Result` easily into exit codes.
**Actionable Suggestion:** `miette::miette!` string mapping is ubiquitous. Introduce strictly typed errors using `thiserror` (or `miette::Diagnostic` traits directly on enums) across foundational inner crates rather than passing strings around.

## 6. Output contract and UX consistency (Score: 10/10)
**Review:** Phenomenal `osp-ui` capability. The application strictly segregates JSON or Machine output vs. formatted rendering. Standardizing to `Rows` before outputting means any target (Table, XML, Markdown) can be synthesized seamlessly with strong table formatting guarantees.
*Deep Dive:* Handlers strictly return data payloads. `osp-core/src/output.rs` acts as a solid API contract wall between business logic and rendering logic, guaranteeing the tool can always be safely scripted.
**Actionable Suggestion:** Ensure any future internal `dbg!` or `println!` statements are linted against so they don't break the stable scriptability features on standard out. 

## 7. Parsing, tokenization, and command semantics (Score: 9/10)
**Review:** Handled intelligently. CLI arguments use `clap`, while REPL and pipelines invoke a specific `shell-words`-backed splitting model and custom parser in `osp-dsl`.
*Deep Dive:* `osp-dsl/src/parse/lexer.rs` houses a robust state machine lexer tracking single/double quotes, spaces, and escape sequences explicitly (`State::EscapeDouble`, `State::SingleQuote`). It gracefully catches and emits specific syntax errors (`UnterminatedSingleQuote`) instead of panicking.
**Actionable Suggestion:** Maintain a very strict boundary between what the OS expands (shell variables) and what the CLI/repl expands organically so users don't face double escaping bugs. Add integration tests for highly confusing quoting interactions (e.g. `osp plugin exec --flag="foo bar"`).

## 8. Testability (Score: 8/10)
**Review:** Testing strategy is structurally enabled by `Context` arguments that break external dependencies. `assert_cmd` and `predicates` provide robust verification on outputs without requiring manual `Command::new` spans.
**Actionable Suggestion:** Expand "snapshot" or golden tests specifically for CLI vs REPL outputs. This would quickly reveal that the dual-implemented REPL code paths in #3 behave identically over time.

## 9. Dependency and compile-time hygiene (Score: 10/10)
**Review:** Very clean! Workspace splits heavily reduce re-compilations. Rust trait boundaries are enforced smartly. No heavy async-runtimes like `tokio` are gratuitously imported.
**Actionable Suggestion:** Lock down internal sub-dependencies (e.g., locking default features in `clap`, `reedline`, and `comfy-table`) to absolutely only what is necessary, saving binary size and build times.

## 10. Ownership and API ergonomics (Score: 9/10)
**Review:** The code adopts idiomatic Rust borrowing correctly (`&'a Context`), avoiding over-boxing or string cloning. 
**Actionable Suggestion:** Command pipelines and generators could benefit from lazily evaluated iterators instead of eagerly constructing `Vec<Cow<'_, str>>` or `Vec<Row>` structures, particularly for lists with large scale config resolutions.

## 11. Concurrency and async boundaries (Score: 10/10)
**Review:** Superb restraint. Given this is purely a CLI app right now, keeping it synchronous avoids the dreaded terminal-blocking deadlocks inherent to async UI interactions.
**Actionable Suggestion:** If streaming fetches or plugin network requests are added later, offload them to separate native threads with `std::sync::mpsc` channels feeding into the UI stream buffer instead of adding an async executor.

## 12. Config and environment model (Score: 9/10)
**Review:** Advanced precedence loading model is evident (file, environment variables, overrides), supporting dynamic profiles (`osp --profile=...`). Includes helpful tools like `config doctor` and config overrides testing.
*Deep Dive:* `osp-config/src/resolver.rs` maintains 6 explicit config precedence layers (Defaults -> File -> Secrets -> Env -> CLI -> Session) keeping source origins easily auditable at runtime.
**Actionable Suggestion:** Centralize strict schemas for configs so missing/invalid keys fail rapidly upon bootstrap rather than failing deep inside application usage. Currently `get_string` resolves dynamically when accessed.

## 13. Extension story (Score: 10/10)
**Review:** Fantastic approach via `PluginManager`, which scans directories, detects new commands (under `Commands::External`), tracks capability versions, and seamlessly delegates command contexts directly to plugins. Outstanding implementation pattern for scalability.
**Actionable Suggestion:** Document the inter-process expectation interface exactly so plugins know they must use specific exit codes or write output as JSON to maintain the format/UX contracts expected by `osp-core`.

## 14. Observability and diagnostics (Score: 9/10)
**Review:** Uses `tracing` to great effect alongside verbosity handlers (`--verbose`, `--debug`) and `tracing-subscriber`. Subcommands like `doctor` are highly advantageous for resolving configuration issues in production contexts.
**Actionable Suggestion:** Write out an explicit `--trace-dump=<file.json>` parameter which can serialize the entire launch context and tracing spans silently for user diagnostic tickets.

## 15. Security and trust boundaries (Score: 8/10)
**Review:** AuthState filters plugin capabilities securely upfront (`ensure_builtin_visible_for`). `PluginManager` isolates subprocess interaction effectively.
**Actionable Suggestion:** Since `Commands::External` relies on CLI spawning plugin subprocesses, enforce stringent shell-escape linting inside `run_external_plugin_command` so unsanitized environment variables passed to standard pipes do not pose remote-execution surfaces.
