use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
mod history;
mod menu;
mod menu_core;
pub use history::{
    HistoryConfig, HistoryEntry, HistoryShellContext, OspHistoryStore, SharedHistory,
    expand_history,
};
use menu::{MenuDebug, MenuStyleDebug, OspCompletionMenu, debug_snapshot, display_text};
use nu_ansi_term::{Color, Style};
use osp_completion::{
    ArgNode, CommandLine, CommandLineParser, CompletionEngine, CompletionNode, CompletionTree,
    SuggestionEntry, SuggestionOutput,
};
use reedline::{
    Completer, EditCommand, EditMode, Editor, Emacs, Highlighter, KeyCode, KeyModifiers, Menu,
    MenuEvent, Prompt, PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline,
    ReedlineEvent, ReedlineMenu, ReedlineRawEvent, Signal, Span, StyledText, Suggestion,
    UndoBehavior, default_emacs_keybindings,
};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct ReplPrompt {
    pub left: String,
    pub indicator: String,
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

    let completer = ReplCompleter::new(Vec::new(), Some(tree.clone()));
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
                menu.replace_in_buffer(editor);
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

    let (stub, replace_range) = if let Some(first) = values.first() {
        let start = first.span.start;
        let end = first.span.end;
        let stub = line.get(start..end).unwrap_or("").to_string();
        (stub, [start, end])
    } else {
        let engine = CompletionEngine::new(tree.clone());
        let (stub, _) = engine.suggestions_with_stub(&line, cursor);
        let start = cursor.saturating_sub(stub.len());
        (stub, [start, cursor])
    };

    let parser = CommandLineParser;
    let tokens = parser.tokenize(&line);
    let cmd = parser.parse(&tokens);
    let (context_node, matched_path) = debug_resolve_context_state(tree, &cmd, &stub);
    let flag_scope = debug_nearest_flag_scope(tree, &matched_path);

    let matches = values
        .iter()
        .map(|item| CompletionDebugMatch {
            id: item.value.clone(),
            label: display_text(item).to_string(),
            description: item.description.clone(),
            kind: debug_match_kind(&cmd, &item.value, context_node, &matched_path, flag_scope),
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

pub fn run_repl<F>(
    prompt: ReplPrompt,
    completion_words: Vec<String>,
    completion_tree: Option<CompletionTree>,
    appearance: ReplAppearance,
    history_config: HistoryConfig,
    mut execute: F,
) -> Result<ReplRunResult>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&completion_words));
    let completer = Box::new(ReplCompleter::new(completion_words, Some(tree.clone())));
    let completion_menu = Box::new(build_completion_menu(&appearance));
    let highlighter = build_repl_highlighter(&tree, &appearance);
    let mut keybindings = default_emacs_keybindings();
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
    let history_store = SharedHistory::new(history_config)?;
    editor = editor.with_history(Box::new(history_store.clone()));
    let mut submission = SubmissionContext {
        history_store: &history_store,
        execute: &mut execute,
    };

    let prompt = OspPrompt::new(prompt.left, prompt.indicator);
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("Warning: Input is not a terminal (fd=0).");
        run_repl_basic(&prompt, &mut submission)?;
        return Ok(ReplRunResult::Exit(0));
    }

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
    fallback_words: Vec<String>,
    use_fallback_words: bool,
}

impl ReplCompleter {
    fn new(mut words: Vec<String>, completion_tree: Option<CompletionTree>) -> Self {
        words.sort();
        words.dedup();
        let use_fallback_words = completion_tree.is_none();
        let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&words));
        Self {
            engine: CompletionEngine::new(tree),
            fallback_words: words,
            use_fallback_words,
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
        let (stub, outputs) = self.engine.suggestions_with_stub(line, pos);
        let span = Span {
            start: pos.saturating_sub(stub.len()),
            end: pos,
        };

        let mut ranked = Vec::new();
        let mut has_path_sentinel = false;
        for output in outputs {
            match output {
                SuggestionOutput::Item(item) => ranked.push(RankedSuggestion {
                    value: item.text,
                    description: item.meta,
                    display: item.display,
                    sort: item.sort,
                    match_score: item.match_score,
                    is_exact: item.is_exact,
                }),
                SuggestionOutput::PathSentinel => has_path_sentinel = true,
            }
        }

        if ranked.is_empty() && self.use_fallback_words {
            ranked.extend(word_suggestions(&self.fallback_words, &stub));
        }

        ranked = dedupe_ranked_suggestions(ranked);
        sort_ranked_suggestions(&mut ranked);
        let mut suggestions = ranked
            .into_iter()
            .map(|item| {
                let description = self
                    .engine
                    .subcommands_for(line, pos, &item.value)
                    .map(|subs| format!("subcommands: {}", subs.join(", ")))
                    .or(item.description);
                Suggestion {
                    value: item.value,
                    description,
                    extra: item.display.map(|display| vec![display]),
                    span,
                    append_whitespace: true,
                    ..Suggestion::default()
                }
            })
            .collect::<Vec<_>>();

        if has_path_sentinel {
            suggestions.extend(path_suggestions(&stub, span));
        }

        suggestions
    }
}

#[derive(Debug, Clone)]
struct RankedSuggestion {
    value: String,
    description: Option<String>,
    display: Option<String>,
    sort: Option<String>,
    match_score: u32,
    is_exact: bool,
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
) -> Option<ReplHighlighter> {
    let command_color = appearance
        .command_highlight_style
        .as_deref()
        .and_then(color_from_style_spec);
    Some(ReplHighlighter::new(tree.clone(), command_color?))
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

struct ReplHighlighter {
    tree: CompletionTree,
    command_color: Color,
    parser: CommandLineParser,
}

impl ReplHighlighter {
    fn new(tree: CompletionTree, command_color: Color) -> Self {
        Self {
            tree,
            command_color,
            parser: CommandLineParser,
        }
    }

    fn matched_command_len(&self, tokens: &[String]) -> usize {
        let mut node = &self.tree.root;
        let mut matched = 0usize;

        for token in tokens {
            if token == "|" {
                break;
            }
            if token.starts_with('-') {
                break;
            }
            let Some(child) = node.children.get(token) else {
                break;
            };
            if child.value_key {
                break;
            }
            matched += 1;
            if child.value_leaf {
                break;
            }
            node = child;
        }

        matched
    }
}

impl Highlighter for ReplHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        if line.is_empty() {
            return styled;
        }

        let tokens = self.parser.tokenize(line);
        if tokens.is_empty() {
            styled.push((Style::new(), line.to_string()));
            return styled;
        }

        let matched_len = self.matched_command_len(&tokens);
        let mut pos = 0usize;

        for (index, token) in tokens.iter().enumerate() {
            if token.is_empty() {
                continue;
            }
            let Some(rel_idx) = line[pos..].find(token) else {
                styled.push((Style::new(), line[pos..].to_string()));
                return styled;
            };
            let idx = pos + rel_idx;
            if idx > pos {
                styled.push((Style::new(), line[pos..idx].to_string()));
            }

            let style = if index < matched_len {
                Style::new().fg(self.command_color)
            } else if let Some(color) = parse_hex_color_token(token) {
                Style::new().fg(color)
            } else {
                Style::new()
            };

            styled.push((style, line[idx..idx + token.len()].to_string()));
            pos = idx + token.len();
        }

        if pos < line.len() {
            styled.push((Style::new(), line[pos..].to_string()));
        }

        styled
    }
}

fn parse_hex_color_token(token: &str) -> Option<Color> {
    let normalized = token.trim();
    let hex = normalized.strip_prefix('#')?;
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
        let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
        let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
        return Some(Color::Rgb(
            r.saturating_mul(17),
            g.saturating_mul(17),
            b.saturating_mul(17),
        ));
    }
    None
}

fn color_from_style_spec(spec: &str) -> Option<Color> {
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

fn dedupe_ranked_suggestions(items: Vec<RankedSuggestion>) -> Vec<RankedSuggestion> {
    let mut out: Vec<RankedSuggestion> = Vec::new();
    for item in items {
        if let Some(existing) = out.iter_mut().find(|entry| entry.value == item.value) {
            if existing.description.is_none() {
                existing.description = item.description;
            }
            if existing.display.is_none() {
                existing.display = item.display;
            }
            if existing.sort.is_none() {
                existing.sort = item.sort;
            }
            existing.is_exact |= item.is_exact;
            existing.match_score = existing.match_score.min(item.match_score);
            continue;
        }
        out.push(item);
    }
    out
}

fn sort_ranked_suggestions(items: &mut [RankedSuggestion]) {
    let all_numeric =
        !items.is_empty() && items.iter().all(|item| numeric_sort_value(item).is_some());

    if all_numeric {
        items.sort_by(|left, right| {
            let left_sort = numeric_sort_value(left).unwrap_or(f64::MAX);
            let right_sort = numeric_sort_value(right).unwrap_or(f64::MAX);
            (not_exact(left), left.match_score)
                .cmp(&(not_exact(right), right.match_score))
                .then_with(|| {
                    left_sort
                        .partial_cmp(&right_sort)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    left.value
                        .to_ascii_lowercase()
                        .cmp(&right.value.to_ascii_lowercase())
                })
        });
        return;
    }

    items.sort_by(|left, right| {
        (
            not_exact(left),
            left.match_score,
            left.value.to_ascii_lowercase(),
        )
            .cmp(&(
                not_exact(right),
                right.match_score,
                right.value.to_ascii_lowercase(),
            ))
    });
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

pub(crate) fn trace_completion(
    event: &str,
    line: &str,
    cursor: usize,
    stub: &str,
    matches: Vec<String>,
    replace_range: Option<[usize; 2]>,
    menu: Option<CompletionTraceMenuState>,
    buffer_before: Option<&str>,
    buffer_after: Option<&str>,
    cursor_before: Option<usize>,
    cursor_after: Option<usize>,
    accepted_value: Option<&str>,
) {
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
    ) = if let Some(menu) = menu {
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
        event,
        line,
        cursor,
        stub,
        matches,
        buffer_before,
        buffer_after,
        cursor_before,
        cursor_after,
        accepted_value,
        replace_range,
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

fn debug_resolve_context_state<'a>(
    tree: &'a CompletionTree,
    cmd: &CommandLine,
    stub: &str,
) -> (&'a CompletionNode, Vec<String>) {
    let (pre_node, _) = debug_resolve_context(tree, &cmd.head);
    let has_subcommands = !pre_node.children.is_empty();

    let head_for_context = if !stub.is_empty() && !stub.starts_with('-') && has_subcommands {
        &cmd.head[..cmd.head.len().saturating_sub(1)]
    } else {
        cmd.head.as_slice()
    };

    debug_resolve_context(tree, head_for_context)
}

fn debug_resolve_context<'a>(
    tree: &'a CompletionTree,
    path: &[String],
) -> (&'a CompletionNode, Vec<String>) {
    let mut node = &tree.root;
    let mut matched = Vec::new();

    for segment in path {
        let Some(next) = node.children.get(segment) else {
            break;
        };
        node = next;
        matched.push(segment.clone());
        if node.value_leaf {
            break;
        }
    }

    (node, matched)
}

fn debug_nearest_flag_scope<'a>(tree: &'a CompletionTree, path: &[String]) -> &'a CompletionNode {
    for i in (0..=path.len()).rev() {
        let prefix = &path[..i];
        let (node, matched) = debug_resolve_context(tree, prefix);
        if matched.len() == prefix.len() && !node.flags.is_empty() {
            return node;
        }
    }
    &tree.root
}

fn debug_match_kind(
    cmd: &CommandLine,
    value: &str,
    context_node: &CompletionNode,
    matched_path: &[String],
    flag_scope: &CompletionNode,
) -> String {
    if cmd.has_pipe {
        return "pipe".to_string();
    }
    if value.starts_with("--") || flag_scope.flags.contains_key(value) {
        return "flag".to_string();
    }
    if context_node.children.contains_key(value) {
        return if matched_path.is_empty() {
            "command".to_string()
        } else {
            "subcommand".to_string()
        };
    }
    "value".to_string()
}

fn not_exact(item: &RankedSuggestion) -> bool {
    !item.is_exact
}

fn numeric_sort_value(item: &RankedSuggestion) -> Option<f64> {
    item.sort
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| item.value.parse::<f64>().ok())
        .or_else(|| {
            item.description
                .as_deref()
                .and_then(|value| value.parse::<f64>().ok())
        })
}

fn word_suggestions(words: &[String], stub: &str) -> Vec<RankedSuggestion> {
    words
        .iter()
        .filter_map(|word| {
            let score = fuzzy_match_score(stub, word)?;
            Some(RankedSuggestion {
                value: word.clone(),
                description: None,
                display: None,
                sort: None,
                match_score: score,
                is_exact: score == 0,
            })
        })
        .collect()
}

fn fuzzy_match_score(stub: &str, candidate: &str) -> Option<u32> {
    if stub.is_empty() {
        return Some(1_000);
    }

    let stub_lc = stub.to_ascii_lowercase();
    let candidate_lc = candidate.to_ascii_lowercase();
    if stub_lc == candidate_lc {
        return Some(0);
    }
    if candidate_lc.starts_with(&stub_lc) {
        return Some(100 + (candidate_lc.len().saturating_sub(stub_lc.len())) as u32);
    }

    if let Some(boundary) = boundary_prefix_index(&candidate_lc, &stub_lc) {
        return Some(200 + boundary as u32);
    }

    let fuzzy = fuzzy_matcher().fuzzy_match(&candidate_lc, &stub_lc)?;
    let normalized = fuzzy.max(0) as u32;
    let penalty = 100_000u32.saturating_sub(normalized);
    Some(10_000 + penalty)
}

fn fuzzy_matcher() -> &'static SkimMatcherV2 {
    static MATCHER: OnceLock<SkimMatcherV2> = OnceLock::new();
    MATCHER.get_or_init(SkimMatcherV2::default)
}

fn boundary_prefix_index(candidate: &str, stub: &str) -> Option<usize> {
    candidate
        .match_indices(stub)
        .find(|(idx, _)| {
            *idx == 0
                || candidate
                    .as_bytes()
                    .get(idx.saturating_sub(1))
                    .is_some_and(|byte| matches!(byte, b'-' | b'_' | b'.' | b':' | b'/'))
        })
        .map(|(idx, _)| idx)
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
}

impl OspPrompt {
    fn new(left: String, indicator: String) -> Self {
        Self { left, indicator }
    }
}

impl Prompt for OspPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.left.as_str())
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
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
    use osp_completion::{CompletionNode, CompletionTree};
    use reedline::{Completer, Highlighter, StyledText};
    use std::path::PathBuf;

    use super::{
        HistoryConfig, HistoryShellContext, RankedSuggestion, ReplCompleter, ReplHighlighter,
        ReplLineResult, SharedHistory, SubmissionContext, SubmissionResult, color_from_style_spec,
        dedupe_ranked_suggestions, default_pipe_verbs, expand_history, process_submission,
        sort_ranked_suggestions,
    };

    fn token_styles(styled: &StyledText) -> Vec<(String, Option<Color>)> {
        styled
            .buffer
            .iter()
            .filter_map(|(style, text)| {
                if text.chars().all(|ch| ch.is_whitespace()) {
                    None
                } else {
                    Some((text.clone(), style.foreground))
                }
            })
            .collect()
    }

    fn styled_text_to_plain(styled: &StyledText) -> String {
        styled
            .buffer
            .iter()
            .map(|(_, text)| text.as_str())
            .collect()
    }

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
        SharedHistory::new(HistoryConfig::new(
            None,
            0,
            false,
            false,
            false,
            Vec::new(),
            None,
            None,
            HistoryShellContext::default(),
        ))
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
        let mut completer = ReplCompleter::new(vec!["ldap".to_string()], None);
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

        let mut completer = ReplCompleter::new(vec!["ldap".to_string()], Some(tree));
        let completions = completer.complete("zzz", 3);
        assert!(completions.is_empty());
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
    fn dedupe_ranked_suggestions_preserves_meta_and_display() {
        let input = vec![
            RankedSuggestion {
                value: "x".to_string(),
                description: Some("meta".to_string()),
                display: None,
                sort: None,
                match_score: 120,
                is_exact: false,
            },
            RankedSuggestion {
                value: "x".to_string(),
                description: None,
                display: Some("Display".to_string()),
                sort: Some("10".to_string()),
                match_score: 80,
                is_exact: false,
            },
        ];

        let deduped = dedupe_ranked_suggestions(input);
        assert_eq!(deduped.len(), 1);
        let only = &deduped[0];
        assert_eq!(only.description.as_deref(), Some("meta"));
        assert_eq!(only.display.as_deref(), Some("Display"));
        assert_eq!(only.sort.as_deref(), Some("10"));
        assert_eq!(only.match_score, 80);
    }

    #[test]
    fn sort_ranked_suggestions_prioritizes_exact_then_match_score() {
        let mut items = vec![
            RankedSuggestion {
                value: "ldap-user".to_string(),
                description: None,
                display: None,
                sort: None,
                match_score: 180,
                is_exact: false,
            },
            RankedSuggestion {
                value: "ldap".to_string(),
                description: None,
                display: None,
                sort: None,
                match_score: 0,
                is_exact: true,
            },
            RankedSuggestion {
                value: "plugins".to_string(),
                description: None,
                display: None,
                sort: None,
                match_score: 420,
                is_exact: false,
            },
        ];

        sort_ranked_suggestions(&mut items);
        let ordered = items
            .iter()
            .map(|item| item.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec!["ldap", "ldap-user", "plugins"]);
    }

    #[test]
    fn highlighter_colors_full_command_chain_only() {
        let tree = completion_tree_with_config_show();
        let highlighter = ReplHighlighter::new(tree, Color::Green);

        let styled = highlighter.highlight("config show", 0);
        let tokens = token_styles(&styled);
        assert_eq!(
            tokens,
            vec![
                ("config".to_string(), Some(Color::Green)),
                ("show".to_string(), Some(Color::Green)),
            ]
        );
    }

    #[test]
    fn highlighter_skips_partial_subcommand_and_flags() {
        let tree = completion_tree_with_config_show();
        let highlighter = ReplHighlighter::new(tree, Color::Green);

        let styled = highlighter.highlight("config sho", 0);
        let tokens = token_styles(&styled);
        assert_eq!(
            tokens,
            vec![
                ("config".to_string(), Some(Color::Green)),
                ("sho".to_string(), None),
            ]
        );

        let styled = highlighter.highlight("config --flag", 0);
        let tokens = token_styles(&styled);
        assert_eq!(
            tokens,
            vec![
                ("config".to_string(), Some(Color::Green)),
                ("--flag".to_string(), None),
            ]
        );
    }

    #[test]
    fn highlighter_fallback_keeps_remaining_text_once() {
        let tree = completion_tree_with_config_show();
        let highlighter = ReplHighlighter::new(tree, Color::Green);
        let line = r#"config "say \"hi\"""#;

        let styled = highlighter.highlight(line, 0);
        assert_eq!(styled_text_to_plain(&styled), line);
    }

    #[test]
    fn split_path_stub_without_slash_uses_current_directory_lookup() {
        let (lookup, insert_prefix, typed_prefix) = super::split_path_stub("do");

        assert_eq!(lookup, PathBuf::from("."));
        assert_eq!(insert_prefix, "");
        assert_eq!(typed_prefix, "do");
    }
}
