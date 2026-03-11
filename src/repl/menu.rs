use reedline::{
    Completer, Editor, Menu, MenuEvent, MenuTextStyle, Painter, Span, Suggestion,
    menu_functions::{can_partially_complete, replace_in_buffer},
};
use unicode_width::UnicodeWidthStr;

use crate::repl::CompletionTraceMenuState;
use crate::repl::menu_core::{MenuAction, MenuCore};

pub(crate) use crate::repl::menu_core::{MenuDebug, MenuStyleDebug, display_text};

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
    quick_complete: bool,
    colors: MenuTextStyle,
    core: MenuCore,
    replace_span: Option<Span>,
    // Keep popup alignment stable while cycling so repaint follows the token,
    // not the cursor column drift from accepted candidates.
    indent_anchor: Option<u16>,
    cursor_col: u16,
    last_available_lines: u16,
    // Queue one reedline menu event so editor mutation, match refresh, and
    // subsequent render all observe the same step. Applying navigation here in
    // `menu_event` would let the editor buffer advance before the refresh pass
    // updates menu state, which is how prompt/menu drift can happen.
    event: Option<MenuEvent>,
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
            quick_complete: true,
            colors: MenuTextStyle::default(),
            core: MenuCore::default(),
            replace_span: None,
            indent_anchor: None,
            cursor_col: 0,
            last_available_lines: 0,
            event: None,
        }
    }
}

impl OspCompletionMenu {
    /// Sets the menu name used by `reedline`.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Limits completion input to the buffer prefix before the cursor.
    pub fn with_only_buffer_difference(mut self, only_buffer_difference: bool) -> Self {
        self.only_buffer_difference = only_buffer_difference;
        self
    }

    /// Controls whether a single menu match may be auto-accepted by reedline.
    pub fn with_quick_complete(mut self, quick_complete: bool) -> Self {
        self.quick_complete = quick_complete;
        self
    }

    /// Sets the marker shown when the menu is active.
    pub fn with_marker(mut self, marker: &str) -> Self {
        self.marker = marker.to_string();
        self
    }

    /// Sets the preferred column count for menu layout.
    pub fn with_columns(mut self, columns: u16) -> Self {
        self.core.set_columns(columns);
        self
    }

    /// Sets horizontal padding between menu columns.
    pub fn with_column_padding(mut self, col_padding: usize) -> Self {
        self.core.set_column_padding(col_padding);
        self
    }

    /// Sets the maximum number of visible menu rows.
    pub fn with_max_rows(mut self, max_rows: u16) -> Self {
        self.core.set_max_rows(max_rows);
        self
    }

    /// Sets how many wrapped description rows may be rendered.
    pub fn with_description_rows(mut self, description_rows: usize) -> Self {
        self.core.set_description_rows(description_rows);
        self
    }

    /// Sets the base text style for unselected items.
    pub fn with_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.text_style = color;
        self
    }

    /// Sets the text style for the selected item.
    pub fn with_selected_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.selected_text_style = color;
        self
    }

    /// Sets the style used for item descriptions.
    pub fn with_description_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.description_style = color;
        self
    }

    /// Sets the style used for matched text in unselected items.
    pub fn with_match_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.match_style = color;
        self
    }

    /// Sets the style used for matched text in the selected item.
    pub fn with_selected_match_text_style(mut self, color: nu_ansi_term::Style) -> Self {
        self.colors.selected_match_style = color;
        self
    }

    pub(crate) fn apply_event(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        if let Some(event) = self.event.take() {
            // Apply queued menu events from the refresh path so the editor
            // buffer, candidate set, and rendered menu all advance from the
            // same state transition.
            let action = self.core.handle_event(event);
            if matches!(action, MenuAction::UpdateValues) {
                self.update_values(editor, completer);
            }
            if matches!(action, MenuAction::ApplySelection) {
                self.apply_selection_in_buffer(editor, ApplyMode::Cycle);
            }
        }
    }

    fn stable_menu_indent(&mut self, editor: &Editor) -> u16 {
        if let Some(indent) = self.indent_anchor {
            return indent;
        }
        // Latch the first live indent so cycling across replacements does not
        // make the menu "walk" horizontally between frames.
        let indent = compute_menu_indent(self, editor);
        if self.core.is_active() && !self.core.values().is_empty() {
            self.indent_anchor = Some(indent);
        }
        indent
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

    fn update_working_details_inner(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        screen_width: u16,
        available_lines: u16,
    ) {
        self.apply_event(editor, completer);
        self.last_available_lines = available_lines;
        let indent = self.stable_menu_indent(editor);
        self.core.update_layout(screen_width, indent);
        trace_menu_state_with_available_lines(self, editor, available_lines);
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
        self.quick_complete
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
            self.indent_anchor = None;
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
        if self.core.values().is_empty() {
            self.indent_anchor = None;
        }
    }

    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        self.update_working_details_inner(
            editor,
            completer,
            painter.screen_width(),
            painter.remaining_lines(),
        );
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

fn trace_menu_state_with_available_lines(
    menu: &OspCompletionMenu,
    editor: &Editor,
    available_lines: u16,
) {
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
        .replace_span
        .map(|span| span.start.min(line.len()))
        .or_else(|| {
            menu.core
                .values()
                .first()
                .map(|suggestion| suggestion.span.start.min(line.len()))
        })
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
        self.update_for_test_with_available_lines(editor, completer, screen_width, u16::MAX);
    }

    fn update_for_test_with_available_lines(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        screen_width: u16,
        available_lines: u16,
    ) {
        self.update_working_details_inner(editor, completer, screen_width, available_lines);
    }

    fn columns_for_test(&self) -> u16 {
        self.core.columns_for_test()
    }
}

#[cfg(test)]
mod tests;
