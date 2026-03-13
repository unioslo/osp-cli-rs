use super::adapter::{ReplCompleter, ReplHistoryCompleter};
use super::config::{DEFAULT_HISTORY_MENU_ROWS, ReplAppearance};
use super::overlay::{build_completion_menu, build_history_menu};
use super::{HISTORY_MENU_NAME, SharedHistory};
use crate::completion::{CompletionEngine, CompletionTree};
use crate::repl::menu::{
    MenuDebug, MenuStyleDebug, OspCompletionMenu, debug_snapshot, display_text,
};
use reedline::{Completer, Editor, Menu, MenuEvent, UndoBehavior};
use serde::Serialize;

/// One rendered completion entry in the debug surface.
#[derive(Debug, Clone, Serialize)]
pub struct CompletionDebugMatch {
    /// Stable completion identifier or inserted value.
    pub id: String,
    /// Primary rendered label for the suggestion.
    pub label: String,
    /// Optional secondary description for the suggestion.
    pub description: Option<String>,
    /// Classified suggestion kind used by the debug view.
    pub kind: String,
}

/// Snapshot of completion/menu state for a given line and cursor position.
///
/// This is primarily consumed by REPL debug commands and tests.
#[derive(Debug, Clone, Serialize)]
pub struct CompletionDebug {
    /// Original input line.
    pub line: String,
    /// Cursor position used for analysis.
    pub cursor: usize,
    /// Replacement byte range for the active suggestion set.
    pub replace_range: [usize; 2],
    /// Current completion stub under the cursor.
    pub stub: String,
    /// Candidate matches visible to the menu.
    pub matches: Vec<CompletionDebugMatch>,
    /// Selected match index, or `-1` when no match is selected.
    pub selected: i64,
    /// Selected menu row.
    pub selected_row: u16,
    /// Selected menu column.
    pub selected_col: u16,
    /// Number of menu columns.
    pub columns: u16,
    /// Number of menu rows.
    pub rows: u16,
    /// Number of visible menu rows after clipping.
    pub visible_rows: u16,
    /// Menu indentation used for rendering.
    pub menu_indent: u16,
    /// Effective menu style snapshot.
    pub menu_styles: MenuStyleDebug,
    /// Optional selected-item description.
    pub menu_description: Option<String>,
    /// Rendered description as shown in the menu.
    pub menu_description_rendered: Option<String>,
    /// Virtual render width.
    pub width: u16,
    /// Virtual render height.
    pub height: u16,
    /// Whether Unicode menu chrome was enabled.
    pub unicode: bool,
    /// Whether ANSI styling was enabled.
    pub color: bool,
    /// Rendered menu lines captured for debugging.
    pub rendered: Vec<String>,
}

/// Synthetic editor action used by completion-debug stepping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugStep {
    /// Advance to the next item or open the menu.
    Tab,
    /// Move to the previous item.
    BackTab,
    /// Move selection upward.
    Up,
    /// Move selection downward.
    Down,
    /// Move selection left.
    Left,
    /// Move selection right.
    Right,
    /// Accept the selected item.
    Accept,
    /// Close the completion menu.
    Close,
}

impl DebugStep {
    /// Parses a step name accepted by REPL debug commands.
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

    /// Returns the stable lowercase name used in debug payloads.
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

/// One frame from a stepped completion-debug session.
#[derive(Debug, Clone, Serialize)]
pub struct CompletionDebugFrame {
    /// Synthetic step that produced this frame.
    pub step: String,
    /// Captured completion state after the step.
    pub state: CompletionDebug,
}

/// Rendering and capture options for completion-debug helpers.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
#[must_use]
pub struct CompletionDebugOptions<'a> {
    /// Virtual render width for the debug menu.
    pub width: u16,
    /// Virtual render height for the debug menu.
    pub height: u16,
    /// Whether ANSI styling should be enabled in captured output.
    pub ansi: bool,
    /// Whether Unicode menu chrome should be enabled in captured output.
    pub unicode: bool,
    /// Optional appearance override used for the debug session.
    pub appearance: Option<&'a ReplAppearance>,
}

impl<'a> CompletionDebugOptions<'a> {
    /// Creates a new debug snapshot configuration for a virtual terminal size.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::repl::CompletionDebugOptions;
    ///
    /// let options = CompletionDebugOptions::new(100, 12)
    ///     .with_ansi(true)
    ///     .with_unicode(true);
    ///
    /// assert_eq!(options.width, 100);
    /// assert_eq!(options.height, 12);
    /// assert!(options.ansi);
    /// assert!(options.unicode);
    /// ```
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            ansi: false,
            unicode: false,
            appearance: None,
        }
    }

    /// Enables ANSI styling in captured menu output.
    pub fn with_ansi(mut self, ansi: bool) -> Self {
        self.ansi = ansi;
        self
    }

    /// Selects unicode or ASCII menu chrome when rendering debug output.
    pub fn with_unicode(mut self, unicode: bool) -> Self {
        self.unicode = unicode;
        self
    }

    /// Applies REPL appearance overrides to the debug menu session.
    pub fn with_appearance(mut self, appearance: Option<&'a ReplAppearance>) -> Self {
        self.appearance = appearance;
        self
    }
}

/// Builds a single completion-debug snapshot for `line` at `cursor`.
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
    menu.apply_event(&mut editor, completer.as_mut());

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

/// Builds a single history-menu debug snapshot for `line` at `cursor`.
pub fn debug_history_menu(
    history: &SharedHistory,
    line: &str,
    cursor: usize,
    options: CompletionDebugOptions<'_>,
) -> CompletionDebug {
    let (editor, mut completer, mut menu) =
        build_debug_history_session(history, line, cursor, options.appearance);
    let mut editor = editor;

    menu.menu_event(MenuEvent::Activate(false));
    menu.apply_event(&mut editor, completer.as_mut());

    snapshot_history_debug(
        &mut menu,
        &editor,
        cursor,
        options.width,
        options.height,
        options.ansi,
        options.unicode,
    )
}

/// Replays a sequence of synthetic editor actions and captures each frame.
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
        apply_debug_step(step, &mut menu, &mut editor, completer.as_mut());
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

/// Replays synthetic editor actions for the history menu and captures each frame.
pub fn debug_history_menu_steps(
    history: &SharedHistory,
    line: &str,
    cursor: usize,
    options: CompletionDebugOptions<'_>,
    steps: &[DebugStep],
) -> Vec<CompletionDebugFrame> {
    let (mut editor, mut completer, mut menu) =
        build_debug_history_session(history, line, cursor, options.appearance);

    let steps = steps.to_vec();
    if steps.is_empty() {
        return Vec::new();
    }

    let mut frames = Vec::with_capacity(steps.len());
    for step in steps {
        apply_debug_step(step, &mut menu, &mut editor, completer.as_mut());
        let state = snapshot_history_debug(
            &mut menu,
            &editor,
            cursor,
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
) -> (Editor, Box<dyn Completer>, OspCompletionMenu) {
    let mut editor = Editor::default();
    editor.edit_buffer(
        |buf| {
            buf.set_buffer(line.to_string());
            buf.set_insertion_point(cursor.min(buf.get_buffer().len()));
        },
        UndoBehavior::CreateUndoPoint,
    );

    let completer = Box::new(ReplCompleter::new(Vec::new(), Some(tree.clone()), None));
    let menu = if let Some(appearance) = appearance {
        build_completion_menu(appearance)
    } else {
        OspCompletionMenu::default()
    };

    (editor, completer, menu)
}

fn build_debug_history_session(
    history: &SharedHistory,
    line: &str,
    cursor: usize,
    appearance: Option<&ReplAppearance>,
) -> (Editor, Box<dyn Completer>, OspCompletionMenu) {
    let mut editor = Editor::default();
    editor.edit_buffer(
        |buf| {
            buf.set_buffer(line.to_string());
            buf.set_insertion_point(cursor.min(buf.get_buffer().len()));
        },
        UndoBehavior::CreateUndoPoint,
    );

    let completer = Box::new(ReplHistoryCompleter::new(history.clone()));
    let menu = if let Some(appearance) = appearance {
        build_history_menu(appearance)
    } else {
        OspCompletionMenu::default()
            .with_name(HISTORY_MENU_NAME)
            .with_quick_complete(false)
            .with_columns(1)
            .with_max_rows(DEFAULT_HISTORY_MENU_ROWS)
    };

    (editor, completer, menu)
}

fn apply_debug_step(
    step: DebugStep,
    menu: &mut OspCompletionMenu,
    editor: &mut Editor,
    completer: &mut dyn Completer,
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
    completer: &mut dyn Completer,
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

fn snapshot_history_debug(
    menu: &mut OspCompletionMenu,
    editor: &Editor,
    cursor: usize,
    width: u16,
    height: u16,
    ansi: bool,
    unicode: bool,
) -> CompletionDebug {
    let line = editor.get_buffer().to_string();
    let cursor = cursor
        .min(editor.line_buffer().insertion_point())
        .min(line.len());
    let values = menu.get_values();
    let query = line.get(..cursor).unwrap_or(&line).trim().to_string();

    let (stub, replace_range) = if let Some(first) = values.first() {
        let start = first.span.start;
        let end = first.span.end;
        let stub = line.get(start..end).unwrap_or("").to_string();
        (stub, [start, end])
    } else {
        (query, [0, line.len()])
    };

    let matches = values
        .iter()
        .map(|item| CompletionDebugMatch {
            id: item.value.clone(),
            label: display_text(item).to_string(),
            description: item.description.clone(),
            kind: "history".to_string(),
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
