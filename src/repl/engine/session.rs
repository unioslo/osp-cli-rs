use super::adapter::{ReplCompleter, build_repl_highlighter, build_repl_tree};
use super::config::{LineProjector, ReplAppearance, ReplLineResult, ReplReloadKind, ReplRunResult};
use super::editor::{AutoCompleteEmacs, OspPrompt, is_cursor_position_error};
use super::overlay::{build_completion_menu, launch_history_picker};
use super::{COMPLETION_MENU_NAME, HOST_COMMAND_HISTORY_PICKER, SharedHistory};
use crate::completion::CompletionTree;
use anyhow::Result;
use reedline::{
    EditCommand, Emacs, KeyCode, KeyModifiers, Reedline, ReedlineEvent, ReedlineMenu, Signal,
    default_emacs_keybindings,
};
use std::io::{self, Write};

pub(crate) struct InteractiveLoopConfig<'a> {
    pub(crate) prompt: &'a OspPrompt,
    pub(crate) completion_words: Vec<String>,
    pub(crate) completion_tree: Option<CompletionTree>,
    pub(crate) appearance: ReplAppearance,
    pub(crate) line_projector: Option<LineProjector>,
}

pub(crate) enum SubmissionResult {
    Noop,
    Print(String),
    ReplaceInput(String),
    Exit(i32),
    Restart {
        output: String,
        reload: ReplReloadKind,
    },
}

pub(crate) struct SubmissionContext<'a, F> {
    pub(crate) history_store: &'a SharedHistory,
    pub(crate) execute: &'a mut F,
}

impl<'a, F> SubmissionContext<'a, F> where F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult> {}

pub(crate) fn process_submission<F>(
    raw: &str,
    ctx: &mut SubmissionContext<'_, F>,
) -> Result<SubmissionResult>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(SubmissionResult::Noop);
    }
    let result = match (ctx.execute)(raw, ctx.history_store) {
        Ok(ReplLineResult::Continue(output)) => SubmissionResult::Print(output),
        Ok(ReplLineResult::ReplaceInput(buffer)) => SubmissionResult::ReplaceInput(buffer),
        Ok(ReplLineResult::Exit(code)) => SubmissionResult::Exit(code),
        Ok(ReplLineResult::Restart { output, reload }) => {
            SubmissionResult::Restart { output, reload }
        }
        Err(err) => {
            eprintln!("{err}");
            SubmissionResult::Noop
        }
    };
    Ok(result)
}

pub(crate) fn run_repl_interactive<F>(
    config: InteractiveLoopConfig<'_>,
    history_store: SharedHistory,
    submission: &mut SubmissionContext<'_, F>,
) -> Result<ReplRunResult>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    let InteractiveLoopConfig {
        prompt,
        completion_words,
        completion_tree,
        appearance,
        line_projector,
    } = config;

    let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&completion_words));
    let completer = Box::new(ReplCompleter::new(
        completion_words,
        Some(tree.clone()),
        line_projector.clone(),
    ));
    let completion_menu = Box::new(build_completion_menu(&appearance));
    let highlighter = build_repl_highlighter(&tree, &appearance, line_projector);
    let edit_mode = Box::new(AutoCompleteEmacs::new(
        Emacs::new(build_repl_keybindings()),
        COMPLETION_MENU_NAME,
    ));

    let mut editor = Reedline::create()
        .with_completer(completer)
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode);
    if let Some(highlighter) = highlighter {
        editor = editor.with_highlighter(Box::new(highlighter));
    }
    editor = editor.with_history(Box::new(history_store.clone()));

    loop {
        let signal = match editor.read_line(prompt) {
            Ok(signal) => signal,
            Err(err) => {
                if is_cursor_position_error(&err) {
                    eprintln!(
                        "WARNING: terminal does not support cursor position requests; \
falling back to basic input mode."
                    );
                    run_repl_basic(prompt, submission)?;
                    return Ok(ReplRunResult::Exit(0));
                }
                return Err(err.into());
            }
        };

        if let Some(result) =
            handle_interactive_signal(signal, &mut editor, &history_store, &appearance, submission)?
        {
            return Ok(result);
        }
    }
}

pub(crate) fn run_repl_basic<F>(
    prompt: &OspPrompt,
    submission: &mut SubmissionContext<'_, F>,
) -> Result<()>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    let stdin = io::stdin();
    loop {
        print!("{}{}", prompt.left(), prompt.indicator());
        io::stdout().flush()?;

        let mut line = String::new();
        let read = stdin.read_line(&mut line)?;
        if read == 0 {
            break;
        }

        match process_submission(&line, submission)? {
            SubmissionResult::Noop => continue,
            SubmissionResult::Print(output) => print!("{output}"),
            SubmissionResult::ReplaceInput(buffer) => {
                println!("{buffer}");
                continue;
            }
            SubmissionResult::Exit(_) => break,
            SubmissionResult::Restart { output, .. } => {
                print!("{output}");
                break;
            }
        }
    }
    Ok(())
}

fn build_repl_keybindings() -> reedline::Keybindings {
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Enter,
        ReedlineEvent::Multiple(vec![ReedlineEvent::Esc, ReedlineEvent::Submit]),
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
    keybindings
}

fn handle_interactive_signal<F>(
    signal: Signal,
    editor: &mut Reedline,
    history_store: &SharedHistory,
    appearance: &ReplAppearance,
    submission: &mut SubmissionContext<'_, F>,
) -> Result<Option<ReplRunResult>>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    match signal {
        Signal::Success(line) if line == HOST_COMMAND_HISTORY_PICKER => {
            // `Ctrl-R` leaves reedline through a private host command so we
            // can temporarily hand terminal ownership to skim. Once skim
            // returns, restore the chosen command into the live editor buffer.
            let current_line = editor.current_buffer_contents().to_string();
            let selected = launch_history_picker(history_store, appearance, &current_line)?;
            if let Some(command) = selected {
                editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(command)]);
            }
            Ok(None)
        }
        Signal::Success(line) => {
            handle_submission_result(process_submission(&line, submission)?, editor)
        }
        Signal::CtrlD => Ok(Some(ReplRunResult::Exit(0))),
        Signal::CtrlC => Ok(None),
    }
}

fn handle_submission_result(
    result: SubmissionResult,
    editor: &mut Reedline,
) -> Result<Option<ReplRunResult>> {
    match result {
        SubmissionResult::Noop => Ok(None),
        SubmissionResult::Print(output) => {
            print!("{output}");
            Ok(None)
        }
        SubmissionResult::ReplaceInput(buffer) => {
            editor.run_edit_commands(&[EditCommand::Clear, EditCommand::InsertString(buffer)]);
            Ok(None)
        }
        SubmissionResult::Exit(code) => Ok(Some(ReplRunResult::Exit(code))),
        SubmissionResult::Restart { output, reload } => {
            Ok(Some(ReplRunResult::Restart { output, reload }))
        }
    }
}
