use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
mod highlight;
mod history;
mod menu;
mod menu_core;
use highlight::ReplHighlighter;
pub use highlight::{HighlightDebugSpan, debug_highlight};
pub use history::{
    HistoryConfig, HistoryEntry, HistoryShellContext, OspHistoryStore, SharedHistory,
    expand_history,
};
use menu::{MenuDebug, MenuStyleDebug, OspCompletionMenu, debug_snapshot, display_text};
use nu_ansi_term::{Color, Style};
use osp_completion::{
    ArgNode, CompletionEngine, CompletionNode, CompletionTree, SuggestionEntry, SuggestionOutput,
};
use reedline::{
    Completer, EditCommand, EditMode, Editor, Emacs, KeyCode, KeyModifiers, Menu, MenuEvent,
    Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, ReedlineMenu, ReedlineRawEvent, Signal, Span, Suggestion, UndoBehavior,
    default_emacs_keybindings,
};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct ReplPrompt {
    pub left: String,
    pub indicator: String,
}

pub type PromptRightRenderer = Arc<dyn Fn() -> String + Send + Sync>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineProjection {
    pub line: String,
    pub hidden_suggestions: BTreeSet<String>,
}

impl LineProjection {
    pub fn passthrough(line: impl Into<String>) -> Self {
        Self {
            line: line.into(),
            hidden_suggestions: BTreeSet::new(),
        }
    }

    pub fn with_hidden_suggestions(mut self, hidden_suggestions: BTreeSet<String>) -> Self {
        self.hidden_suggestions = hidden_suggestions;
        self
    }
}

pub type LineProjector = Arc<dyn Fn(&str) -> LineProjection + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplInputMode {
    Auto,
    Interactive,
    Basic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplReloadKind {
    Default,
    WithIntro,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplLineResult {
    Continue(String),
    ReplaceInput(String),
    Exit(i32),
    Restart {
        output: String,
        reload: ReplReloadKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplRunResult {
    Exit(i32),
    Restart {
        output: String,
        reload: ReplReloadKind,
    },
}

pub struct ReplRunConfig {
    pub prompt: ReplPrompt,
    pub completion_words: Vec<String>,
    pub completion_tree: Option<CompletionTree>,
    pub appearance: ReplAppearance,
    pub history_config: HistoryConfig,
    pub input_mode: ReplInputMode,
    pub prompt_right: Option<PromptRightRenderer>,
    pub line_projector: Option<LineProjector>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionDebugMatch {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionDebug {
    pub line: String,
    pub cursor: usize,
    pub replace_range: [usize; 2],
    pub stub: String,
    pub matches: Vec<CompletionDebugMatch>,
    pub selected: i64,
    pub selected_row: u16,
    pub selected_col: u16,
    pub columns: u16,
    pub rows: u16,
    pub visible_rows: u16,
    pub menu_indent: u16,
    pub menu_styles: MenuStyleDebug,
    pub menu_description: Option<String>,
    pub menu_description_rendered: Option<String>,
    pub width: u16,
    pub height: u16,
    pub unicode: bool,
    pub color: bool,
    pub rendered: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugStep {
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Accept,
    Close,
}

impl DebugStep {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tab" => Some(Self::Tab),
            "backtab" | "shift-tab" | "shift_tab" => Some(Self::BackTab),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "accept" | "enter" => Some(Self::Accept),
            "close" | "esc" | "escape" => Some(Self::Close),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tab => "tab",
            Self::BackTab => "backtab",
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::Accept => "accept",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionDebugFrame {
    pub step: String,
    pub state: CompletionDebug,
}

#[derive(Debug, Clone, Copy)]
pub struct CompletionDebugOptions<'a> {
    pub width: u16,
    pub height: u16,
    pub ansi: bool,
    pub unicode: bool,
    pub appearance: Option<&'a ReplAppearance>,
}

impl<'a> CompletionDebugOptions<'a> {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            ansi: false,
            unicode: false,
            appearance: None,
        }
    }

    pub fn ansi(mut self, ansi: bool) -> Self {
        self.ansi = ansi;
        self
    }

    pub fn unicode(mut self, unicode: bool) -> Self {
        self.unicode = unicode;
        self
    }

    pub fn appearance(mut self, appearance: Option<&'a ReplAppearance>) -> Self {
        self.appearance = appearance;
        self
    }
}

pub fn debug_completion(
    tree: &CompletionTree,
    line: &str,
    cursor: usize,
    options: CompletionDebugOptions<'_>,
) -> CompletionDebug {
    let (editor, mut completer, mut menu) =
        build_debug_completion_session(tree, line, cursor, options.appearance);
    let mut editor = editor;

    menu.menu_event(MenuEvent::Activate(false));
    menu.apply_event(&mut editor, &mut completer);

    snapshot_completion_debug(
        tree,
        &mut menu,
        &editor,
        options.width,
        options.height,
        options.ansi,
        options.unicode,
    )
}

pub fn debug_completion_steps(
    tree: &CompletionTree,
    line: &str,
    cursor: usize,
    options: CompletionDebugOptions<'_>,
    steps: &[DebugStep],
) -> Vec<CompletionDebugFrame> {
    let (mut editor, mut completer, mut menu) =
        build_debug_completion_session(tree, line, cursor, options.appearance);

    let steps = steps.to_vec();
    if steps.is_empty() {
        return Vec::new();
    }

    let mut frames = Vec::with_capacity(steps.len());
    for step in steps {
        apply_debug_step(step, &mut menu, &mut editor, &mut completer);
        let state = snapshot_completion_debug(
            tree,
            &mut menu,
            &editor,
            options.width,
            options.height,
            options.ansi,
            options.unicode,
        );
        frames.push(CompletionDebugFrame {
            step: step.as_str().to_string(),
            state,
        });
    }

    frames
}

fn build_debug_completion_session(
    tree: &CompletionTree,
    line: &str,
    cursor: usize,
    appearance: Option<&ReplAppearance>,
) -> (Editor, ReplCompleter, OspCompletionMenu) {
    let mut editor = Editor::default();
    editor.edit_buffer(
        |buf| {
            buf.set_buffer(line.to_string());
            buf.set_insertion_point(cursor.min(buf.get_buffer().len()));
        },
        UndoBehavior::CreateUndoPoint,
    );

    let completer = ReplCompleter::new(Vec::new(), Some(tree.clone()), None);
    let menu = if let Some(appearance) = appearance {
        build_completion_menu(appearance)
    } else {
        OspCompletionMenu::default()
    };

    (editor, completer, menu)
}

fn apply_debug_step(
    step: DebugStep,
    menu: &mut OspCompletionMenu,
    editor: &mut Editor,
    completer: &mut ReplCompleter,
) {
    match step {
        DebugStep::Tab => {
            if menu.is_active() {
                dispatch_menu_event(menu, editor, completer, MenuEvent::NextElement);
            } else {
                dispatch_menu_event(menu, editor, completer, MenuEvent::Activate(false));
            }
        }
        DebugStep::BackTab => {
            if menu.is_active() {
                dispatch_menu_event(menu, editor, completer, MenuEvent::PreviousElement);
            } else {
                dispatch_menu_event(menu, editor, completer, MenuEvent::Activate(false));
            }
        }
        DebugStep::Up => {
            if menu.is_active() {
                dispatch_menu_event(menu, editor, completer, MenuEvent::MoveUp);
            }
        }
        DebugStep::Down => {
            if menu.is_active() {
                dispatch_menu_event(menu, editor, completer, MenuEvent::MoveDown);
            }
        }
        DebugStep::Left => {
            if menu.is_active() {
                dispatch_menu_event(menu, editor, completer, MenuEvent::MoveLeft);
            }
        }
        DebugStep::Right => {
            if menu.is_active() {
                dispatch_menu_event(menu, editor, completer, MenuEvent::MoveRight);
            }
        }
        DebugStep::Accept => {
            if menu.is_active() {
                menu.accept_selection_in_buffer(editor);
                dispatch_menu_event(menu, editor, completer, MenuEvent::Deactivate);
            }
        }
        DebugStep::Close => {
            dispatch_menu_event(menu, editor, completer, MenuEvent::Deactivate);
        }
    }
}

fn dispatch_menu_event(
    menu: &mut OspCompletionMenu,
    editor: &mut Editor,
    completer: &mut ReplCompleter,
    event: MenuEvent,
) {
    menu.menu_event(event);
    menu.apply_event(editor, completer);
}

fn snapshot_completion_debug(
    tree: &CompletionTree,
    menu: &mut OspCompletionMenu,
    editor: &Editor,
    width: u16,
    height: u16,
    ansi: bool,
    unicode: bool,
) -> CompletionDebug {
    let line = editor.get_buffer().to_string();
    let cursor = editor.line_buffer().insertion_point();
    let values = menu.get_values();
    let engine = CompletionEngine::new(tree.clone());
    let analysis = engine.analyze(&line, cursor);

    let (stub, replace_range) = if let Some(first) = values.first() {
        let start = first.span.start;
        let end = first.span.end;
        let stub = line.get(start..end).unwrap_or("").to_string();
        (stub, [start, end])
    } else {
        (
            analysis.cursor.raw_stub.clone(),
            [
                analysis.cursor.replace_range.start,
                analysis.cursor.replace_range.end,
            ],
        )
    };

    let matches = values
        .iter()
        .map(|item| CompletionDebugMatch {
            id: item.value.clone(),
            label: display_text(item).to_string(),
            description: item.description.clone(),
            kind: engine
                .classify_match(&analysis, &item.value)
                .as_str()
                .to_string(),
        })
        .collect::<Vec<_>>();

    let MenuDebug {
        columns,
        rows,
        visible_rows,
        indent,
        selected_index,
        selected_row,
        selected_col,
        description,
        description_rendered,
        styles,
        rendered,
    } = debug_snapshot(menu, editor, width, height, ansi);

    let selected = if matches.is_empty() {
        -1
    } else {
        selected_index
    };

    CompletionDebug {
        line,
        cursor,
        replace_range,
        stub,
        matches,
        selected,
        selected_row,
        selected_col,
        columns,
        rows,
        visible_rows,
        menu_indent: indent,
        menu_styles: styles,
        menu_description: description,
        menu_description_rendered: description_rendered,
        width,
        height,
        unicode,
        color: ansi,
        rendered,
    }
}

impl ReplPrompt {
    pub fn simple(left: impl Into<String>) -> Self {
        Self {
            left: left.into(),
            indicator: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReplAppearance {
    pub completion_text_style: Option<String>,
    pub completion_background_style: Option<String>,
    pub completion_highlight_style: Option<String>,
    pub command_highlight_style: Option<String>,
}

struct AutoCompleteEmacs {
    inner: Emacs,
    menu_name: String,
}

impl AutoCompleteEmacs {
    fn new(inner: Emacs, menu_name: impl Into<String>) -> Self {
        Self {
            inner,
            menu_name: menu_name.into(),
        }
    }

    fn should_reopen_menu(commands: &[EditCommand]) -> bool {
        commands.iter().any(|cmd| {
            matches!(
                cmd,
                EditCommand::InsertChar(_)
                    | EditCommand::InsertString(_)
                    | EditCommand::ReplaceChar(_)
                    | EditCommand::ReplaceChars(_, _)
                    | EditCommand::Backspace
                    | EditCommand::Delete
                    | EditCommand::CutChar
                    | EditCommand::BackspaceWord
                    | EditCommand::DeleteWord
                    | EditCommand::Clear
                    | EditCommand::ClearToLineEnd
                    | EditCommand::CutCurrentLine
                    | EditCommand::CutFromStart
                    | EditCommand::CutFromLineStart
                    | EditCommand::CutToEnd
                    | EditCommand::CutToLineEnd
                    | EditCommand::CutWordLeft
                    | EditCommand::CutBigWordLeft
                    | EditCommand::CutWordRight
                    | EditCommand::CutBigWordRight
                    | EditCommand::CutWordRightToNext
                    | EditCommand::CutBigWordRightToNext
                    | EditCommand::PasteCutBufferBefore
                    | EditCommand::PasteCutBufferAfter
                    | EditCommand::Undo
                    | EditCommand::Redo
            )
        })
    }
}

impl EditMode for AutoCompleteEmacs {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let parsed = self.inner.parse_event(event);
        match parsed {
            ReedlineEvent::Edit(commands) if Self::should_reopen_menu(&commands) => {
                ReedlineEvent::Multiple(vec![
                    ReedlineEvent::Edit(commands),
                    ReedlineEvent::Menu(self.menu_name.clone()),
                ])
            }
            other => other,
        }
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.inner.edit_mode()
    }
}

enum SubmissionResult {
    Noop,
    Print(String),
    ReplaceInput(String),
    Exit(i32),
    Restart {
        output: String,
        reload: ReplReloadKind,
    },
}

struct SubmissionContext<'a, F> {
    history_store: &'a SharedHistory,
    execute: &'a mut F,
}

impl<'a, F> SubmissionContext<'a, F> where F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult> {}

fn process_submission<F>(raw: &str, ctx: &mut SubmissionContext<'_, F>) -> Result<SubmissionResult>
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

pub fn run_repl<F>(config: ReplRunConfig, mut execute: F) -> Result<ReplRunResult>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    let ReplRunConfig {
        prompt,
        completion_words,
        completion_tree,
        appearance,
        history_config,
        input_mode,
        prompt_right,
        line_projector,
    } = config;
    let history_store = SharedHistory::new(history_config)?;
    let mut submission = SubmissionContext {
        history_store: &history_store,
        execute: &mut execute,
    };
    let prompt = OspPrompt::new(prompt.left, prompt.indicator, prompt_right);

    if matches!(input_mode, ReplInputMode::Basic)
        || !io::stdin().is_terminal()
        || !io::stdout().is_terminal()
    {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            eprintln!("Warning: Input is not a terminal (fd=0).");
        }
        run_repl_basic(&prompt, &mut submission)?;
        return Ok(ReplRunResult::Exit(0));
    }

    let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&completion_words));
    let completer = Box::new(ReplCompleter::new(
        completion_words,
        Some(tree.clone()),
        line_projector.clone(),
    ));
    let completion_menu = Box::new(build_completion_menu(&appearance));
    let highlighter = build_repl_highlighter(&tree, &appearance, line_projector);
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
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::BackTab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuPrevious,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char(' '),
        ReedlineEvent::Menu("completion_menu".to_string()),
    );
    let edit_mode = Box::new(AutoCompleteEmacs::new(
        Emacs::new(keybindings),
        "completion_menu",
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
        let signal = match editor.read_line(&prompt) {
            Ok(signal) => signal,
            Err(err) => {
                if is_cursor_position_error(&err) {
                    eprintln!(
                        "WARNING: terminal does not support cursor position requests; \
falling back to basic input mode."
                    );
                    run_repl_basic(&prompt, &mut submission)?;
                    return Ok(ReplRunResult::Exit(0));
                }
                return Err(err.into());
            }
        };

        match signal {
            Signal::Success(line) => match process_submission(&line, &mut submission)? {
                SubmissionResult::Noop => continue,
                SubmissionResult::Print(output) => print!("{output}"),
                SubmissionResult::ReplaceInput(buffer) => {
                    editor.run_edit_commands(&[
                        EditCommand::Clear,
                        EditCommand::InsertString(buffer),
                    ]);
                    continue;
                }
                SubmissionResult::Exit(code) => return Ok(ReplRunResult::Exit(code)),
                SubmissionResult::Restart { output, reload } => {
                    return Ok(ReplRunResult::Restart { output, reload });
                }
            },
            Signal::CtrlD => return Ok(ReplRunResult::Exit(0)),
            Signal::CtrlC => continue,
        }
    }
}

fn is_cursor_position_error(err: &io::Error) -> bool {
    if matches!(err.raw_os_error(), Some(6 | 25)) {
        return true;
    }
    let message = err.to_string().to_ascii_lowercase();
    message.contains("cursor position could not be read")
        || message.contains("no such device or address")
        || message.contains("inappropriate ioctl")
}

fn run_repl_basic<F>(prompt: &OspPrompt, submission: &mut SubmissionContext<'_, F>) -> Result<()>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    let stdin = io::stdin();
    loop {
        print!("{}{}", prompt.left, prompt.indicator);
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

struct ReplCompleter {
    engine: CompletionEngine,
    line_projector: Option<LineProjector>,
}

impl ReplCompleter {
    fn new(
        mut words: Vec<String>,
        completion_tree: Option<CompletionTree>,
        line_projector: Option<LineProjector>,
    ) -> Self {
        words.sort();
        words.dedup();
        let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&words));
        Self {
            engine: CompletionEngine::new(tree),
            line_projector,
        }
    }
}

impl Completer for ReplCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        debug_assert!(
            pos <= line.len(),
            "completer received pos {pos} beyond line length {}",
            line.len()
        );
        let projected = self
            .line_projector
            .as_ref()
            .map(|project| project(line))
            .unwrap_or_else(|| LineProjection::passthrough(line));
        let (cursor_state, outputs) = self.engine.complete(&projected.line, pos);
        let span = Span {
            start: cursor_state.replace_range.start,
            end: cursor_state.replace_range.end,
        };

        let mut ranked = Vec::new();
        let mut has_path_sentinel = false;
        for output in outputs {
            match output {
                SuggestionOutput::Item(item) => ranked.push(item),
                SuggestionOutput::PathSentinel => has_path_sentinel = true,
            }
        }

        let mut suggestions = ranked
            .into_iter()
            .filter(|item| !projected.hidden_suggestions.contains(&item.text))
            .map(|item| Suggestion {
                value: item.text,
                description: item.meta,
                extra: item.display.map(|display| vec![display]),
                span,
                append_whitespace: true,
                ..Suggestion::default()
            })
            .collect::<Vec<_>>();

        if has_path_sentinel {
            suggestions.extend(path_suggestions(&cursor_state.raw_stub, span));
        }

        suggestions
    }
}

pub fn default_pipe_verbs() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("F".to_string(), "Filter rows".to_string()),
        ("P".to_string(), "Project columns".to_string()),
        ("S".to_string(), "Sort rows".to_string()),
        ("G".to_string(), "Group rows".to_string()),
        ("A".to_string(), "Aggregate rows/groups".to_string()),
        ("L".to_string(), "Limit rows".to_string()),
        ("Z".to_string(), "Collapse grouped output".to_string()),
        ("C".to_string(), "Count rows".to_string()),
        ("Y".to_string(), "Mark output for copy".to_string()),
        ("H".to_string(), "Show DSL help".to_string()),
        ("V".to_string(), "Value-only quick search".to_string()),
        ("K".to_string(), "Key-only quick search".to_string()),
        ("?".to_string(), "Clean rows / exists filter".to_string()),
        ("U".to_string(), "Unroll list field".to_string()),
        ("JQ".to_string(), "Run jq expression".to_string()),
        ("VAL".to_string(), "Extract values".to_string()),
        ("VALUE".to_string(), "Extract values".to_string()),
    ])
}

fn build_repl_tree(words: &[String]) -> CompletionTree {
    let suggestions = words
        .iter()
        .map(|word| SuggestionEntry::value(word.clone()))
        .collect::<Vec<_>>();
    let args = (0..12)
        .map(|_| ArgNode {
            suggestions: suggestions.clone(),
            ..ArgNode::default()
        })
        .collect::<Vec<_>>();

    CompletionTree {
        root: CompletionNode {
            args,
            ..CompletionNode::default()
        },
        pipe_verbs: default_pipe_verbs(),
    }
}

fn build_completion_menu(appearance: &ReplAppearance) -> OspCompletionMenu {
    let text_color = appearance
        .completion_text_style
        .as_deref()
        .and_then(color_from_style_spec);
    let background_color = appearance
        .completion_background_style
        .as_deref()
        .and_then(color_from_style_spec);
    let highlight_color = appearance
        .completion_highlight_style
        .as_deref()
        .and_then(color_from_style_spec);

    OspCompletionMenu::default()
        .with_name("completion_menu")
        .with_only_buffer_difference(false)
        .with_marker("")
        .with_columns(u16::MAX)
        .with_max_rows(u16::MAX)
        .with_description_rows(1)
        .with_column_padding(2)
        .with_text_style(style_with_fg_bg(text_color, background_color))
        .with_description_text_style(style_with_fg_bg(text_color, highlight_color))
        .with_match_text_style(style_with_fg_bg(highlight_color, background_color))
        .with_selected_text_style(style_with_fg_bg(highlight_color, text_color))
        .with_selected_match_text_style(style_with_fg_bg(highlight_color, text_color))
}

fn build_repl_highlighter(
    tree: &CompletionTree,
    appearance: &ReplAppearance,
    line_projector: Option<LineProjector>,
) -> Option<ReplHighlighter> {
    let command_color = appearance
        .command_highlight_style
        .as_deref()
        .and_then(color_from_style_spec);
    Some(ReplHighlighter::new(
        tree.clone(),
        command_color?,
        line_projector,
    ))
}

fn style_with_fg_bg(fg: Option<Color>, bg: Option<Color>) -> Style {
    let mut style = Style::new();
    if let Some(fg) = fg {
        style = style.fg(fg);
    }
    if let Some(bg) = bg {
        style = style.on(bg);
    }
    style
}

pub fn color_from_style_spec(spec: &str) -> Option<Color> {
    let token = extract_color_token(spec)?;
    parse_color_token(token)
}

fn extract_color_token(spec: &str) -> Option<&str> {
    let attrs = [
        "bold",
        "dim",
        "dimmed",
        "italic",
        "underline",
        "blink",
        "reverse",
        "hidden",
        "strikethrough",
    ];

    let mut last: Option<&str> = None;
    for part in spec.split_whitespace() {
        let token = part
            .trim()
            .strip_prefix("fg:")
            .or_else(|| part.trim().strip_prefix("bg:"))
            .unwrap_or(part.trim());
        if token.is_empty() {
            continue;
        }
        if attrs.iter().any(|attr| token.eq_ignore_ascii_case(attr)) {
            continue;
        }
        last = Some(token);
    }
    last
}

fn parse_color_token(token: &str) -> Option<Color> {
    let normalized = token.trim().to_ascii_lowercase();

    if let Some(value) = normalized.strip_prefix('#') {
        if value.len() == 6 {
            let r = u8::from_str_radix(&value[0..2], 16).ok()?;
            let g = u8::from_str_radix(&value[2..4], 16).ok()?;
            let b = u8::from_str_radix(&value[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        if value.len() == 3 {
            let r = u8::from_str_radix(&value[0..1], 16).ok()?;
            let g = u8::from_str_radix(&value[1..2], 16).ok()?;
            let b = u8::from_str_radix(&value[2..3], 16).ok()?;
            return Some(Color::Rgb(
                r.saturating_mul(17),
                g.saturating_mul(17),
                b.saturating_mul(17),
            ));
        }
    }

    if let Some(value) = normalized.strip_prefix("ansi")
        && let Ok(index) = value.parse::<u8>()
    {
        return Some(Color::Fixed(index));
    }

    if let Some(value) = normalized
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut parts = value.split(',').map(|part| part.trim().parse::<u8>().ok());
        if let (Some(Some(r)), Some(Some(g)), Some(Some(b))) =
            (parts.next(), parts.next(), parts.next())
        {
            return Some(Color::Rgb(r, g, b));
        }
    }

    match normalized.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" | "purple" => Some(Color::Purple),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "darkgray" | "dark_gray" | "gray" | "grey" => Some(Color::DarkGray),
        "lightgray" | "light_gray" | "lightgrey" | "light_grey" => Some(Color::LightGray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" | "lightpurple" | "light_purple" => {
            Some(Color::LightPurple)
        }
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompletionTraceMenuState {
    pub selected_index: i64,
    pub selected_row: u16,
    pub selected_col: u16,
    pub active: bool,
    pub just_activated: bool,
    pub columns: u16,
    pub visible_rows: u16,
    pub rows: u16,
    pub menu_indent: u16,
}

#[derive(Debug, Clone, Serialize)]
struct CompletionTracePayload<'a> {
    event: &'a str,
    line: &'a str,
    cursor: usize,
    stub: &'a str,
    matches: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_before: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_after: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_before: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_after: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accepted_value: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replace_range: Option<[usize; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_row: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_col: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    just_activated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    columns: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visible_rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    menu_indent: Option<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompletionTraceEvent<'a> {
    pub event: &'a str,
    pub line: &'a str,
    pub cursor: usize,
    pub stub: &'a str,
    pub matches: Vec<String>,
    pub replace_range: Option<[usize; 2]>,
    pub menu: Option<CompletionTraceMenuState>,
    pub buffer_before: Option<&'a str>,
    pub buffer_after: Option<&'a str>,
    pub cursor_before: Option<usize>,
    pub cursor_after: Option<usize>,
    pub accepted_value: Option<&'a str>,
}

pub(crate) fn trace_completion(trace: CompletionTraceEvent<'_>) {
    if !trace_completion_enabled() {
        return;
    }

    let (
        selected_index,
        selected_row,
        selected_col,
        active,
        just_activated,
        columns,
        visible_rows,
        rows,
        menu_indent,
    ) = if let Some(menu) = trace.menu {
        (
            Some(menu.selected_index),
            Some(menu.selected_row),
            Some(menu.selected_col),
            Some(menu.active),
            Some(menu.just_activated),
            Some(menu.columns),
            Some(menu.visible_rows),
            Some(menu.rows),
            Some(menu.menu_indent),
        )
    } else {
        (None, None, None, None, None, None, None, None, None)
    };

    let payload = CompletionTracePayload {
        event: trace.event,
        line: trace.line,
        cursor: trace.cursor,
        stub: trace.stub,
        matches: trace.matches,
        buffer_before: trace.buffer_before,
        buffer_after: trace.buffer_after,
        cursor_before: trace.cursor_before,
        cursor_after: trace.cursor_after,
        accepted_value: trace.accepted_value,
        replace_range: trace.replace_range,
        selected_index,
        selected_row,
        selected_col,
        active,
        just_activated,
        columns,
        visible_rows,
        rows,
        menu_indent,
    };

    let serialized = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    if let Ok(path) = std::env::var("OSP_REPL_TRACE_PATH")
        && !path.trim().is_empty()
    {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{serialized}");
        }
    } else {
        eprintln!("{serialized}");
    }
}

pub(crate) fn trace_completion_enabled() -> bool {
    let Ok(raw) = std::env::var("OSP_REPL_TRACE_COMPLETION") else {
        return false;
    };
    !matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "off" | "no"
    )
}

fn path_suggestions(stub: &str, span: Span) -> Vec<Suggestion> {
    let (lookup, insert_prefix, typed_prefix) = split_path_stub(stub);
    let read_dir = std::fs::read_dir(&lookup);
    let Ok(entries) = read_dir else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(&typed_prefix) {
            continue;
        }

        let path = entry.path();
        let is_dir = path.is_dir();
        let suffix = if is_dir { "/" } else { "" };

        out.push(Suggestion {
            value: format!("{insert_prefix}{file_name}{suffix}"),
            description: Some(if is_dir { "dir" } else { "file" }.to_string()),
            span,
            append_whitespace: !is_dir,
            ..Suggestion::default()
        });
    }

    out
}

fn split_path_stub(stub: &str) -> (PathBuf, String, String) {
    if stub.is_empty() {
        return (PathBuf::from("."), String::new(), String::new());
    }

    let expanded = expand_home(stub);
    let mut lookup = PathBuf::from(&expanded);
    if stub.ends_with('/') {
        return (lookup, stub.to_string(), String::new());
    }

    let typed_prefix = Path::new(stub)
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_default();

    let insert_prefix = match stub.rfind('/') {
        Some(index) => stub[..=index].to_string(),
        None => String::new(),
    };

    if let Some(parent) = lookup.parent() {
        if parent.as_os_str().is_empty() {
            lookup = PathBuf::from(".");
        } else {
            lookup = parent.to_path_buf();
        }
    } else {
        lookup = PathBuf::from(".");
    }

    (lookup, insert_prefix, typed_prefix)
}

fn expand_home(path: &str) -> String {
    if path == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    path.to_string()
}

struct OspPrompt {
    left: String,
    indicator: String,
    right: Option<PromptRightRenderer>,
}

impl OspPrompt {
    fn new(left: String, indicator: String, right: Option<PromptRightRenderer>) -> Self {
        Self {
            left,
            indicator,
            right,
        }
    }
}

impl Prompt for OspPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.left.as_str())
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        match &self.right {
            Some(render) => Cow::Owned(render()),
            None => Cow::Borrowed(""),
        }
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(self.indicator.as_str())
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }

    fn get_prompt_color(&self) -> reedline::Color {
        reedline::Color::Reset
    }

    fn get_indicator_color(&self) -> reedline::Color {
        reedline::Color::Reset
    }
}

#[cfg(test)]
mod tests {
    use nu_ansi_term::Color;
    use osp_completion::{CompletionNode, CompletionTree, FlagNode};
    use reedline::{
        Completer, EditCommand, Prompt, PromptEditMode, PromptHistorySearch,
        PromptHistorySearchStatus,
    };
    use std::collections::BTreeSet;
    use std::io;
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::{
        AutoCompleteEmacs, CompletionDebugOptions, DebugStep, HistoryConfig, HistoryShellContext,
        OspPrompt, PromptRightRenderer, ReplAppearance, ReplCompleter, ReplLineResult, ReplPrompt,
        ReplReloadKind, ReplRunResult, SharedHistory, SubmissionContext, SubmissionResult,
        build_repl_highlighter, color_from_style_spec, debug_completion, debug_completion_steps,
        default_pipe_verbs, expand_history, expand_home, is_cursor_position_error,
        path_suggestions, process_submission, split_path_stub, trace_completion,
        trace_completion_enabled,
    };
    use crate::LineProjection;

    fn completion_tree_with_config_show() -> CompletionTree {
        let mut config = CompletionNode::default();
        config
            .children
            .insert("show".to_string(), CompletionNode::default());

        let mut root = CompletionNode::default();
        root.children.insert("config".to_string(), config);
        CompletionTree {
            root,
            ..CompletionTree::default()
        }
    }

    fn disabled_history() -> SharedHistory {
        SharedHistory::new(
            HistoryConfig {
                path: None,
                max_entries: 0,
                enabled: false,
                dedupe: false,
                profile_scoped: false,
                exclude_patterns: Vec::new(),
                profile: None,
                terminal: None,
                shell_context: HistoryShellContext::default(),
            }
            .normalized(),
        )
        .expect("history config should build")
    }

    #[test]
    fn expands_double_bang() {
        let history = vec!["ldap user oistes".to_string()];
        assert_eq!(
            expand_history("!!", &history, None, false),
            Some("ldap user oistes".to_string())
        );
    }

    #[test]
    fn expands_relative() {
        let history = vec![
            "ldap user oistes".to_string(),
            "ldap netgroup ucore".to_string(),
        ];
        assert_eq!(
            expand_history("!-1", &history, None, false),
            Some("ldap netgroup ucore".to_string())
        );
    }

    #[test]
    fn expands_prefix() {
        let history = vec![
            "ldap user oistes".to_string(),
            "ldap netgroup ucore".to_string(),
        ];
        assert_eq!(
            expand_history("!ldap user", &history, None, false),
            Some("ldap user oistes".to_string())
        );
    }

    #[test]
    fn submission_delegates_help_and_exit_to_host() {
        let history = disabled_history();
        let mut seen = Vec::new();
        let mut execute = |line: &str, _: &SharedHistory| {
            seen.push(line.to_string());
            Ok(match line {
                "help" => ReplLineResult::Continue("host help".to_string()),
                "!!" => ReplLineResult::ReplaceInput("ldap user oistes".to_string()),
                "exit" => ReplLineResult::Exit(7),
                other => ReplLineResult::Continue(other.to_string()),
            })
        };
        let mut submission = SubmissionContext {
            history_store: &history,
            execute: &mut execute,
        };

        let help = process_submission("help", &mut submission).expect("help should succeed");
        let bang = process_submission("!!", &mut submission).expect("bang should succeed");
        let exit = process_submission("exit", &mut submission).expect("exit should succeed");

        assert!(matches!(help, SubmissionResult::Print(text) if text == "host help"));
        assert!(matches!(bang, SubmissionResult::ReplaceInput(text) if text == "ldap user oistes"));
        assert!(matches!(exit, SubmissionResult::Exit(7)));
        assert_eq!(
            seen,
            vec!["help".to_string(), "!!".to_string(), "exit".to_string()]
        );
    }

    #[test]
    fn completer_suggests_word_prefixes() {
        let mut completer = ReplCompleter::new(
            vec![
                "ldap".to_string(),
                "plugins".to_string(),
                "theme".to_string(),
            ],
            None,
            None,
        );

        let completions = completer.complete("ld", 2);
        let values = completions
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["ldap".to_string()]);
    }

    #[test]
    fn completer_supports_fuzzy_word_matching() {
        let mut completer = ReplCompleter::new(
            vec![
                "ldap".to_string(),
                "plugins".to_string(),
                "theme".to_string(),
            ],
            None,
            None,
        );

        let completions = completer.complete("lap", 3);
        let values = completions
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();
        assert!(values.contains(&"ldap".to_string()));
    }

    #[test]
    fn completer_suggests_pipe_verbs_after_pipe() {
        let mut completer = ReplCompleter::new(vec!["ldap".to_string()], None, None);
        let completions = completer.complete("ldap user | F", "ldap user | F".len());
        let values = completions
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();
        assert!(values.contains(&"F".to_string()));
    }

    #[test]
    fn default_pipe_verbs_include_extended_dsl_surface() {
        let verbs = default_pipe_verbs();

        assert_eq!(
            verbs.get("?"),
            Some(&"Clean rows / exists filter".to_string())
        );
        assert_eq!(verbs.get("JQ"), Some(&"Run jq expression".to_string()));
        assert_eq!(verbs.get("VALUE"), Some(&"Extract values".to_string()));
    }

    #[test]
    fn completer_with_tree_does_not_fallback_to_word_list() {
        let mut root = CompletionNode::default();
        root.children
            .insert("config".to_string(), CompletionNode::default());
        let tree = CompletionTree {
            root,
            ..CompletionTree::default()
        };

        let mut completer = ReplCompleter::new(vec!["ldap".to_string()], Some(tree), None);
        let completions = completer.complete("zzz", 3);
        assert!(completions.is_empty());
    }

    #[test]
    fn completer_can_use_projected_line_for_host_flags_unit() {
        let tree = completion_tree_with_config_show();
        let projector = Arc::new(|line: &str| {
            LineProjection::passthrough(line.replacen("--json", "      ", 1))
        });
        let mut completer = ReplCompleter::new(Vec::new(), Some(tree), Some(projector));

        let completions = completer.complete("--json config sh", "--json config sh".len());
        let values = completions
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();

        assert!(values.contains(&"show".to_string()));
    }

    #[test]
    fn completer_hides_suggestions_requested_by_projection_unit() {
        let mut root = CompletionNode::default();
        root.flags
            .insert("--json".to_string(), FlagNode::new().flag_only());
        root.flags
            .insert("--debug".to_string(), FlagNode::new().flag_only());
        let tree = CompletionTree {
            root,
            ..CompletionTree::default()
        };
        let projector = Arc::new(|line: &str| {
            let mut hidden = BTreeSet::new();
            hidden.insert("--json".to_string());
            LineProjection {
                line: line.to_string(),
                hidden_suggestions: hidden,
            }
        });
        let mut completer = ReplCompleter::new(Vec::new(), Some(tree), Some(projector));

        let values = completer
            .complete("-", 1)
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();

        assert!(!values.contains(&"--json".to_string()));
        assert!(values.contains(&"--debug".to_string()));
    }

    #[test]
    fn completer_uses_engine_metadata_for_subcommands() {
        let mut ldap = CompletionNode {
            tooltip: Some("Directory lookup".to_string()),
            ..CompletionNode::default()
        };
        ldap.children
            .insert("user".to_string(), CompletionNode::default());
        ldap.children
            .insert("host".to_string(), CompletionNode::default());

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("ldap", ldap),
            ..CompletionTree::default()
        };

        let mut completer = ReplCompleter::new(Vec::new(), Some(tree), None);
        let completion = completer
            .complete("ld", 2)
            .into_iter()
            .find(|item| item.value == "ldap")
            .expect("ldap completion should exist");

        assert!(completion.description.as_deref().is_some_and(|value| {
            value.contains("Directory lookup")
                && value.contains("subcommands:")
                && value.contains("host")
                && value.contains("user")
        }));
    }

    #[test]
    fn color_parser_extracts_hex_and_named_colors() {
        assert_eq!(
            color_from_style_spec("bold #ff79c6"),
            Some(Color::Rgb(255, 121, 198))
        );
        assert_eq!(
            color_from_style_spec("fg:cyan underline"),
            Some(Color::Cyan)
        );
        assert_eq!(color_from_style_spec("bg:ansi141"), Some(Color::Fixed(141)));
        assert_eq!(
            color_from_style_spec("fg:rgb(80,250,123)"),
            Some(Color::Rgb(80, 250, 123))
        );
        assert!(color_from_style_spec("not-a-color").is_none());
    }

    #[test]
    fn split_path_stub_without_slash_uses_current_directory_lookup() {
        let (lookup, insert_prefix, typed_prefix) = super::split_path_stub("do");

        assert_eq!(lookup, PathBuf::from("."));
        assert_eq!(insert_prefix, "");
        assert_eq!(typed_prefix, "do");
    }

    #[test]
    fn debug_step_parse_round_trips_known_values_unit() {
        assert_eq!(DebugStep::Tab.as_str(), "tab");
        assert_eq!(DebugStep::Up.as_str(), "up");
        assert_eq!(DebugStep::Down.as_str(), "down");
        assert_eq!(DebugStep::Left.as_str(), "left");
        assert_eq!(DebugStep::parse("shift-tab"), Some(DebugStep::BackTab));
        assert_eq!(DebugStep::parse("ENTER"), Some(DebugStep::Accept));
        assert_eq!(DebugStep::parse("esc"), Some(DebugStep::Close));
        assert_eq!(DebugStep::Right.as_str(), "right");
        assert_eq!(DebugStep::parse("wat"), None);
    }

    #[test]
    fn debug_completion_and_steps_surface_menu_state_unit() {
        let tree = completion_tree_with_config_show();
        let debug = debug_completion(
            &tree,
            "config sh",
            "config sh".len(),
            CompletionDebugOptions::new(80, 6),
        );
        assert_eq!(debug.stub, "sh");
        assert!(debug.matches.iter().any(|item| item.id == "show"));

        let frames = debug_completion_steps(
            &tree,
            "config sh",
            "config sh".len(),
            CompletionDebugOptions::new(80, 6),
            &[DebugStep::Tab, DebugStep::Accept],
        );
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].step, "tab");
        assert!(frames[0].state.matches.iter().any(|item| item.id == "show"));
        assert_eq!(frames[1].step, "accept");
        assert_eq!(frames[1].state.line, "config show ");
    }

    #[test]
    fn debug_completion_options_and_empty_steps_cover_builder_paths_unit() {
        let appearance = ReplAppearance {
            completion_text_style: Some("white".to_string()),
            completion_background_style: Some("black".to_string()),
            completion_highlight_style: Some("cyan".to_string()),
            command_highlight_style: Some("green".to_string()),
        };
        let options = CompletionDebugOptions::new(90, 12)
            .ansi(true)
            .unicode(true)
            .appearance(Some(&appearance));

        assert!(options.ansi);
        assert!(options.unicode);
        assert!(options.appearance.is_some());

        let tree = completion_tree_with_config_show();
        let frames = debug_completion_steps(&tree, "config sh", 9, options, &[]);
        assert!(frames.is_empty());
    }

    #[test]
    fn debug_completion_navigation_variants_and_empty_matches_are_stable_unit() {
        let tree = completion_tree_with_config_show();
        let frames = debug_completion_steps(
            &tree,
            "config sh",
            "config sh".len(),
            CompletionDebugOptions::new(80, 6),
            &[
                DebugStep::Tab,
                DebugStep::Down,
                DebugStep::Right,
                DebugStep::Left,
                DebugStep::Up,
                DebugStep::BackTab,
                DebugStep::Close,
            ],
        );
        assert_eq!(frames.len(), 7);
        assert_eq!(
            frames.last().map(|frame| frame.step.as_str()),
            Some("close")
        );

        let debug = debug_completion(&tree, "zzz", 3, CompletionDebugOptions::new(80, 6));
        assert!(debug.matches.is_empty());
        assert_eq!(debug.selected, -1);
    }

    #[test]
    fn autocomplete_policy_and_path_helpers_cover_non_happy_paths_unit() {
        assert!(AutoCompleteEmacs::should_reopen_menu(&[
            EditCommand::InsertChar('x')
        ]));
        assert!(!AutoCompleteEmacs::should_reopen_menu(&[
            EditCommand::MoveToStart { select: false }
        ]));

        let missing = path_suggestions(
            "/definitely/not/a/real/dir/",
            reedline::Span { start: 0, end: 0 },
        );
        assert!(missing.is_empty());

        let (lookup, insert_prefix, typed_prefix) = split_path_stub("/tmp/demo/");
        assert_eq!(lookup, PathBuf::from("/tmp/demo/"));
        assert_eq!(insert_prefix, "/tmp/demo/");
        assert!(typed_prefix.is_empty());
    }

    #[test]
    fn completion_debug_options_builders_and_empty_steps_unit() {
        let appearance = super::ReplAppearance {
            completion_text_style: Some("cyan".to_string()),
            ..Default::default()
        };
        let options = CompletionDebugOptions::new(120, 40)
            .ansi(true)
            .unicode(true)
            .appearance(Some(&appearance));

        assert_eq!(options.width, 120);
        assert_eq!(options.height, 40);
        assert!(options.ansi);
        assert!(options.unicode);
        assert!(options.appearance.is_some());

        let tree = completion_tree_with_config_show();
        let frames = debug_completion_steps(&tree, "config sh", 9, options, &[]);
        assert!(frames.is_empty());
    }

    #[test]
    fn debug_completion_navigation_steps_cover_menu_branches_unit() {
        let tree = completion_tree_with_config_show();
        let frames = debug_completion_steps(
            &tree,
            "config sh",
            9,
            CompletionDebugOptions::new(80, 6),
            &[
                DebugStep::Tab,
                DebugStep::Down,
                DebugStep::Right,
                DebugStep::Left,
                DebugStep::Up,
                DebugStep::BackTab,
                DebugStep::Close,
            ],
        );

        assert_eq!(frames.len(), 7);
        assert_eq!(frames[0].step, "tab");
        assert_eq!(frames[1].step, "down");
        assert_eq!(frames[2].step, "right");
        assert_eq!(frames[3].step, "left");
        assert_eq!(frames[4].step, "up");
        assert_eq!(frames[5].step, "backtab");
        assert_eq!(frames[6].step, "close");
    }

    #[test]
    fn debug_completion_without_matches_reports_unmatched_cursor_state_unit() {
        let tree = completion_tree_with_config_show();
        let debug = debug_completion(&tree, "zzz", 99, CompletionDebugOptions::new(80, 6));

        assert_eq!(debug.line, "zzz");
        assert_eq!(debug.cursor, 3);
        assert!(debug.matches.is_empty());
        assert_eq!(debug.selected, -1);
        assert_eq!(debug.stub, "zzz");
        assert_eq!(debug.replace_range, [0, 3]);
    }

    #[test]
    fn autocomplete_emacs_reopens_for_edits_but_not_movement_unit() {
        assert!(super::AutoCompleteEmacs::should_reopen_menu(&[
            EditCommand::InsertChar('x')
        ]));
        assert!(super::AutoCompleteEmacs::should_reopen_menu(&[
            EditCommand::BackspaceWord
        ]));
        assert!(!super::AutoCompleteEmacs::should_reopen_menu(&[
            EditCommand::MoveLeft { select: false }
        ]));
        assert!(!super::AutoCompleteEmacs::should_reopen_menu(&[
            EditCommand::MoveToLineEnd { select: false }
        ]));
    }

    #[test]
    fn process_submission_handles_restart_and_error_paths_unit() {
        let history = disabled_history();

        let mut restart_execute = |_line: &str, _: &SharedHistory| {
            Ok(ReplLineResult::Restart {
                output: "restarting".to_string(),
                reload: ReplReloadKind::WithIntro,
            })
        };
        let mut submission = SubmissionContext {
            history_store: &history,
            execute: &mut restart_execute,
        };
        let restart =
            process_submission("config set", &mut submission).expect("restart should map");
        assert!(matches!(
            restart,
            SubmissionResult::Restart {
                output,
                reload: ReplReloadKind::WithIntro
            } if output == "restarting"
        ));

        let mut failing_execute =
            |_line: &str, _: &SharedHistory| -> anyhow::Result<ReplLineResult> {
                Err(anyhow::anyhow!("boom"))
            };
        let mut failing_submission = SubmissionContext {
            history_store: &history,
            execute: &mut failing_execute,
        };
        let result = process_submission("broken", &mut failing_submission)
            .expect("error should be absorbed");
        assert!(matches!(result, SubmissionResult::Noop));

        let mut noop_execute =
            |_line: &str, _: &SharedHistory| Ok(ReplLineResult::Continue("ignored".to_string()));
        let mut noop_submission = SubmissionContext {
            history_store: &history,
            execute: &mut noop_execute,
        };
        let result =
            process_submission("   ", &mut noop_submission).expect("blank lines should noop");
        assert!(matches!(result, SubmissionResult::Noop));
    }

    #[test]
    fn highlighter_builder_requires_command_color_unit() {
        let tree = completion_tree_with_config_show();
        let none = build_repl_highlighter(&tree, &super::ReplAppearance::default(), None);
        assert!(none.is_none());

        let some = build_repl_highlighter(
            &tree,
            &super::ReplAppearance {
                command_highlight_style: Some("green".to_string()),
                ..Default::default()
            },
            None,
        );
        assert!(some.is_some());
    }

    #[test]
    fn path_suggestions_distinguish_files_and_directories_unit() {
        let root = make_temp_dir("osp-repl-paths");
        std::fs::write(root.join("alpha.txt"), "x").expect("file should be written");
        std::fs::create_dir_all(root.join("alpine")).expect("dir should be created");
        let stub = format!("{}/al", root.display());

        let suggestions = path_suggestions(
            &stub,
            reedline::Span {
                start: 0,
                end: stub.len(),
            },
        );
        let values = suggestions
            .iter()
            .map(|item| {
                (
                    item.value.clone(),
                    item.description.clone(),
                    item.append_whitespace,
                )
            })
            .collect::<Vec<_>>();

        assert!(values.iter().any(|(value, desc, append)| {
            value.ends_with("alpha.txt") && desc.as_deref() == Some("file") && *append
        }));
        assert!(values.iter().any(|(value, desc, append)| {
            value.ends_with("alpine/") && desc.as_deref() == Some("dir") && !*append
        }));
    }

    #[test]
    fn trace_completion_writes_jsonl_when_enabled_unit() {
        let temp_dir = make_temp_dir("osp-repl-trace");
        let trace_path = temp_dir.join("trace.jsonl");
        let previous_enabled = std::env::var("OSP_REPL_TRACE_COMPLETION").ok();
        let previous_path = std::env::var("OSP_REPL_TRACE_PATH").ok();
        set_env_var_for_test("OSP_REPL_TRACE_COMPLETION", "1");
        set_env_var_for_test("OSP_REPL_TRACE_PATH", &trace_path);

        assert!(trace_completion_enabled());
        trace_completion(super::CompletionTraceEvent {
            event: "complete",
            line: "config sh",
            cursor: 9,
            stub: "sh",
            matches: vec!["show".to_string()],
            replace_range: Some([7, 9]),
            menu: None,
            buffer_before: None,
            buffer_after: None,
            cursor_before: None,
            cursor_after: None,
            accepted_value: None,
        });

        let contents = std::fs::read_to_string(&trace_path).expect("trace file should exist");
        assert!(contents.contains("\"event\":\"complete\""));
        assert!(contents.contains("\"stub\":\"sh\""));

        restore_env("OSP_REPL_TRACE_COMPLETION", previous_enabled);
        restore_env("OSP_REPL_TRACE_PATH", previous_path);
    }

    #[test]
    fn trace_completion_enabled_recognizes_falsey_values_unit() {
        let previous = std::env::var("OSP_REPL_TRACE_COMPLETION").ok();

        set_env_var_for_test("OSP_REPL_TRACE_COMPLETION", "off");
        assert!(!trace_completion_enabled());
        set_env_var_for_test("OSP_REPL_TRACE_COMPLETION", "yes");
        assert!(trace_completion_enabled());

        restore_env("OSP_REPL_TRACE_COMPLETION", previous);
    }

    #[test]
    fn cursor_position_errors_are_recognized_unit() {
        assert!(is_cursor_position_error(&io::Error::from_raw_os_error(25)));
        assert!(is_cursor_position_error(&io::Error::other(
            "Cursor position could not be read"
        )));
        assert!(!is_cursor_position_error(&io::Error::other(
            "permission denied"
        )));
    }

    #[test]
    fn expand_home_and_prompt_renderers_behave_unit() {
        let previous_home = std::env::var("HOME").ok();
        set_env_var_for_test("HOME", "/tmp/osp-home");
        assert_eq!(expand_home("~"), "/tmp/osp-home");
        assert_eq!(expand_home("~/cache"), "/tmp/osp-home/cache");
        assert_eq!(expand_home("/etc/hosts"), "/etc/hosts");

        let right: PromptRightRenderer = Arc::new(|| "rhs".to_string());
        let prompt = OspPrompt::new("left".to_string(), "> ".to_string(), Some(right));
        assert_eq!(prompt.render_prompt_left(), "left");
        assert_eq!(prompt.render_prompt_right(), "rhs");
        assert_eq!(
            prompt.render_prompt_indicator(PromptEditMode::Default),
            "> "
        );
        assert_eq!(prompt.render_prompt_multiline_indicator(), "... ");
        assert_eq!(
            prompt.render_prompt_history_search_indicator(PromptHistorySearch {
                status: PromptHistorySearchStatus::Passing,
                term: "ldap".to_string(),
            }),
            "(reverse-search: ldap) "
        );

        let simple = ReplPrompt::simple("osp");
        assert_eq!(simple.left, "osp");
        assert!(simple.indicator.is_empty());

        let restart = ReplRunResult::Restart {
            output: "x".to_string(),
            reload: ReplReloadKind::Default,
        };
        assert!(matches!(
            restart,
            ReplRunResult::Restart {
                output,
                reload: ReplReloadKind::Default
            } if output == "x"
        ));

        restore_env("HOME", previous_home);
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        dir.push(format!("{prefix}-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    fn restore_env(key: &str, value: Option<String>) {
        if let Some(value) = value {
            set_env_var_for_test(key, value);
        } else {
            remove_env_var_for_test(key);
        }
    }

    fn set_env_var_for_test(key: &str, value: impl AsRef<std::ffi::OsStr>) {
        // Test-only environment mutation is process-global on Rust 2024.
        // Keep the unsafe boundary explicit and local to these regression
        // tests instead of spreading raw calls through the module.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_env_var_for_test(key: &str) {
        // See `set_env_var_for_test`; these tests intentionally restore the
        // process environment after probing env-dependent behavior.
        unsafe {
            std::env::remove_var(key);
        }
    }
}
