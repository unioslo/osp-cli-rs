# ADR 0001: Terminal and REPL contracts

Status: accepted (2026-10-23)

## Context

Repeated maintainer requirements must survive renderer and editor changes.
Passing snapshots do not establish scrollback, cancellation or input readiness.
The framework owns these mechanisms; products own service policy and artwork.

## Decision

Every rule below requires a guard test that fails when the rule is violated.
Removing or weakening a guard requires amending this ADR. Demonstrate each
new guard with a deliberate violating mutation, then revert the mutation.
`UNGUARDED` identifies outstanding enforcement, never an exception to a rule.
Product guards below exercise framework behaviour through the assembled CLI.

| Rule | Decision and reason | Guard |
|---|---|---|
| T1 Screen lifetime | Intro begins at the top of the screen, after one blank row. Move existing visible output into scrollback before homing; after exit, scrolling reaches both OSP output and the earlier shell output. Ctrl-L does the same. Never erase scrollback (ESC[3J) or use the alternate screen for the intro, REPL or command output; transient full-screen dialogs (pager, pickers, first-run selector) may use it only while open. Earlier shell output belongs to the user. | `e2e::terminal_rules::intro_and_clear_preserve_terminal_scrollback` |
| T2 Section rules | All ruled section headings and closing rules start at column 0, independently of the body margin. Intro, help, documents and message layouts share this geometry. | `ui::rules_tests::section_rules_start_at_column_zero` |
| T3 Nested blocks | Indent two columns per level. Tables, grids, lists and nested objects hang at the same parent + 2 position; never align across levels. Fields beneath a card title are one level in. Every list subtable uses `Name (n):`, including one row, followed by its column-header row. Same-level scalar continuations may align with their value column. This keeps related output readable without label-dependent nesting. | `ui::rules_tests::nested_blocks_use_one_indent` |
| T4 Margins and copying | Default REPL body margin is two columns, applied once. One-shot output and copied/exported output have no REPL margin. Copying must not export terminal chrome. | `ui::rules_tests::repl_margin_applies_once_and_exports_have_none` |
| T5 Password cancellation | Ctrl-C at any visible or hidden password dialogue is cancellation, never an empty successful answer or permission to continue. The startup caller exits; an in-REPL caller abandons the command. | Product `contracts::rules::password_prompts_cancel_startup_and_commands`; BOFH also has `contracts::bofh_contract::repl_cancellation_and_sensitive_history_preserve_session`. |
| T6 Startup | Measure startup until a keystroke is accepted into a live editable line, not first paint. Slow service/catalog/cache refreshes must not postpone that point. Existing-session startup is included. | Product `startup::intro_bytes_and_startup_requests` (`scripts/startup.py` holds service responses until an edit succeeds); release timing uses its warm/cold/slow-services scenarios and `perf/budgets.json`. |
| T7 Transient output | Clear spinners and replaceable lines before any other stdout/stderr output, including errors, notices and prompts. They must not become prefixes of durable output. | `integration::transient_output::transient_output_clears_before_documents`; `integration::transient_output::password_dialogues_clear_transient_output` |
| T8 Spinner delay | Wait one second before showing a spinner or replaceable progress; short operations stay quiet. | `integration::transient_output::spinner_waits_one_second` |
| T9 Colour and intro layout | Preserve resolved colour settings, semantic style distinctions and reviewed intro spacing. Accidental colour/spacing changes must fail a test, rather than silently updating the expected output. Completion values and descriptions retain distinct styles. | Product `startup::intro_bytes_and_startup_requests` compares ANSI goldens; `e2e::repl_smoke::repl_keeps_the_rich_editor_when_cursor_reports_arrive_late` covers rich-editor fallback. `repl::menu_core::tests::completion_values_and_inline_descriptions_keep_distinct_styles` guards semantic colour contrast. |
| T10 Width | Intro and prose fit the resolved terminal width, counting display columns rather than ANSI bytes; wrap text rather than silently clipping the intro. | Product `contracts::rules::intro_fits_terminal_width` guards intro width; `ui::rules_tests::colored_prose_wraps_without_losing_text` guards styled prose. |
| T11 Recovery hints | An insertable command recovery hint advertises its insert key inline; generic recovery text has no key advertisement. REPL builtins participate in command corrections. Alt+Enter inserts the scoped command as editable input; only Enter executes it. Hints must work at the current prompt. Subcommand corrections preserve later arguments; two or three choices cycle on repeated Alt+Enter while the inserted line is unchanged. An edit restarts at the first choice; more than three choices stay text-only. | `e2e::terminal_rules::recovery_key_inserts_editable_scoped_command` |
| T12 Value output | Extracted scalar-list values print one entry per line, suitable for pasting. | `ui::rules_tests::scalar_value_lists_render_one_entry_per_line` |
| T13 Fitting hints | When columns or values are hidden/shortened, name what was affected and offer usable projection/full-output recovery, including `\| P` and `--json` where applicable. | `ui::rules_tests::fitting_footer_names_losses_and_recovery` |
| T14 EOF | Ctrl-D on an empty input line exits the application from root or a service shell. Nonempty-input editing remains editor-owned. | `e2e::terminal_rules::eof_exits_from_root_and_service_shell` |

Guard paths refer to the terminal-rule implementation landed with this ADR.
A suite prefix (`e2e`, `integration` or `contracts`) identifies the test binary.
Existing guards are named where they actually prove a promise; partial coverage
is explicitly marked. [UI](../UI.md), [REPL](../REPL.md) and
[testing](../TESTING.md) own configuration and execution details.

## Consequences

Terminal promises need sized PTYs, raw escape-byte capture and a terminal model
with scrollback. Colour goldens need controlled settings. Startup guards must
prove editing while service responses remain blocked. Re-blessing a golden is
a reviewed behaviour change, not a repair for a failing contract.

Replaceable native progress is terminal presentation: machine formats and
redirected output suppress its body. Explicit event streams retain canonical
events; durable notices still reach nonterminal sinks. This keeps preliminary
UI activity out of ordinary machine output without dropping trace streams.

An explicit table recommendation preserves a collection as a table even when
it contains one record. Empty-result wording and duration formatting belong
to display metadata; canonical payloads retain their original typed values.

## Amendment: guided session cancellation (2026-10-29)

Guided embedded sessions follow normal REPL Ctrl-C behavior: clear the current
line or cancel the active command/prompt, preserving the session and its state.
Ctrl-C never leaves a guided session. Password cancellation remains governed
by T5. `exit` leaves; Ctrl-D and the existing guided Esc binding leave only
on empty input. Nonempty-input editing remains editor-owned, as in T14.

The existing
`repl::engine::session::tests::interactive_signal_paths_cover_host_picker_submit_and_ctrl_events_unit`
exercises guided cancellation. The rich-editor empty/nonempty key boundary
is additionally checked with a real product tutorial PTY; automated guided
PTY coverage remains UNGUARDED.
