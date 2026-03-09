use reedline::{
    Completer, Editor, Menu, MenuEvent, MenuTextStyle, Painter, Span, Suggestion,
    menu_functions::{can_partially_complete, replace_in_buffer},
};
use std::cell::Cell;
use unicode_width::UnicodeWidthStr;

use crate::repl::CompletionTraceMenuState;
use crate::repl::menu_core::{MenuAction, MenuCore};

#[allow(unused_imports)]
pub(crate) use crate::repl::menu_core::{MenuDebug, MenuStyleDebug, StyleDebug, display_text};

/// Completion menu with adaptive column layout and an optional meta line.
///
/// This is a forked, simplified version of reedline's ColumnarMenu that:
/// - Pads each cell with background style to avoid "island" chips.
/// - Uses display labels from `Suggestion.extra` when available.
/// - Shows the selected item's description as a single meta line.
pub struct OspCompletionMenu {
    name: String,
    marker: String,
    // When true, only pass the buffer prefix up to the cursor to the completer.
    only_buffer_difference: bool,
    colors: MenuTextStyle,
    core: MenuCore,
    replace_span: Option<Span>,
    cursor_col: u16,
    last_available_lines: u16,
    event: Option<MenuEvent>,
}

thread_local! {
    static ACTIVE_EDITOR: Cell<*mut Editor> = const { Cell::new(std::ptr::null_mut()) };
}

/// Scoped registration for the reedline editor pointer used by menu callbacks.
///
/// reedline's menu callback API does not pass the active `Editor` into
/// `menu_event`, so the menu keeps a same-thread raw pointer while it runs the
/// callback path. This is only sound as long as:
/// - the pointer is registered and used on the same thread
/// - the pointed-to `Editor` outlives the callback scope
/// - the thread-local is cleared before the editor can be moved or dropped
///
/// The guard enforces the last invariant so future REPL lifecycle refactors do
/// not accidentally leave a stale pointer behind.
struct ActiveEditorGuard;

impl ActiveEditorGuard {
    fn register(editor: &mut Editor) -> Self {
        ACTIVE_EDITOR.with(|cell| cell.set(editor as *mut Editor));
        Self
    }
}

impl Drop for ActiveEditorGuard {
    fn drop(&mut self) {
        ACTIVE_EDITOR.with(|cell| cell.set(std::ptr::null_mut()));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyMode {
    Cycle,
    Accept,
}

impl Default for OspCompletionMenu {
    fn default() -> Self {
        Self {
            name: "completion_menu".to_string(),
            marker: "| ".to_string(),
            only_buffer_difference: false,
            colors: MenuTextStyle::default(),
            core: MenuCore::default(),
            replace_span: None,
            cursor_col: 0,
            last_available_lines: 0,
            event: None,
        }
    }
}

impl OspCompletionMenu {
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    pub fn with_only_buffer_difference(mut self, only_buffer_difference: bool) -> Self {
        self.only_buffer_difference = only_buffer_difference;
        self
    }

    pub fn with_marker(mut self, marker: &str) -> Self {
        self.marker = marker.to_string();
        self
    }

    pub fn with_columns(mut self, columns: u16) -> Self {
        self.core.set_columns(columns);
        self
    }

    pub fn with_column_padding(mut self, col_padding: usize) -> Self {
        self.core.set_column_padding(col_padding);
        self
    }

    pub fn with_max_rows(mut self, max_rows: u16) -> Self {
        self.core.set_max_rows(max_rows);
        self
    }

    pub fn with_description_rows(mut self, description_rows: usize) -> Self {
        self.core.set_description_rows(description_rows);
        self
    }

    pub fn with_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.text_style = color;
        self
    }

    pub fn with_selected_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.selected_text_style = color;
        self
    }

    pub fn with_description_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.description_style = color;
        self
    }

    pub fn with_match_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.match_style = color;
        self
    }

    pub fn with_selected_match_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.selected_match_style = color;
        self
    }

    pub(crate) fn apply_event(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        if let Some(event) = self.event.take() {
            let action = self.core.handle_event(event);
            if matches!(action, MenuAction::UpdateValues) {
                self.update_values(editor, completer);
            }
            if matches!(action, MenuAction::ApplySelection) {
                self.apply_selection_in_buffer(editor, ApplyMode::Cycle);
            }
        }
    }

    fn apply_selection_in_buffer(&mut self, editor: &mut Editor, mode: ApplyMode) {
        if let Some((start, value_len, prefixed_space)) = self.apply_selection(editor, mode)
            && matches!(mode, ApplyMode::Cycle)
        {
            let token_start = if prefixed_space {
                start.saturating_add(1)
            } else {
                start
            };
            self.replace_span = Some(Span {
                start: token_start,
                end: token_start + value_len,
            });
        }
    }

    pub(crate) fn accept_selection_in_buffer(&self, editor: &mut Editor) {
        self.apply_selection(editor, ApplyMode::Accept);
    }

    fn apply_selection(
        &self,
        editor: &mut Editor,
        mode: ApplyMode,
    ) -> Option<(usize, usize, bool)> {
        let suggestion = match mode {
            ApplyMode::Accept => self.core.selected_value()?.clone(),
            ApplyMode::Cycle => self.core.selected_value()?.clone(),
        };
        let line_before = editor.get_buffer().to_string();
        let cursor_before = editor.line_buffer().insertion_point();

        let base_span = self.replace_span.unwrap_or(suggestion.span);
        let start = base_span.start.min(line_before.len());
        let mut end = base_span.end.min(line_before.len());
        if end < start {
            end = start;
        }

        let stub = line_before.get(start..end).unwrap_or("").to_string();
        let replace_range = Some([start, end]);
        let matches = self
            .core
            .values()
            .iter()
            .map(|item| item.value.clone())
            .collect::<Vec<_>>();

        let available_lines = if self.last_available_lines > 0 {
            self.last_available_lines
        } else {
            u16::MAX
        };
        let menu_state = CompletionTraceMenuState {
            selected_index: self
                .core
                .selected_index()
                .map(|idx| idx as i64)
                .unwrap_or(-1),
            selected_row: self.core.selected_row(),
            selected_col: self.core.selected_col(),
            active: self.core.is_active(),
            just_activated: self.core.just_activated(),
            columns: self.core.columns(),
            visible_rows: self.core.visible_rows_for(available_lines),
            rows: self.core.rows(),
            menu_indent: self.core.input_indent(),
        };

        let prefixed_space = needs_space_prefix(&line_before, start, end);
        let mut replacement = String::new();
        if prefixed_space {
            replacement.push(' ');
        }
        replacement.push_str(&suggestion.value);
        if matches!(mode, ApplyMode::Accept) && suggestion.append_whitespace {
            replacement.push(' ');
        }

        let mut adjusted = suggestion.clone();
        adjusted.value = replacement.clone();
        adjusted.span = Span { start, end };
        adjusted.append_whitespace = false;

        replace_in_buffer(Some(adjusted), editor);

        if crate::repl::trace_completion_enabled() {
            let line_after = editor.get_buffer().to_string();
            let cursor_after = editor.line_buffer().insertion_point();
            let event = match mode {
                ApplyMode::Cycle => "cycle",
                ApplyMode::Accept => "accept",
            };
            crate::repl::trace_completion(crate::repl::CompletionTraceEvent {
                event,
                line: &line_before,
                cursor: cursor_before,
                stub: &stub,
                matches,
                replace_range,
                menu: Some(menu_state),
                buffer_before: Some(&line_before),
                buffer_after: Some(&line_after),
                cursor_before: Some(cursor_before),
                cursor_after: Some(cursor_after),
                accepted_value: Some(&suggestion.value),
            });
        }

        Some((start, suggestion.value.len(), prefixed_space))
    }
}

impl Menu for OspCompletionMenu {
    fn name(&self) -> &str {
        &self.name
    }

    fn indicator(&self) -> &str {
        if self.core.values().is_empty() {
            ""
        } else {
            &self.marker
        }
    }

    fn is_active(&self) -> bool {
        self.core.is_active()
    }

    fn can_quick_complete(&self) -> bool {
        true
    }

    fn can_partially_complete(
        &mut self,
        values_updated: bool,
        editor: &mut Editor,
        completer: &mut dyn Completer,
    ) -> bool {
        if !values_updated {
            self.update_values(editor, completer);
        }

        if can_partially_complete(self.core.values(), editor) {
            self.update_values(editor, completer);
            true
        } else {
            false
        }
    }

    fn menu_event(&mut self, event: MenuEvent) {
        self.core.pre_event(&event);
        if matches!(event, MenuEvent::Activate(_) | MenuEvent::Deactivate) {
            self.replace_span = None;
        }

        if matches!(
            event,
            MenuEvent::NextElement
                | MenuEvent::PreviousElement
                | MenuEvent::MoveUp
                | MenuEvent::MoveDown
                | MenuEvent::MoveLeft
                | MenuEvent::MoveRight
        ) {
            let ptr = ACTIVE_EDITOR.with(|cell| cell.get());
            if !ptr.is_null() {
                // SAFETY: `ActiveEditorGuard` registers this pointer from the same
                // thread immediately before menu callbacks run and clears it on
                // scope exit, so the editor outlives this dereference.
                let editor = unsafe { &mut *ptr };
                let action = self.core.handle_event(event);
                if matches!(action, MenuAction::ApplySelection) {
                    self.apply_selection_in_buffer(editor, ApplyMode::Cycle);
                }
                return;
            }
        }

        self.event = Some(event);
    }

    fn update_values(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        let buffer = editor.get_buffer();
        let pos = editor.line_buffer().insertion_point();
        let input = if self.only_buffer_difference {
            buffer.get(0..pos).unwrap_or(buffer)
        } else {
            buffer
        };
        let values = completer.complete(input, pos);
        self.core.set_values(values);
        self.replace_span = self.core.values().first().map(|item| item.span);
    }

    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        let _active_editor = ActiveEditorGuard::register(editor);
        self.apply_event(editor, completer);
        self.last_available_lines = painter.remaining_lines();
        let indent = compute_menu_indent(self, editor);
        self.core.update_layout(painter.screen_width(), indent);
        trace_menu_state(self, editor, painter);
    }

    fn replace_in_buffer(&self, editor: &mut Editor) {
        self.apply_selection(editor, ApplyMode::Accept);
    }

    fn menu_required_lines(&self, _terminal_columns: u16) -> u16 {
        self.core.menu_required_lines()
    }

    fn menu_string(&self, available_lines: u16, use_ansi_coloring: bool) -> String {
        self.core
            .menu_string(available_lines, use_ansi_coloring, &self.colors)
    }

    fn min_rows(&self) -> u16 {
        1
    }

    fn get_values(&self) -> &[Suggestion] {
        self.core.values()
    }

    fn set_cursor_pos(&mut self, pos: (u16, u16)) {
        self.cursor_col = pos.0;
    }
}

pub(crate) fn debug_snapshot(
    menu: &mut OspCompletionMenu,
    editor: &Editor,
    screen_width: u16,
    screen_height: u16,
    ansi: bool,
) -> MenuDebug {
    let indent = compute_menu_indent(menu, editor);
    menu.core
        .debug_snapshot(&menu.colors, screen_width, screen_height, indent, ansi)
}

fn trace_menu_state(menu: &OspCompletionMenu, editor: &Editor, painter: &Painter) {
    if !crate::repl::trace_completion_enabled() {
        return;
    }
    let line = editor.get_buffer().to_string();
    let cursor = editor.line_buffer().insertion_point();
    let values = menu.core.values();
    let (stub, replace_range) = if let Some(first) = values.first() {
        let start = first.span.start;
        let end = first.span.end;
        let stub = line.get(start..end).unwrap_or("").to_string();
        (stub, Some([start, end]))
    } else {
        (String::new(), None)
    };
    let matches = values
        .iter()
        .map(|item| item.value.clone())
        .collect::<Vec<_>>();

    let available_lines = painter.remaining_lines();
    let selected_index = menu
        .core
        .selected_index()
        .map(|idx| idx as i64)
        .unwrap_or(-1);

    let menu_state = CompletionTraceMenuState {
        selected_index,
        selected_row: menu.core.selected_row(),
        selected_col: menu.core.selected_col(),
        active: menu.core.is_active(),
        just_activated: menu.core.just_activated(),
        columns: menu.core.columns(),
        visible_rows: menu.core.visible_rows_for(available_lines),
        rows: menu.core.rows(),
        menu_indent: menu.core.input_indent(),
    };

    crate::repl::trace_completion(crate::repl::CompletionTraceEvent {
        event: "complete",
        line: &line,
        cursor,
        stub: &stub,
        matches,
        replace_range,
        menu: Some(menu_state),
        buffer_before: None,
        buffer_after: None,
        cursor_before: None,
        cursor_after: None,
        accepted_value: None,
    });
}

fn needs_space_prefix(line: &str, start: usize, end: usize) -> bool {
    if start != end || start == 0 {
        return false;
    }
    let Some(prefix) = line.get(..start) else {
        return false;
    };
    let Some(prev) = prefix.chars().last() else {
        return false;
    };
    !prev.is_whitespace() && prev != '='
}

fn compute_menu_indent(menu: &OspCompletionMenu, editor: &Editor) -> u16 {
    let line = editor.get_buffer();
    let span_start = menu
        .core
        .values()
        .first()
        .map(|s| s.span.start.min(line.len()))
        .unwrap_or_else(|| editor.line_buffer().insertion_point().min(line.len()));
    let prefix = line.get(0..span_start).unwrap_or("");
    let prefix_width = prefix.width();
    let cursor = editor.line_buffer().insertion_point().min(line.len());
    let cursor_prefix = line.get(0..cursor).unwrap_or("");
    let cursor_prefix_width = cursor_prefix.width();
    let prompt_width = menu
        .cursor_col
        .saturating_sub(cursor_prefix_width.min(u16::MAX as usize) as u16);
    let width = prompt_width as usize + prefix_width;
    width.min(u16::MAX as usize) as u16
}

#[cfg(test)]
impl OspCompletionMenu {
    fn update_for_test(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        screen_width: u16,
    ) {
        let _active_editor = ActiveEditorGuard::register(editor);
        self.apply_event(editor, completer);
        let indent = compute_menu_indent(self, editor);
        self.core.update_layout(screen_width, indent);
    }

    fn columns_for_test(&self) -> u16 {
        self.core.columns_for_test()
    }
}

#[cfg(test)]
mod tests {
    use super::{OspCompletionMenu, needs_space_prefix};
    use nu_ansi_term::{Color, Style};
    use reedline::{Completer, Editor, Menu, MenuEvent, Span, Suggestion, UndoBehavior};
    use unicode_width::UnicodeWidthStr;

    #[derive(Clone)]
    struct FixedCompleter {
        suggestions: Vec<Suggestion>,
    }

    impl Completer for FixedCompleter {
        fn complete(&mut self, _line: &str, _pos: usize) -> Vec<Suggestion> {
            self.suggestions.clone()
        }
    }

    #[derive(Clone)]
    struct DynamicSpanCompleter;

    impl Completer for DynamicSpanCompleter {
        fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
            let start = line
                .get(..pos)
                .unwrap_or("")
                .rfind(|ch: char| ch.is_whitespace())
                .map(|idx| idx + 1)
                .unwrap_or(0);
            let span = Span { start, end: pos };
            vec![suggestion("config", span), suggestion("doctor", span)]
        }
    }

    fn set_buffer(editor: &mut Editor, buffer: &str) {
        editor.edit_buffer(
            |buf| buf.set_buffer(buffer.to_string()),
            UndoBehavior::CreateUndoPoint,
        );
    }

    fn suggestion(value: &str, span: Span) -> Suggestion {
        Suggestion {
            value: value.to_string(),
            span,
            append_whitespace: true,
            ..Suggestion::default()
        }
    }

    fn split_lines(output: &str) -> Vec<&str> {
        output.split_terminator("\r\n").collect()
    }

    #[test]
    fn tab_cycles_selection_replaces_buffer() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "co");

        let mut completer = DynamicSpanCompleter;
        let mut menu = OspCompletionMenu::default();

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        let debug = super::debug_snapshot(&mut menu, &editor, 80, 5, false);
        assert_eq!(debug.selected_index, 0);
        assert_eq!(editor.line_buffer().get_buffer(), "co");

        menu.menu_event(MenuEvent::NextElement);
        menu.update_for_test(&mut editor, &mut completer, 80);

        let debug = super::debug_snapshot(&mut menu, &editor, 80, 5, false);
        assert_eq!(debug.selected_index, 0);
        assert_eq!(editor.line_buffer().get_buffer(), "config");

        menu.menu_event(MenuEvent::NextElement);
        menu.update_for_test(&mut editor, &mut completer, 80);

        let debug = super::debug_snapshot(&mut menu, &editor, 80, 5, false);
        assert_eq!(debug.selected_index, 1);
        assert_eq!(editor.line_buffer().get_buffer(), "doctor");
    }

    #[test]
    fn explicit_accept_applies_selected_completion() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "doctor ");
        let insert_at = editor.line_buffer().len();

        let suggestions = vec![
            suggestion(
                "all",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
            suggestion(
                "config",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
            suggestion(
                "plugins",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
            suggestion(
                "theme",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
        ];
        let mut completer = FixedCompleter { suggestions };
        let mut menu = OspCompletionMenu::default();

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        menu.accept_selection_in_buffer(&mut editor);

        assert_eq!(editor.line_buffer().get_buffer(), "doctor all ");
    }

    #[test]
    fn replace_in_buffer_accepts_selected_completion() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "doctor ");
        let insert_at = editor.line_buffer().len();

        let suggestions = vec![
            suggestion(
                "all",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
            suggestion(
                "config",
                Span {
                    start: insert_at,
                    end: insert_at,
                },
            ),
        ];
        let mut completer = FixedCompleter { suggestions };
        let mut menu = OspCompletionMenu::default();

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        menu.replace_in_buffer(&mut editor);

        assert_eq!(editor.line_buffer().get_buffer(), "doctor all ");
    }

    #[test]
    fn menu_uses_display_text_when_present() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let mut first = suggestion("config", Span { start: 0, end: 0 });
        first.extra = Some(vec!["Configure".to_string()]);
        let second = suggestion("doctor", Span { start: 0, end: 0 });

        let mut completer = FixedCompleter {
            suggestions: vec![first, second],
        };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        let output = menu.menu_string(10, false);
        assert!(output.contains("Configure"));
        assert!(output.contains("doctor"));
    }

    #[test]
    fn menu_shows_description_for_selected_item() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let mut first = suggestion("config", Span { start: 0, end: 0 });
        first.description = Some("Inspect and edit runtime config".to_string());
        let second = suggestion("doctor", Span { start: 0, end: 0 });

        let mut completer = FixedCompleter {
            suggestions: vec![first, second],
        };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        menu.menu_event(MenuEvent::NextElement);
        menu.update_for_test(&mut editor, &mut completer, 80);

        let output = menu.menu_string(10, false);
        let lines = split_lines(&output);
        let last = lines.last().map(|line| line.trim()).unwrap_or_default();
        assert!(!last.is_empty());
        assert!("Inspect and edit runtime config".starts_with(last));
    }

    #[test]
    fn menu_truncates_description_on_narrow_width() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let mut first = suggestion("config", Span { start: 0, end: 0 });
        first.description = Some("this description is very long".to_string());

        let mut completer = FixedCompleter {
            suggestions: vec![first],
        };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 10);

        menu.menu_event(MenuEvent::NextElement);
        menu.update_for_test(&mut editor, &mut completer, 10);

        let output = menu.menu_string(10, false);
        assert!(output.contains("this"));
        assert!(!output.contains("description is very long"));
    }

    #[test]
    fn menu_narrows_columns_on_small_width() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");

        let suggestions = vec![
            suggestion("alpha", Span { start: 0, end: 0 }),
            suggestion("bravo", Span { start: 0, end: 0 }),
            suggestion("charlie", Span { start: 0, end: 0 }),
            suggestion("delta", Span { start: 0, end: 0 }),
        ];

        let mut completer = FixedCompleter {
            suggestions: suggestions.clone(),
        };
        let mut menu_small = OspCompletionMenu::default();
        menu_small.menu_event(MenuEvent::Activate(false));
        menu_small.update_for_test(&mut editor, &mut completer, 10);
        assert_eq!(menu_small.columns_for_test(), 1);

        let mut completer = FixedCompleter { suggestions };
        let mut menu_large = OspCompletionMenu::default();
        menu_large.menu_event(MenuEvent::Activate(false));
        menu_large.update_for_test(&mut editor, &mut completer, 80);
        assert!(menu_large.columns_for_test() > 1);
    }

    #[test]
    fn menu_respects_available_lines() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let mut suggestions = Vec::new();
        for idx in 0..10 {
            suggestions.push(suggestion(&format!("item{idx}"), Span { start: 0, end: 0 }));
        }
        let mut completer = FixedCompleter { suggestions };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        let output = menu.menu_string(1, false);
        let lines = split_lines(&output);
        assert!(!lines.is_empty());
        assert!(lines.len() <= 1);
    }

    #[test]
    fn menu_ansi_coloring_preserves_case_and_resets() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let suggestions = vec![
            suggestion("config", Span { start: 0, end: 0 }),
            suggestion("doctor", Span { start: 0, end: 0 }),
        ];
        let mut completer = FixedCompleter { suggestions };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        let output = menu.menu_string(10, true);
        assert!(output.contains("config"));
        assert!(!output.contains("CONFIG"));
        assert!(output.contains("\u{1b}["));
        assert!(output.contains("\u{1b}[0m"));
    }

    #[test]
    fn menu_non_ansi_marks_selected_item() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let suggestions = vec![
            suggestion("config", Span { start: 0, end: 0 }),
            suggestion("doctor", Span { start: 0, end: 0 }),
        ];
        let mut completer = FixedCompleter { suggestions };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        menu.menu_event(MenuEvent::NextElement);
        menu.update_for_test(&mut editor, &mut completer, 80);

        let output = menu.menu_string(10, false);
        assert!(output.contains("> config"));
        assert!(output.contains("  doctor"));
    }

    #[test]
    fn menu_width_never_exceeds_screen_width_without_ansi() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let suggestions = vec![
            suggestion("alpha", Span { start: 0, end: 0 }),
            suggestion("bravo", Span { start: 0, end: 0 }),
            suggestion("charlie", Span { start: 0, end: 0 }),
        ];
        let mut completer = FixedCompleter { suggestions };

        let screen_width = 20;
        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, screen_width);

        let output = menu.menu_string(10, false);
        for line in split_lines(&output) {
            assert!(line.width() <= screen_width as usize);
        }
    }

    #[test]
    fn menu_description_is_omitted_when_no_lines_available() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "");
        let mut menu = OspCompletionMenu::default();

        let mut first = suggestion("config", Span { start: 0, end: 0 });
        first.description = Some("Long description".to_string());
        let mut completer = FixedCompleter {
            suggestions: vec![first],
        };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        menu.menu_event(MenuEvent::NextElement);
        menu.update_for_test(&mut editor, &mut completer, 80);

        let output = menu.menu_string(1, false);
        let lines = split_lines(&output);
        assert_eq!(lines.len(), 1);
        assert!(!output.contains("Long description"));
    }

    #[test]
    fn menu_debug_reports_styles_and_selection() {
        let span = Span { start: 0, end: 0 };
        let suggestions = vec![suggestion("config", span)];
        let mut menu = OspCompletionMenu::default()
            .with_text_style(Style::new().fg(Color::Red).on(Color::Black))
            .with_selected_text_style(Style::new().fg(Color::Green).on(Color::Blue))
            .with_description_text_style(Style::new().fg(Color::Yellow))
            .with_match_text_style(Style::new().fg(Color::Cyan))
            .with_selected_match_text_style(Style::new().fg(Color::Magenta));

        let mut editor = Editor::default();
        let mut completer = FixedCompleter { suggestions };

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 20);

        let debug = super::debug_snapshot(&mut menu, &editor, 20, 5, false);
        assert_eq!(debug.styles.text.foreground.as_deref(), Some("red"));
        assert_eq!(debug.styles.text.background.as_deref(), Some("black"));
        assert_eq!(
            debug.styles.selected_text.foreground.as_deref(),
            Some("green")
        );
        assert_eq!(
            debug.styles.selected_text.background.as_deref(),
            Some("blue")
        );
        assert_eq!(
            debug.styles.description.foreground.as_deref(),
            Some("yellow")
        );
        assert_eq!(debug.styles.match_text.foreground.as_deref(), Some("cyan"));
        assert_eq!(
            debug.styles.selected_match.foreground.as_deref(),
            Some("magenta")
        );
        assert_eq!(debug.selected_index, 0);
        assert_eq!(debug.selected_row, 0);
        assert_eq!(debug.selected_col, 0);
    }

    #[test]
    fn indicator_is_empty_until_menu_has_values() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "co");
        let mut completer = DynamicSpanCompleter;
        let mut menu = OspCompletionMenu::default().with_marker(">> ");

        assert_eq!(menu.indicator(), "");

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);

        assert_eq!(menu.indicator(), ">> ");
        assert!(menu.menu_required_lines(80) >= 1);
    }

    #[test]
    fn partial_complete_uses_buffer_prefix_when_requested() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "config sh");
        let cursor = editor.line_buffer().len();
        let mut completer = FixedCompleter {
            suggestions: vec![
                suggestion(
                    "show",
                    Span {
                        start: cursor - 2,
                        end: cursor,
                    },
                ),
                suggestion(
                    "shell",
                    Span {
                        start: cursor - 2,
                        end: cursor,
                    },
                ),
            ],
        };
        let mut menu = OspCompletionMenu::default().with_only_buffer_difference(true);

        assert!(menu.can_partially_complete(false, &mut editor, &mut completer));
    }

    #[test]
    fn replace_in_buffer_inserts_missing_space_before_completion() {
        let mut editor = Editor::default();
        set_buffer(&mut editor, "doctor");
        let cursor = editor.line_buffer().len();
        let mut completer = FixedCompleter {
            suggestions: vec![suggestion(
                "config",
                Span {
                    start: cursor,
                    end: cursor,
                },
            )],
        };
        let mut menu = OspCompletionMenu::default();

        menu.menu_event(MenuEvent::Activate(false));
        menu.update_for_test(&mut editor, &mut completer, 80);
        menu.menu_event(MenuEvent::NextElement);
        menu.update_for_test(&mut editor, &mut completer, 80);
        menu.accept_selection_in_buffer(&mut editor);

        assert_eq!(editor.line_buffer().get_buffer(), "doctor config ");
        assert!(needs_space_prefix("doctor", 6, 6));
        assert!(!needs_space_prefix("doctor ", 7, 7));
        assert!(!needs_space_prefix("a=", 2, 2));
    }
}
