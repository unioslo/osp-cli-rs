use nu_ansi_term::ansi::RESET;
use reedline::{
    Completer, Editor, Menu, MenuEvent, MenuTextStyle, Painter, Suggestion,
    menu_functions::{can_partially_complete, completer_input, replace_in_buffer},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone)]
struct DefaultGridDetails {
    columns: u16,
    col_padding: usize,
    max_rows: u16,
    description_rows: usize,
}

impl Default for DefaultGridDetails {
    fn default() -> Self {
        Self {
            columns: 20,
            col_padding: 2,
            max_rows: 20,
            description_rows: 1,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct WorkingGridDetails {
    columns: u16,
    col_widths: Vec<usize>,
}

/// Completion menu with adaptive column layout and an optional meta line.
///
/// This is a forked, simplified version of reedline's ColumnarMenu that:
/// - Pads each cell with background style to avoid "island" chips.
/// - Uses display labels from `Suggestion.extra` when available.
/// - Shows the selected item's description as a single meta line.
pub struct OspCompletionMenu {
    name: String,
    marker: String,
    only_buffer_difference: bool,
    colors: MenuTextStyle,
    active: bool,
    default_details: DefaultGridDetails,
    working_details: WorkingGridDetails,
    values: Vec<Suggestion>,
    col_pos: u16,
    row_pos: u16,
    cursor_col: u16,
    input_indent: u16,
    event: Option<MenuEvent>,
    input: Option<String>,
}

impl Default for OspCompletionMenu {
    fn default() -> Self {
        Self {
            name: "completion_menu".to_string(),
            marker: "| ".to_string(),
            only_buffer_difference: true,
            colors: MenuTextStyle::default(),
            active: false,
            default_details: DefaultGridDetails::default(),
            working_details: WorkingGridDetails::default(),
            values: Vec::new(),
            col_pos: 0,
            row_pos: 0,
            cursor_col: 0,
            input_indent: 0,
            event: None,
            input: None,
        }
    }
}

impl OspCompletionMenu {
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    pub fn with_marker(mut self, marker: &str) -> Self {
        self.marker = marker.to_string();
        self
    }

    pub fn with_only_buffer_difference(mut self, only_buffer_difference: bool) -> Self {
        self.only_buffer_difference = only_buffer_difference;
        self
    }

    pub fn with_columns(mut self, columns: u16) -> Self {
        self.default_details.columns = columns.max(1);
        self
    }

    pub fn with_column_padding(mut self, col_padding: usize) -> Self {
        self.default_details.col_padding = col_padding;
        self
    }

    pub fn with_max_rows(mut self, max_rows: u16) -> Self {
        self.default_details.max_rows = max_rows.max(1);
        self
    }

    pub fn with_description_rows(mut self, description_rows: usize) -> Self {
        self.default_details.description_rows = description_rows;
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

    fn reset_position(&mut self) {
        self.col_pos = 0;
        self.row_pos = 0;
    }

    fn index(&self) -> usize {
        (self.row_pos * self.get_cols() + self.col_pos) as usize
    }

    fn get_cols(&self) -> u16 {
        self.working_details.columns.max(1)
    }

    fn get_col_width(&self, column: u16) -> usize {
        self.working_details
            .col_widths
            .get(column as usize)
            .copied()
            .unwrap_or(1)
    }

    fn get_rows(&self) -> u16 {
        let values = self.values.len() as u16;
        if values == 0 {
            return 1;
        }
        let cols = self.get_cols();
        let rows = values / cols;
        if values % cols != 0 { rows + 1 } else { rows }
    }

    fn compute_column_layout(&self, available_width: usize) -> (u16, Vec<usize>) {
        let total = self.values.len();
        if total == 0 {
            return (1, vec![available_width.max(1)]);
        }
        let max_cols = self.default_details.columns.max(1) as usize;
        let max_cols = max_cols.min(total);
        let pad = self.default_details.col_padding;

        for cols in (1..=max_cols).rev() {
            let mut widths = vec![0usize; cols];
            for (idx, suggestion) in self.values.iter().enumerate() {
                let col = idx % cols;
                let w = display_text(suggestion).width();
                if w > widths[col] {
                    widths[col] = w;
                }
            }
            let total_width = widths.iter().sum::<usize>() + pad.saturating_mul(cols.saturating_sub(1));
            if total_width <= available_width || cols == 1 {
                let mut col_widths = Vec::with_capacity(cols);
                for (i, w) in widths.into_iter().enumerate() {
                    let mut width = w;
                    if i + 1 < cols {
                        width = width.saturating_add(pad);
                    }
                    if width == 0 {
                        width = 1;
                    }
                    col_widths.push(width);
                }
                return (cols as u16, col_widths);
            }
        }

        (1, vec![available_width.max(1)])
    }

    fn end_of_line(&self, column: u16, index: usize, last_index: usize) -> &str {
        if column == self.get_cols().saturating_sub(1) || index == last_index {
            "\r\n"
        } else {
            ""
        }
    }

    fn move_next(&mut self) {
        let mut new_col = self.col_pos + 1;
        let mut new_row = self.row_pos;

        if new_col >= self.get_cols() {
            new_row += 1;
            new_col = 0;
        }

        if new_row >= self.get_rows() {
            new_row = 0;
            new_col = 0;
        }

        let position = new_row * self.get_cols() + new_col;
        if position >= self.values.len() as u16 {
            self.reset_position();
        } else {
            self.col_pos = new_col;
            self.row_pos = new_row;
        }
    }

    fn move_previous(&mut self) {
        let new_col = self.col_pos.checked_sub(1);

        let (new_col, new_row) = match new_col {
            Some(col) => (col, self.row_pos),
            None => match self.row_pos.checked_sub(1) {
                Some(row) => (self.get_cols().saturating_sub(1), row),
                None => (
                    self.get_cols().saturating_sub(1),
                    self.get_rows().saturating_sub(1),
                ),
            },
        };

        let position = new_row * self.get_cols() + new_col;
        if position >= self.values.len() as u16 {
            self.col_pos = (self.values.len() as u16 % self.get_cols()).saturating_sub(1);
            self.row_pos = self.get_rows().saturating_sub(1);
        } else {
            self.col_pos = new_col;
            self.row_pos = new_row;
        }
    }

    fn move_up(&mut self) {
        self.row_pos = if let Some(new_row) = self.row_pos.checked_sub(1) {
            new_row
        } else {
            let new_row = self.get_rows().saturating_sub(1);
            let index = new_row * self.get_cols() + self.col_pos;
            if index >= self.values.len() as u16 {
                new_row.saturating_sub(1)
            } else {
                new_row
            }
        }
    }

    fn move_down(&mut self) {
        let new_row = self.row_pos + 1;
        self.row_pos = if new_row >= self.get_rows() {
            0
        } else {
            let index = new_row * self.get_cols() + self.col_pos;
            if index >= self.values.len() as u16 {
                0
            } else {
                new_row
            }
        }
    }

    fn move_left(&mut self) {
        self.col_pos = if let Some(col) = self.col_pos.checked_sub(1) {
            col
        } else if self.index() + 1 == self.values.len() {
            0
        } else {
            self.get_cols().saturating_sub(1)
        }
    }

    fn move_right(&mut self) {
        let new_col = self.col_pos + 1;
        self.col_pos = if new_col >= self.get_cols() || self.index() + 2 > self.values.len() {
            0
        } else {
            new_col
        };
    }

    fn get_value(&self) -> Option<Suggestion> {
        self.values.get(self.index()).cloned()
    }

    fn description_line(&self) -> Option<String> {
        let description = self
            .get_value()
            .and_then(|suggestion| suggestion.description)
            .unwrap_or_default();
        if description.trim().is_empty() {
            return None;
        }
        Some(
            description
                .lines()
                .next()
                .unwrap_or_default()
                .replace('\n', " "),
        )
    }

    fn create_entry_string(
        &self,
        suggestion: &Suggestion,
        index: usize,
        column: u16,
        empty_space: usize,
        last_index: usize,
        use_ansi_coloring: bool,
    ) -> String {
        let display = display_text(suggestion);
        let styled = if index == self.index() {
            self.colors.selected_text_style.prefix()
        } else {
            self.colors.text_style.prefix()
        };

        if use_ansi_coloring {
            format!(
                "{}{}{:>empty$}{}{}",
                styled,
                display,
                "",
                RESET,
                self.end_of_line(column, index, last_index),
                empty = empty_space,
            )
        } else {
            let line = format!(
                "{}{:>empty$}{}",
                display,
                "",
                self.end_of_line(column, index, last_index),
                empty = empty_space,
            );
            if index == self.index() {
                line.to_uppercase()
            } else {
                line
            }
        }
    }

    fn create_description_string(
        &self,
        description: &str,
        width: usize,
        use_ansi_coloring: bool,
    ) -> String {
        let truncated = truncate_to_width(description, width);
        let padding = width.saturating_sub(truncated.width());
        if use_ansi_coloring {
            format!(
                "{}{}{:>pad$}{}",
                self.colors.description_style.prefix(),
                truncated,
                "",
                RESET,
                pad = padding,
            )
        } else {
            format!("{truncated:>pad$}", pad = padding)
        }
    }
}

impl Menu for OspCompletionMenu {
    fn name(&self) -> &str {
        &self.name
    }

    fn indicator(&self) -> &str {
        if self.values.is_empty() {
            ""
        } else {
            &self.marker
        }
    }

    fn is_active(&self) -> bool {
        self.active
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

        if can_partially_complete(&self.values, editor) {
            self.update_values(editor, completer);
            true
        } else {
            false
        }
    }

    fn menu_event(&mut self, event: MenuEvent) {
        match &event {
            MenuEvent::Activate(_) => {
                self.active = true;
                self.values.clear();
            }
            MenuEvent::Deactivate => {
                self.active = false;
                self.input = None;
            }
            _ => {}
        }
        self.event = Some(event);
    }

    fn update_values(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        let (input, pos) = completer_input(
            editor.get_buffer(),
            editor.line_buffer().insertion_point(),
            self.input.as_deref(),
            self.only_buffer_difference,
        );
        self.values = completer.complete(&input, pos);
        if self.values.is_empty() {
            self.active = false;
            self.input = None;
        }
        self.reset_position();
    }

    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        if let Some(event) = self.event.take() {
            match event {
                MenuEvent::Activate(_) => {
                    self.active = true;
                    self.input = Some(editor.get_buffer().to_string());
                    self.update_values(editor, completer);
                }
                MenuEvent::Deactivate => self.active = false,
                MenuEvent::Edit(_) => {
                    self.reset_position();
                    self.update_values(editor, completer);
                }
                MenuEvent::NextElement => self.move_next(),
                MenuEvent::PreviousElement => self.move_previous(),
                MenuEvent::MoveUp => self.move_up(),
                MenuEvent::MoveDown => self.move_down(),
                MenuEvent::MoveLeft => self.move_left(),
                MenuEvent::MoveRight => self.move_right(),
                MenuEvent::NextPage | MenuEvent::PreviousPage => {}
            }
        }

        self.input_indent = self.compute_input_indent(editor, painter.screen_width());
        let available_width = painter.screen_width().saturating_sub(self.input_indent).max(1);
        let (cols, col_widths) = self.compute_column_layout(available_width as usize);
        self.working_details.columns = cols.max(1);
        self.working_details.col_widths = col_widths;
    }

    fn replace_in_buffer(&self, editor: &mut Editor) {
        if let Some(suggestion) = self.get_value() {
            replace_in_buffer(Some(suggestion), editor);
        }
    }

    fn menu_required_lines(&self, _terminal_columns: u16) -> u16 {
        let rows = self.get_rows().min(self.default_details.max_rows).max(1);
        let desc_line = self.description_line().map(|_| 1).unwrap_or(0);
        rows + desc_line
    }

    fn menu_string(&self, available_lines: u16, use_ansi_coloring: bool) -> String {
        if self.values.is_empty() {
            return String::new();
        }

        let desc_line = self.description_line();
        let desc_rows = if desc_line.is_some() {
            self.default_details.description_rows.max(1)
        } else {
            0
        };
        let available_rows = available_lines.saturating_sub(desc_rows as u16).max(1);
        let total_rows = self.get_rows();
        let visible_rows = available_rows
            .min(total_rows)
            .min(self.default_details.max_rows);

        let skip_values = if self.row_pos >= visible_rows {
            let skip_lines = self.row_pos.saturating_sub(visible_rows) + 1;
            (skip_lines * self.get_cols()) as usize
        } else {
            0
        };
        let available_values = (visible_rows * self.get_cols()) as usize;
        let last_index = self.values.len().saturating_sub(1).min(
            skip_values
                .saturating_add(available_values)
                .saturating_sub(1),
        );

        let selection_values: String = self
            .values
            .iter()
            .skip(skip_values)
            .take(available_values)
            .enumerate()
            .map(|(index, suggestion)| {
                let index = index + skip_values;
                let column = index as u16 % self.get_cols();
                let display_width = display_text(suggestion).width();
                let empty_space = self.get_col_width(column).saturating_sub(display_width);
                self.create_entry_string(
                    suggestion,
                    index,
                    column,
                    empty_space,
                    last_index,
                    use_ansi_coloring,
                )
            })
            .collect();

        let output = if let Some(description) = desc_line {
            let menu_width = self.working_details.col_widths.iter().sum::<usize>();
            let description_line =
                self.create_description_string(&description, menu_width.max(1), use_ansi_coloring);
            format!("{selection_values}{description_line}")
        } else {
            selection_values
        };

        indent_lines(&output, self.input_indent as usize)
    }

    fn min_rows(&self) -> u16 {
        1
    }

    fn get_values(&self) -> &[Suggestion] {
        &self.values
    }

    fn set_cursor_pos(&mut self, pos: (u16, u16)) {
        self.cursor_col = pos.0;
    }
}

fn display_text(suggestion: &Suggestion) -> &str {
    suggestion
        .extra
        .as_ref()
        .and_then(|extra| extra.first())
        .map(String::as_str)
        .unwrap_or(&suggestion.value)
}

fn truncate_to_width(input: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in input.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

fn indent_lines(text: &str, indent: usize) -> String {
    if indent == 0 || text.is_empty() {
        return text.to_string();
    }
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for chunk in text.split_inclusive("\r\n") {
        if let Some(body) = chunk.strip_suffix("\r\n") {
            out.push_str(&pad);
            out.push_str(body);
            out.push_str("\r\n");
        } else {
            out.push_str(&pad);
            out.push_str(chunk);
        }
    }
    out
}

impl OspCompletionMenu {
    fn compute_input_indent(&self, editor: &Editor, screen_width: u16) -> u16 {
        let buffer = editor.get_buffer();
        let cursor = editor.line_buffer().insertion_point();
        let before = buffer.get(..cursor).unwrap_or(buffer);
        let before_width = before.width() as u16;
        let indent = self.cursor_col.saturating_sub(before_width);
        indent.min(screen_width.saturating_sub(1))
    }
}
