use super::adapter::{ReplCompleter, build_repl_highlighter, color_from_style_spec};
use super::config::{LineProjector, ReplAppearance, ReplCompleteOn, ReplResult};
use super::editor::{AutoCompleteEmacs, OspPrompt, PaintedLine};
use super::hint::ReplHinter;
use super::overlay::{HistorySelection, build_completion_menu, launch_history_picker};
use super::{
    COMPLETION_MENU_NAME, HOST_COMMAND_CLEAR_SCREEN, HOST_COMMAND_HISTORY_PICKER,
    HOST_COMMAND_LAST_ARGUMENT, HOST_COMMAND_PAGE_NEXT, HOST_COMMAND_PAGE_PREVIOUS,
    HOST_COMMAND_PREVIOUS_ARGUMENT, SharedHistory,
};
use crate::app::{StdIoUiSink, UiSink};
use crate::completion::CompletionTree;
use crate::repl::highlight::ReplHighlighter;
use crate::repl::menu::SharedCompletionMenu;
use anyhow::Result;
use nu_ansi_term::{Color, Style};
use reedline::{
    EditCommand, Emacs, ExternalPrinter, KeyCode, KeyModifiers, Reedline, ReedlineEvent,
    ReedlineMenu, Signal, default_emacs_keybindings,
};
use std::io::{self, Write};

pub(crate) struct InteractiveLoopConfig<'a> {
    pub(crate) prompt: &'a OspPrompt,
    pub(crate) completion_tree: CompletionTree,
    pub(crate) appearance: ReplAppearance,
    pub(crate) complete_on: ReplCompleteOn,
    pub(crate) autosuggest: bool,
    pub(crate) line_projector: Option<LineProjector>,
    pub(crate) actions: Vec<crate::repl::ReplAction>,
    pub(crate) after_paint: Box<dyn FnMut() + Send>,
}

pub(crate) struct SubmissionContext<'a, F> {
    pub(crate) history_store: &'a SharedHistory,
    pub(crate) root_input: bool,
    pub(crate) exit_on_escape: bool,
    pub(crate) execute: &'a mut F,
}

pub(crate) fn evaluate_repl_submission<F>(
    raw: &str,
    ctx: &mut SubmissionContext<'_, F>,
) -> ReplResult
where
    F: FnMut(crate::repl::ReplEvent<'_>, &SharedHistory) -> Result<ReplResult>,
{
    let raw = raw.trim_end();
    if raw.trim().is_empty() {
        return ReplResult::Continue;
    }
    let event = if std::mem::take(&mut ctx.root_input) {
        crate::repl::ReplEvent::RootLine(raw)
    } else {
        crate::repl::ReplEvent::Line(raw)
    };
    if std::io::IsTerminal::is_terminal(&std::io::stdin())
        && let Err(error) = crate::ui::interrupt::prepare()
    {
        StdIoUiSink.write_stderr(&format!("{error}\n"));
        return ReplResult::Continue;
    }
    match (ctx.execute)(event, ctx.history_store) {
        Ok(result) => result,
        Err(err) => {
            StdIoUiSink.write_stderr(&format!("{err}\n"));
            ReplResult::Continue
        }
    }
}

pub(crate) fn run_repl_interactive<F>(
    config: InteractiveLoopConfig<'_>,
    history_store: SharedHistory,
    submission: &mut SubmissionContext<'_, F>,
) -> Result<ReplResult>
where
    F: FnMut(crate::repl::ReplEvent<'_>, &SharedHistory) -> Result<ReplResult>,
{
    let InteractiveLoopConfig {
        prompt,
        completion_tree,
        appearance,
        complete_on,
        autosuggest,
        line_projector,
        actions,
        after_paint,
    } = config;

    let mut bindings = build_repl_keybindings();
    if submission.exit_on_escape {
        bindings.add_binding(KeyModifiers::NONE, KeyCode::Esc, ReedlineEvent::CtrlD);
    }
    let mut action_keys = std::collections::HashSet::new();
    for action in &actions {
        if let Some(key) = action.key {
            let (modifiers, code) = match key {
                crate::repl::ReplActionKey::Function(key) => (KeyModifiers::NONE, KeyCode::F(key)),
                crate::repl::ReplActionKey::AltEnter => (KeyModifiers::ALT, KeyCode::Enter),
                crate::repl::ReplActionKey::Alt(key) => (KeyModifiers::ALT, KeyCode::Char(key)),
            };
            anyhow::ensure!(
                action_keys.insert((modifiers, code))
                    && (matches!(key, crate::repl::ReplActionKey::Alt(_))
                        || bindings.find_binding(modifiers, code).is_none()),
                "conflicting action key {key}"
            );
            bindings.add_binding(
                modifiers,
                code,
                ReedlineEvent::ExecuteHostCommand(format!("\0osp-action:{}", action.id)),
            );
        }
    }
    let mut editor = {
        let _phase = tracing::info_span!(target: "osp_startup", "editor construction").entered();
        build_interactive_editor(
            completion_tree,
            &appearance,
            complete_on,
            autosuggest,
            line_projector,
            history_store.clone(),
            Some(bindings),
        )
    };
    // Reedline enters the idle callback after flushing its initial repaint.
    let mut after_paint = after_paint;
    let prompt_left = prompt.left().to_string();
    editor = editor.with_idle_callback(Box::new(move || {
        after_paint();
        crate::repl::notices::flush(&prompt_left);
    }));
    drive_interactive_editor(
        &appearance,
        &history_store,
        submission,
        &mut editor,
        |editor| editor.read_line(prompt),
    )
}

pub(super) fn build_interactive_editor(
    completion_tree: CompletionTree,
    appearance: &ReplAppearance,
    complete_on: ReplCompleteOn,
    autosuggest: bool,
    line_projector: Option<LineProjector>,
    history_store: SharedHistory,
    bindings: Option<reedline::Keybindings>,
) -> Reedline {
    let tree = std::sync::Arc::new(completion_tree);
    let completer = Box::new(
        ReplCompleter::new(tree.clone(), line_projector.clone())
            .with_command_prefixes(appearance.command_prefixes)
            .with_history(history_store.clone()),
    );
    let completion_menu = SharedCompletionMenu::new(build_completion_menu(appearance));
    let painted = PaintedLine::default();
    let hinter = build_repl_hinter(
        &tree,
        appearance,
        line_projector.clone(),
        painted.clone(),
        &history_store,
        autosuggest,
    );
    let highlighter = build_repl_highlighter(tree.clone(), appearance, line_projector);
    let edit_mode = Box::new(AutoCompleteEmacs::new(
        Emacs::new(bindings.unwrap_or_else(build_repl_keybindings)),
        completion_menu.clone(),
        complete_on,
        painted,
    ));

    // Partial completions route Tab on a closed menu through
    // `OspCompletionMenu::complete_like_shell`.
    let mut editor = Reedline::create()
        .with_completer(completer)
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(completion_menu)))
        .with_partial_completions(true)
        .with_ansi_colors(appearance.ansi_colors)
        .with_hinter(Box::new(hinter))
        .with_edit_mode(edit_mode);
    if let Some(highlighter) = highlighter {
        editor = editor.with_highlighter(Box::new(highlighter));
    }
    let printer = ExternalPrinter::default();
    crate::repl::notices::attach(&printer);
    editor
        .with_external_printer(printer)
        .with_history(Box::new(history_store))
        .with_history_exclusion_prefix(Some(" ".to_string()))
}

pub(super) fn drive_interactive_editor<F, R>(
    appearance: &ReplAppearance,
    history_store: &SharedHistory,
    submission: &mut SubmissionContext<'_, F>,
    editor: &mut Reedline,
    mut read_signal: R,
) -> Result<ReplResult>
where
    F: FnMut(crate::repl::ReplEvent<'_>, &SharedHistory) -> Result<ReplResult>,
    R: FnMut(&mut Reedline) -> io::Result<Signal>,
{
    crate::startup_trace::begin_first_prompt();
    let mut last_argument = None;
    loop {
        let signal = read_signal(editor)?;

        if let Signal::HostCommand(command) = &signal
            && matches!(
                command.as_str(),
                HOST_COMMAND_LAST_ARGUMENT | HOST_COMMAND_PREVIOUS_ARGUMENT
            )
        {
            if command == HOST_COMMAND_LAST_ARGUMENT {
                last_argument = None;
            }
            insert_last_argument(editor, history_store, &mut last_argument)?;
            continue;
        }

        let result =
            handle_interactive_signal(signal, editor, history_store, appearance, submission)?;
        crate::repl::bell::finish();
        if let Some(result) = result {
            return Ok(result);
        }
    }
}

pub(crate) fn run_repl_basic<F>(
    prompt: &OspPrompt,
    submission: &mut SubmissionContext<'_, F>,
    after_paint: Box<dyn FnMut() + Send>,
) -> Result<ReplResult>
where
    F: FnMut(crate::repl::ReplEvent<'_>, &SharedHistory) -> Result<ReplResult>,
{
    crate::repl::notices::attach_basic();
    let mut after_paint = Some(after_paint);
    crate::startup_trace::begin_first_prompt();
    loop {
        print!("{}{}", prompt.left(), prompt.indicator());
        io::stdout().flush()?;
        if let Some(mut after_paint) = after_paint.take() {
            after_paint();
        }

        let mut line = String::new();
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            crate::ui::interrupt::prepare()?;
        }
        let read = match crate::ui::prompt::read_line(&mut line) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                StdIoUiSink.write_stderr("\n");
                continue;
            }
            result => result?,
        };
        if read == 0 {
            return Ok(ReplResult::Exit(0));
        }

        submission
            .history_store
            .with(|store| store.stage_command_line(&line))
            .map_err(anyhow::Error::msg)??;
        if let Some(result) = apply_submission(evaluate_repl_submission(&line, submission), None) {
            return Ok(result);
        }
    }
}

fn build_repl_hinter(
    tree: &std::sync::Arc<CompletionTree>,
    appearance: &ReplAppearance,
    line_projector: Option<LineProjector>,
    painted: PaintedLine,
    history: &SharedHistory,
    autosuggest: bool,
) -> ReplHinter {
    let color = |spec: &Option<String>| spec.as_deref().and_then(color_from_style_spec);
    let error_color = color(&appearance.error_highlight_style).unwrap_or(Color::Red);
    ReplHinter::new(
        ReplCompleter::new(tree.clone(), line_projector.clone())
            .with_command_prefixes(appearance.command_prefixes)
            .with_history(history.clone()),
        ReplHighlighter::new(tree.clone(), Color::Default, error_color, line_projector)
            .with_command_prefixes(appearance.command_prefixes),
        painted,
        color(&appearance.hint_style).map(|hint| Style::new().fg(hint)),
        autosuggest,
    )
    .with_command_limit(appearance.command_hint_limit)
}

fn build_repl_keybindings() -> reedline::Keybindings {
    let mut keybindings = default_emacs_keybindings();
    // Lines are single-line commands; Alt+Enter is free for a host action.
    for modifiers in [KeyModifiers::ALT, KeyModifiers::SHIFT] {
        keybindings.remove_binding(modifiers, KeyCode::Enter);
    }
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Enter,
        // With a menu open reedline accepts the selection and closes the menu
        // instead of submitting; otherwise this submits the line.
        ReedlineEvent::Submit,
    );
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(COMPLETION_MENU_NAME.to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::BackTab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu(COMPLETION_MENU_NAME.to_string()),
            ReedlineEvent::MenuPrevious,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char(' '),
        ReedlineEvent::Menu(COMPLETION_MENU_NAME.to_string()),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('r'),
        ReedlineEvent::ExecuteHostCommand(HOST_COMMAND_HISTORY_PICKER.to_string()),
    );
    keybindings.add_binding(
        KeyModifiers::ALT,
        KeyCode::Char('.'),
        ReedlineEvent::ExecuteHostCommand(HOST_COMMAND_LAST_ARGUMENT.to_string()),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('l'),
        ReedlineEvent::ExecuteHostCommand(HOST_COMMAND_CLEAR_SCREEN.to_string()),
    );
    // Step through paginated results in both directions without retyping
    // `prev` / `next`, which any other command would otherwise reset.
    for (modifiers, previous, next) in [
        (KeyModifiers::SHIFT, KeyCode::Left, KeyCode::Right),
        (KeyModifiers::NONE, KeyCode::PageUp, KeyCode::PageDown),
    ] {
        keybindings.add_binding(
            modifiers,
            previous,
            ReedlineEvent::ExecuteHostCommand(HOST_COMMAND_PAGE_PREVIOUS.to_string()),
        );
        keybindings.add_binding(
            modifiers,
            next,
            ReedlineEvent::ExecuteHostCommand(HOST_COMMAND_PAGE_NEXT.to_string()),
        );
    }
    keybindings
}

struct LastArgument {
    next_entry: usize,
    inserted: std::ops::Range<usize>,
}

fn insert_last_argument(
    editor: &mut Reedline,
    history: &SharedHistory,
    cycle: &mut Option<LastArgument>,
) -> Result<()> {
    let cursor = editor.current_insertion_point();
    let cycle = cycle.get_or_insert(LastArgument {
        next_entry: 0,
        inserted: cursor..cursor,
    });
    let entries = history
        .with(|store| store.list_entries_for(store.shell_prefix().as_deref()))
        .map_err(anyhow::Error::msg)?;
    let Some(entry) = entries.iter().rev().nth(cycle.next_entry) else {
        return Ok(());
    };
    let Some(argument) = last_shell_word(&entry.command) else {
        return Ok(());
    };
    editor.run_edit_commands(&[
        EditCommand::MoveToPosition {
            position: cycle.inserted.start,
            select: false,
        },
        EditCommand::MoveToPosition {
            position: cycle.inserted.end,
            select: true,
        },
        EditCommand::InsertString(argument.to_string()),
    ]);
    cycle.inserted.end = cycle.inserted.start + argument.len();
    cycle.next_entry += 1;
    Ok(())
}

fn last_shell_word(line: &str) -> Option<&str> {
    use crate::core::shell_words::{LexEventKind, scan_shell};

    let mut start = None;
    let mut last = None;
    // Keep the original quotes and escapes so inserting the word preserves it.
    scan_shell(line, |event| {
        if event.kind == LexEventKind::Separator {
            start = None;
        } else {
            let start = *start.get_or_insert(event.index);
            last = Some(start..event.end);
        }
    });
    last.map(|span| &line[span])
}

fn handle_interactive_signal<F>(
    signal: Signal,
    editor: &mut Reedline,
    history_store: &SharedHistory,
    appearance: &ReplAppearance,
    submission: &mut SubmissionContext<'_, F>,
) -> Result<Option<ReplResult>>
where
    F: FnMut(crate::repl::ReplEvent<'_>, &SharedHistory) -> Result<ReplResult>,
{
    match signal {
        Signal::HostCommand(line) if line == HOST_COMMAND_CLEAR_SCREEN => {
            clear_repl_screen(editor)?;
            Ok(None)
        }
        Signal::HostCommand(line) if line.starts_with("\0osp-action:") => {
            let draft = editor.current_buffer_contents().to_string();
            let cursor = editor.current_insertion_point();
            let id = line.trim_start_matches("\0osp-action:");
            let edited = id == "hint.insert:edited";
            let id = if edited { "hint.insert" } else { id };
            let event = if id == "hint.insert" {
                crate::repl::ReplEvent::InsertHint {
                    draft: &draft,
                    edited,
                }
            } else {
                crate::repl::ReplEvent::Action(id)
            };
            let result = match (submission.execute)(event, history_store) {
                Ok(ReplResult::Print(output)) => {
                    print!("{output}");
                    None
                }
                Ok(ReplResult::ReplaceInput(command)) if id == "hint.insert" => {
                    editor.run_edit_commands(&[
                        EditCommand::Clear,
                        EditCommand::InsertString(command),
                    ]);
                    return Ok(None);
                }
                Ok(ReplResult::Continue | ReplResult::ReplaceInput(_)) => None,
                Ok(result) => Some(result),
                Err(error) => {
                    StdIoUiSink.write_stderr(&format!("{error}\n"));
                    None
                }
            };
            if editor.current_buffer_contents() != draft {
                editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(draft)]);
            }
            editor.run_edit_commands(&[EditCommand::MoveToPosition {
                position: cursor,
                select: false,
            }]);
            Ok(result)
        }
        Signal::HostCommand(line) if line == HOST_COMMAND_HISTORY_PICKER => {
            // `Ctrl-R` leaves reedline through a private host command so we
            // can temporarily hand terminal ownership to skim. Once skim
            // returns, submit the selected command or restore it for editing.
            let current_line = editor.current_buffer_contents().to_string();
            let selected = match launch_history_picker(history_store, appearance, &current_line) {
                Ok(selected) => selected,
                Err(err) => {
                    StdIoUiSink.write_stderr(&format!("{}\n", render_history_picker_failure(&err)));
                    return Ok(None);
                }
            };
            match selected {
                Some(HistorySelection::Run { command, root }) => {
                    submission.root_input = root;
                    StdIoUiSink.write_stderr(&format!("{command}\n"));
                    history_store
                        .with(|store| store.stage_command_line(&command))
                        .map_err(anyhow::Error::msg)??;
                    editor.run_edit_commands(&[EditCommand::Clear]);
                    return handle_interactive_signal(
                        Signal::Success(command),
                        editor,
                        history_store,
                        appearance,
                        submission,
                    );
                }
                Some(HistorySelection::Edit { command, root }) => {
                    submission.root_input = root;
                    editor.run_edit_commands(&[
                        EditCommand::Clear,
                        EditCommand::InsertString(command),
                    ]);
                }
                None => {}
            }
            Ok(None)
        }
        Signal::HostCommand(line)
            if line == HOST_COMMAND_PAGE_PREVIOUS || line == HOST_COMMAND_PAGE_NEXT =>
        {
            let previous = line == HOST_COMMAND_PAGE_PREVIOUS;
            if !editor.current_buffer_contents().is_empty() {
                // While typing, the keys keep their ordinary editing meaning.
                editor.run_edit_commands(&[if previous {
                    EditCommand::MoveLeft { select: false }
                } else {
                    EditCommand::MoveRight { select: false }
                }]);
                return Ok(None);
            }
            let id = if previous {
                "pagination.prev"
            } else {
                "pagination.next"
            };
            let result = (submission.execute)(crate::repl::ReplEvent::Action(id), history_store)
                .unwrap_or_else(|err| {
                    StdIoUiSink.write_stderr(&format!("{err}\n"));
                    ReplResult::Continue
                });
            Ok(apply_submission(result, Some(editor)))
        }
        Signal::Success(line) => Ok(apply_submission(
            evaluate_repl_submission(&line, submission),
            Some(editor),
        )),
        Signal::CtrlD => Ok(Some(ReplResult::Exit(0))),
        Signal::CtrlC => {
            submission.root_input = false;
            Ok(None)
        }
        // No other host commands are bound and no external break is wired.
        _ => Ok(None),
    }
}

fn clear_repl_screen(editor: &mut Reedline) -> io::Result<()> {
    io::stdout().write_all(crate::repl::screen::clear_sequence().as_bytes())?;
    io::stdout().flush()?;
    // Reedline resets its painter origin after clearing the now-blank display.
    editor.clear_screen()
}

fn render_history_picker_failure(err: &anyhow::Error) -> String {
    format!(
        "History picker unavailable: {}. Continue typing normally or rerun the command directly.",
        err.root_cause()
    )
}

fn apply_submission(result: ReplResult, editor: Option<&mut Reedline>) -> Option<ReplResult> {
    match result {
        ReplResult::Continue => None,
        ReplResult::Print(output) => {
            print!("{output}");
            None
        }
        ReplResult::ReplaceInput(buffer) => {
            if let Some(editor) = editor {
                editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(buffer)]);
            } else {
                println!("{buffer}");
            }
            None
        }
        result @ (ReplResult::Exit(_) | ReplResult::Restart { .. }) => Some(result),
    }
}

/// Standalone file input shares the REPL's completion menu and navigation.
pub(crate) fn read_path(label: &str) -> io::Result<Option<String>> {
    use reedline::{Completer, CompletionResult, Span, Suggestion};
    struct Paths;
    impl Completer for Paths {
        fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
            CompletionResult::fresh(
                crate::completion::path_candidates(&line[..pos])
                    .into_iter()
                    .map(|(value, directory)| Suggestion {
                        value,
                        span: Span::new(0, pos),
                        append_whitespace: false,
                        description: Some(if directory { "dir" } else { "file" }.into()),
                        ..Suggestion::default()
                    })
                    .collect::<Vec<_>>(),
            )
        }
    }
    let menu = SharedCompletionMenu::new(build_completion_menu(&ReplAppearance::default()));
    let mut bindings = build_repl_keybindings();
    bindings.add_binding(KeyModifiers::NONE, KeyCode::Esc, ReedlineEvent::CtrlC);
    let mode = AutoCompleteEmacs::new(
        Emacs::new(bindings),
        menu.clone(),
        ReplCompleteOn {
            after_letters: Some(1),
            space: true,
            ..ReplCompleteOn::default()
        },
        PaintedLine::default(),
    );
    let mut editor = Reedline::create()
        .with_completer(Box::new(Paths))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
        .with_partial_completions(true)
        .with_edit_mode(Box::new(mode));
    let prompt = OspPrompt::new(label.into(), String::new(), None);
    loop {
        match editor.read_line(&prompt)? {
            Signal::HostCommand(command) if command == HOST_COMMAND_CLEAR_SCREEN => {
                clear_repl_screen(&mut editor)?;
            }
            Signal::Success(path) => return Ok(Some(path)),
            _ => return Ok(None),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::repl::engine::{HistoryConfig, ReplReloadKind};
    use reedline::{KeyCode, KeyModifiers, ReedlineEvent};
    use std::sync::{Arc, Mutex};

    fn disabled_history() -> SharedHistory {
        SharedHistory::new(HistoryConfig {
            enabled: false,
            max_entries: 0,
            ..HistoryConfig::default()
        })
    }

    fn test_appearance() -> ReplAppearance {
        ReplAppearance::builder()
            .with_completion_text_style(Some("white".to_string()))
            .with_completion_background_style(Some("black".to_string()))
            .with_completion_highlight_style(Some("cyan".to_string()))
            .with_history_menu_rows(5)
            .build()
    }

    #[test]
    fn repl_keybindings_wire_completion_and_history_shortcuts_unit() {
        let keybindings = build_repl_keybindings();

        assert!(matches!(
            keybindings.find_binding(KeyModifiers::NONE, KeyCode::Enter),
            Some(ReedlineEvent::Submit)
        ));
        assert!(matches!(
            keybindings.find_binding(KeyModifiers::NONE, KeyCode::Tab),
            Some(ReedlineEvent::UntilFound(_))
        ));
        assert!(matches!(
            keybindings.find_binding(KeyModifiers::SHIFT, KeyCode::BackTab),
            Some(ReedlineEvent::UntilFound(_))
        ));
        assert!(matches!(
            keybindings.find_binding(KeyModifiers::CONTROL, KeyCode::Char(' ')),
            Some(ReedlineEvent::Menu(name)) if name == COMPLETION_MENU_NAME
        ));
        assert!(matches!(
            keybindings.find_binding(KeyModifiers::CONTROL, KeyCode::Char('r')),
            Some(ReedlineEvent::ExecuteHostCommand(name)) if name == HOST_COMMAND_HISTORY_PICKER
        ));
    }

    #[test]
    fn apply_submission_covers_editor_update_and_terminal_outcomes_unit() {
        let mut editor = Reedline::create();

        assert!(apply_submission(ReplResult::Continue, Some(&mut editor)).is_none());
        assert!(
            apply_submission(ReplResult::Print("printed".to_string()), Some(&mut editor)).is_none()
        );

        assert!(
            apply_submission(
                ReplResult::ReplaceInput("config show".to_string()),
                Some(&mut editor),
            )
            .is_none()
        );
        assert_eq!(editor.current_buffer_contents(), "config show");

        assert!(matches!(
            apply_submission(ReplResult::Exit(7), Some(&mut editor)),
            Some(ReplResult::Exit(7))
        ));
        assert!(matches!(
            apply_submission(
                ReplResult::Restart {
                    output: "reload".to_string(),
                    reload: ReplReloadKind::Default,
                },
                Some(&mut editor),
            ),
            Some(ReplResult::Restart { output, reload: ReplReloadKind::Default })
                if output == "reload"
        ));
    }

    #[test]
    fn interactive_signal_paths_cover_host_picker_submit_and_ctrl_events_unit() {
        let history = disabled_history();
        let appearance = test_appearance();
        let mut editor = Reedline::create();
        editor.run_edit_commands(&[EditCommand::InsertString("typed".to_string())]);

        let executed = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&executed);
        let mut execute = move |event: crate::repl::ReplEvent<'_>, _: &SharedHistory| {
            let crate::repl::ReplEvent::Line(line) = event else {
                unreachable!()
            };
            seen.lock()
                .expect("executed command log should not be poisoned")
                .push(line.to_string());
            Ok(ReplResult::Continue)
        };
        let mut submission = SubmissionContext {
            root_input: false,
            exit_on_escape: true,
            history_store: &history,
            execute: &mut execute,
        };

        assert!(
            handle_interactive_signal(
                Signal::HostCommand(HOST_COMMAND_HISTORY_PICKER.to_string()),
                &mut editor,
                &history,
                &appearance,
                &mut submission,
            )
            .expect("host picker should succeed")
            .is_none()
        );
        assert_eq!(editor.current_buffer_contents(), "typed");

        assert!(
            handle_interactive_signal(
                Signal::Success("help".to_string()),
                &mut editor,
                &history,
                &appearance,
                &mut submission,
            )
            .expect("submission should succeed")
            .is_none()
        );
        assert_eq!(
            *executed
                .lock()
                .expect("executed command log should not be poisoned"),
            vec!["help".to_string()]
        );

        assert!(matches!(
            handle_interactive_signal(
                Signal::CtrlD,
                &mut editor,
                &history,
                &appearance,
                &mut submission,
            )
            .expect("ctrl-d should succeed"),
            Some(ReplResult::Exit(0))
        ));
        assert!(
            handle_interactive_signal(
                Signal::CtrlC,
                &mut editor,
                &history,
                &appearance,
                &mut submission,
            )
            .expect("ctrl-c should succeed")
            .is_none()
        );
    }

    #[test]
    fn history_picker_failure_message_stays_local_and_actionable_unit() {
        let err = anyhow::anyhow!("failed to launch REPL history picker: no tty");
        let rendered = render_history_picker_failure(&err);

        assert!(rendered.contains("History picker unavailable"));
        assert!(rendered.contains("no tty"));
        assert!(rendered.contains("Continue typing normally"));
    }
}
