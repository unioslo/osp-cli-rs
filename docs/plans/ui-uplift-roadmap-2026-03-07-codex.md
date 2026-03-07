# OSP CLI Rust UI Uplift Roadmap

Date: 2026-03-07

Status: Completed on 2026-03-07

This roadmap is now closed.

## Goal

Lift the Rust UI/UX beyond parity with `osprov-cli` without doing a 1:1 port.

The target is:

- stronger user-facing defaults
- cleaner and more explicit config
- faster rendering and startup
- more maintainable code boundaries
- more readable output and interaction patterns

This roadmap treats old `osprov-cli` ideas such as `gammel-og-bitter` as
useful UX intent, not as architecture to copy.

## What We Are Preserving From `gammel-og-bitter`

The valuable part of that mode is not the enum value itself. The value is the
intent behind it:

- less chrome
- less color dependence
- tighter prompt and layout
- fast visual scanning
- fewer decorative borders and panels
- output that still feels serious when piped, logged, or used all day

That intent is worth keeping.

The overloaded `AppInterfaceMode` shape is not worth keeping.

## What We Will Not Do

- Do not reintroduce one ambiguous switch that secretly means color, prompt,
  help style, and render backend all at once.
- Do not port Python quirks that only exist because concerns are mixed.
- Do not make plain mode feel like a degraded fallback.
- Do not put command-specific behavior into `osp-ui`.

## Breaking-Change Posture

`osprov-cli` is the production tool. `osp-cli-rust` is not.

That gives us room to make deliberate breaking changes now when they buy
long-term gains in:

- clarity
- maintainability
- testability
- config readability
- UI consistency

We should use that room carefully instead of spending it on churn.

Breaking changes are justified when they:

- remove overloaded or ambiguous concepts
- replace hidden heuristics with explicit settings
- collapse duplicate UX paths into one consistent model
- improve long-term CLI/REPL parity
- make behavior easier to explain and test

Breaking changes are not justified when they:

- only mimic Python naming for comfort
- move user-visible behavior without a clear quality gain
- create migration work without reducing architecture debt

Examples of changes this roadmap should be willing to make:

- replacing legacy interface modes with explicit presentation presets
- renaming config keys when the new shape is materially clearer
- changing default chrome and prompt behavior if the result is more coherent
- removing accidental Python-era coupling between render backend and taste

Migration guidance still matters, but compatibility with Python internals is
not a design goal.

## Product Direction

The Rust UI should have:

1. one rendering engine with explicit knobs
2. a small number of presentation presets built on those knobs
3. a first-class plain/ascii path
4. one coherent chrome system across help, prompt, messages, and data output
5. parity where parity is good, and deliberate departures where the Python UX
   is coupled or vague

## Design Principles

### 1. Separate mechanism from taste

Mechanism:

- `ui.render.mode`
- `ui.color.mode`
- `ui.unicode.mode`
- `ui.table.overflow`
- `theme.name`

Taste:

- prompt density
- border preference
- message chrome
- spacing
- help/document presentation

The engine should expose mechanism. Taste should be layered on top.

### 2. Replace legacy interface modes with presentation presets

Instead of bringing back `AppInterfaceMode`, add an explicit presentation layer.

Recommended direction:

- `ui.presentation = expressive | compact | austere`

Where:

- `expressive`
  - richer prompt
  - themed message grouping
  - comfortable spacing
- `compact`
  - tighter prompt
  - less chrome
  - denser tables and docs
- `austere`
  - spiritual successor to `gammel-og-bitter`
  - plain-first feel even when rich backend is available
  - minimal borders
  - low-noise help and message formatting

Compatibility note:

- accept `gammel-og-bitter` as a config/CLI alias for `ui.presentation=austere`
- do not use it as an internal engine concept

### 3. Plain output is a product surface

Plain mode is not just for broken terminals.

It should be:

- deliberate
- readable
- stable for scripting
- visually consistent with the compact/austere intent

### 4. One chrome language across the app

Prompt, help, diagnostics, and result rendering should feel like one tool.

Today the Rust code already has good boundaries for this:

- `osp-ui` owns rendering
- `osp-cli` owns product policy
- `osp-repl` owns terminal mechanics

Keep that split.

### 5. Favor explicit heuristics

If output changes shape automatically, the rules must be:

- documented
- testable
- explainable

No hidden “looks better here” branches.

## Stream 0: Baseline And Taste Inventory

### Objective

Capture which current Rust UI behaviors are already good, and which old Python
behaviors are worth translating into cleaner forms.

### Tasks

- Write a short inventory of current Rust presentation surfaces:
  - prompt
  - help output
  - message rendering
  - table rendering
  - MREG layout
  - JSON/code formatting
  - markdown rendering
- Record current UI config knobs and where they are applied.
- Record which Python behaviors are intent worth keeping:
  - compact prompt
  - low-chrome message style
  - no-surprises plain output
  - serious help formatting

### Files to inspect

- `docs/UI.md`
- `docs/UI_PARITY_SPEC.md`
- `docs/REPL.md`
- `crates/osp-ui/src/lib.rs`
- `crates/osp-cli/src/repl/presentation.rs`
- `osprov-cli/src/osprov_cli/repl/manager.py`
- `osprov-cli/src/osprov_cli/ui/`

### Exit criteria

- We have a written “keep / change / drop” inventory instead of vague taste.

## Stream 1: Finish The Core UI Parity Layer

### Objective

Close the remaining gaps in the rendering core before adding new presentation
profiles.

### Tasks

- Finish non-default table alignment plumbing.
- Finish richer value-style mapping from config keys.
- Add help/doc panel parity where useful.
- Improve code and JSON styling options without leaking command logic into UI.
- Ensure grouped payload rendering is stable in `table`, `md`, `mreg`, and
  `value`.

### Verification

- Add/expand golden tests for:
  - tables
  - MREG layouts
  - markdown
  - message groups
  - JSON/value modes
- Verify stdout/stderr separation stays unchanged.

### Exit criteria

- `osp-ui` feels complete enough that presentation work can compose on top of
  it instead of working around it.

## Stream 2: Add Presentation Profiles

### Objective

Introduce a clean UX layer for “how the tool should feel” without coupling that
layer to backend selection.

### Proposed Model

- `ui.presentation = expressive | compact | austere`
- explicit render knobs still exist and always win when set directly
- presentation presets seed defaults; they do not override explicit per-key
  choices

### Tasks

- Define which config keys each preset seeds.
- Keep presets in one policy module owned by `osp-cli`, not `osp-ui`.
- Add a compatibility alias from legacy `gammel-og-bitter` to `austere`.
- Add `config explain` visibility so users can see when a value came from a
  presentation preset.
  Status: implemented for effective UI values that still keep their raw config
  winner at builtin defaults.

### Initial preset intent

- `expressive`
  - richer prompt
  - grouped messages with stronger chrome
  - wider spacing
  - border-friendly
- `compact`
  - tight prompt
  - reduced spacing
  - lighter help/doc chrome
  - denser tables
- `austere`
  - compact prompt by default
  - minimal message chrome
  - plain-looking output even under rich backend
  - no decorative panels unless they add meaning

### Verification

- Contract tests for resolved settings per preset.
- Snapshot tests for the same payload rendered under each preset.
  Status: help, message, and prompt snapshots are now covered in unit tests.

### Exit criteria

- Users can choose a coherent UI personality without giving up explicit
  low-level control.

## Stream 3: Prompt, Help, And Message Chrome

### Objective

Make the app feel coherent at the “edges” of interaction, not just in result
tables.

### Tasks

- Define one prompt density policy:
  - multiline expressive
  - single-line compact/austere
- Make help rendering use the same presentation profile system.
- Keep help density in `ui.help.layout`.
- Standardize message chrome:
  - `grouped`
  - `minimal`
- Keep chrome geometry in `ui.chrome.frame`; keep message density in
  `ui.messages.layout`.
- Make error/help output feel deliberate in plain and rich backends.

### Important rule

The chrome system should be additive and removable.

If a user chooses compact or austere mode, the tool should show less, not just
the same thing with fewer colors.

### Verification

- Snapshot tests for:
  - `--help`
  - REPL intro
  - prompt rendering
  - grouped warnings/errors
  Status: prompt rendering and grouped warning/error snapshots are covered;
  intro coverage is partly covered and can still be tightened.
- CLI/REPL parity tests where the same command emits the same message groups.

### Exit criteria

- Prompt/help/messages read like one product.

## Stream 4: REPL Feel And Interaction Quality

### Objective

Raise the REPL from “functional” to “good to live in”.

### Tasks

- Finish the shared parsed-input path for completion, highlighting, and
  dispatch.
- Tighten shell-scope history behavior.
- Improve menu density and ordering for compact/austere profiles.
- Keep completion side-effect-free.
- Make partial-line help and error hints less noisy.

### UX target

- expressive mode feels guided
- compact mode feels fast
- austere mode feels sharp, quiet, and efficient

### Status

- Compact/austere root completion ordering now prioritizes core REPL commands
  and builtin shells over plugin commands.
- Shell-scoped `!?text` history now respects the active shell instead of
  leaking across root/other shell history.
- Partial/near-miss builtin parse errors now stay inside REPL help chrome
  instead of dumping raw clap stderr.
- Leading invocation flags now go through a shared projected-input path for
  REPL completion/highlighting, and flag-prefixed help alias dispatch is
  covered at the host-level agreement layer.
- PTY coverage now locks down prompt chrome and flag-prefixed completion
  behavior on the actual line editor surface.
- Aliases that contribute fixed positional context now participate in the same
  completion/highlighting/dispatch agreement model instead of falling back to
  conservative alias stubs.
- PTY coverage now includes a fixed-key alias completion path on the real line
  editor surface.
- The original roadmap-level REPL agreement gaps are now closed.

### Verification

- PTY tests for prompt, completion, history, and unknown-command recovery.
- Agreement tests for completion/highlight/dispatch on partial lines.
- Added PTY completion coverage for leading invocation flags.
- Added rebuild regression coverage for prompt-state changes after REPL
  `config set/unset`.

### Exit criteria

- REPL quality is structurally good, not just cosmetically improved.

## Stream 5: Performance And Stability

### Objective

Make the improved UI cheap enough to keep on by default.

### Tasks

- Measure render cost for common table/MREG/JSON payloads.
- Avoid repeated width/style/layout recomputation within one render pass.
- Keep plain rendering fast and allocation-light.
- Make theme/presentation reload deterministic after `config set/unset`.
- Add a small benchmark target for representative outputs.

### Status

- The hot render/copy paths now resolve render settings once per operation and
  reuse that resolved state through formatting and rendering.
- Prompt-state rebuilds after REPL `config set/unset` now have direct
  regression coverage.
- A runnable benchmark target now exists for representative renderer workloads:
  `cargo run -p osp-ui --example render_bench --release -- <iterations>`.

### Verification

- Baseline and compare:
  - cold startup
  - one-shot table render
  - one-shot JSON render
  - REPL prompt cycle
  - config/theme rebuild
- Regression coverage now proves prompt styling survives a real REPL
  `config set/unset` rebuild cycle.
- The benchmark example covers table, JSON, MREG, and grouped-message
  rendering paths.

### Exit criteria

- UI uplift does not make the tool feel heavier.

## Stream 6: Documentation And Migration

### Objective

Make the new UI model easy to understand and safe to adopt.

### Tasks

- Update `docs/UI.md` with presentation profiles and override precedence.
- Update `docs/CONFIG.md` with the new keys and compatibility aliases.
- Document `gammel-og-bitter -> austere` as a user-facing migration path.
- Add examples for:
  - logging-friendly plain output
  - compact REPL
  - austere all-day operator profile

### Status

- `docs/UI.md` now documents the final preset model, override precedence, and
  benchmark target.
- `docs/CONFIG.md` now documents the current UI/REPL keys, REPL-vs-CLI config
  write defaults, and the `gammel-og-bitter -> austere` migration path.
- `docs/REPL.md` now reflects the current prompt/help/message model instead of
  the earlier MVP-only framing.

### Exit criteria

- New users can find the right knobs quickly.
- Existing users can keep the spirit of their old setup without carrying old
  implementation debt forward.

## Suggested Implementation Order

1. Finish Stream 1 core parity gaps.
2. Implement Stream 2 presentation presets.
3. Apply Stream 3 chrome policy to prompt/help/messages.
4. Land Stream 4 REPL feel work.
5. Harden with Stream 5 performance and Stream 6 docs.

## Definition Of Done

The UI uplift is done when:

- the rendering core is explicit and test-backed
- presentation presets exist without reintroducing overloaded interface modes
- `austere` captures the useful spirit of `gammel-og-bitter`
- CLI and REPL feel like the same product
- plain output remains first-class
- the code is easier to read than the Python version, not just safer

## Completion Assessment

This roadmap is complete.

All six streams have reached their intended outcome:

- Stream 1
  - The renderer contract is explicit, test-backed, and no longer the weak
    point.
- Stream 2
  - Presentation presets exist as policy on top of low-level knobs, with
    `config explain` visibility and behavior-level coverage.
- Stream 3
  - Prompt, help, and message surfaces now share one clearer chrome/layout
    vocabulary.
- Stream 4
  - REPL completion, highlighting, dispatch, shell-scoped history behavior,
    menu density, and alias agreement are now structurally aligned.
- Stream 5
  - Render-path resolution and rebuild behavior are hardened enough for the
    roadmap scope, with benchmark support in place.
- Stream 6
  - User docs reflect the final model rather than the transitional plan.

## Post-Roadmap Residuals

These are normal follow-up work items, not open roadmap streams:

- add more PTY breadth when new REPL behavior is introduced
- keep growing renderer snapshots as new output shapes are added
- optionally remove a few small compiler warnings unrelated to the roadmap
- continue ordinary product iteration on aliases, plugins, and provider-owned
  shells as those features evolve

## Short Opinion

The right move is not to port `gammel-og-bitter`.

The right move is to preserve its seriousness, density, and anti-chrome intent,
then rebuild it as a clean presentation profile on top of explicit rendering
contracts.
