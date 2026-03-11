use nu_ansi_term::ansi::RESET;
use nu_ansi_term::{Color, Style};
use reedline::{MenuEvent, MenuTextStyle, Suggestion};
use serde::Serialize;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuAction {
    None,
    UpdateValues,
    ApplySelection,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MenuCore {
    active: bool,
    default_details: DefaultGridDetails,
    working_details: WorkingGridDetails,
    values: Vec<Suggestion>,
    col_pos: u16,
    row_pos: u16,
    input_indent: u16,
    just_activated: bool,
    activated_while_active: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct StyleDebug {
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub bold: bool,
    pub dimmed: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub reverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MenuStyleDebug {
    pub text: StyleDebug,
    pub selected_text: StyleDebug,
    pub description: StyleDebug,
    pub match_text: StyleDebug,
    pub selected_match: StyleDebug,
}

pub(crate) struct MenuDebug {
    pub(crate) columns: u16,
    pub(crate) rows: u16,
    pub(crate) visible_rows: u16,
    pub(crate) indent: u16,
    pub(crate) selected_index: i64,
    pub(crate) selected_row: u16,
    pub(crate) selected_col: u16,
    pub(crate) description: Option<String>,
    pub(crate) description_rendered: Option<String>,
    pub(crate) styles: MenuStyleDebug,
    pub(crate) rendered: Vec<String>,
}

impl MenuCore {
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn just_activated(&self) -> bool {
        self.just_activated
    }

    pub(crate) fn values(&self) -> &[Suggestion] {
        &self.values
    }

    pub(crate) fn selected_value(&self) -> Option<&Suggestion> {
        self.values.get(self.index())
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        (!self.values.is_empty()).then_some(self.index())
    }

    pub(crate) fn selected_row(&self) -> u16 {
        self.row_pos
    }

    pub(crate) fn selected_col(&self) -> u16 {
        self.col_pos
    }

    pub(crate) fn rows(&self) -> u16 {
        self.get_rows()
    }

    pub(crate) fn columns(&self) -> u16 {
        self.working_details.columns.max(1)
    }

    pub(crate) fn input_indent(&self) -> u16 {
        self.input_indent
    }

    pub(crate) fn visible_rows_for(&self, available_lines: u16) -> u16 {
        let (_, desc_rows) = self.description_plan(available_lines);
        let available_rows = available_lines.saturating_sub(desc_rows).max(1);
        let total_rows = self.get_rows();
        available_rows
            .min(total_rows)
            .min(self.default_details.max_rows)
    }

    pub(crate) fn pre_event(&mut self, event: &MenuEvent) {
        match event {
            MenuEvent::Activate(_) => {
                self.activated_while_active = self.active;
                self.active = true;
                if !self.activated_while_active {
                    self.values.clear();
                }
            }
            MenuEvent::Deactivate => {
                self.active = false;
                self.values.clear();
                self.just_activated = false;
                self.activated_while_active = false;
            }
            _ => {}
        }
    }

    pub(crate) fn handle_event(&mut self, event: MenuEvent) -> MenuAction {
        match event {
            MenuEvent::Activate(_) => {
                if self.activated_while_active {
                    if self.just_activated {
                        self.just_activated = false;
                        MenuAction::ApplySelection
                    } else {
                        self.move_next();
                        MenuAction::ApplySelection
                    }
                } else {
                    self.just_activated = true;
                    MenuAction::UpdateValues
                }
            }
            MenuEvent::Deactivate => {
                self.just_activated = false;
                MenuAction::None
            }
            MenuEvent::Edit(_) => {
                self.reset_position();
                self.just_activated = true;
                MenuAction::UpdateValues
            }
            MenuEvent::NextElement => {
                if self.just_activated {
                    self.just_activated = false;
                    MenuAction::ApplySelection
                } else {
                    self.move_next();
                    MenuAction::ApplySelection
                }
            }
            MenuEvent::PreviousElement => {
                self.just_activated = false;
                self.move_previous();
                MenuAction::ApplySelection
            }
            MenuEvent::MoveUp => {
                self.just_activated = false;
                self.move_up();
                MenuAction::ApplySelection
            }
            MenuEvent::MoveDown => {
                self.just_activated = false;
                self.move_down();
                MenuAction::ApplySelection
            }
            MenuEvent::MoveLeft => {
                self.just_activated = false;
                self.move_left();
                MenuAction::ApplySelection
            }
            MenuEvent::MoveRight => {
                self.just_activated = false;
                self.move_right();
                MenuAction::ApplySelection
            }
            MenuEvent::NextPage => {
                self.just_activated = false;
                self.move_page_down();
                MenuAction::ApplySelection
            }
            MenuEvent::PreviousPage => {
                self.just_activated = false;
                self.move_page_up();
                MenuAction::ApplySelection
            }
        }
    }

    pub(crate) fn set_values(&mut self, values: Vec<Suggestion>) {
        self.values = values;
        if self.values.is_empty() {
            self.active = false;
            self.just_activated = false;
        }
        self.reset_position();
    }

    pub(crate) fn restore_selection_by_value(&mut self, value: &str) {
        if let Some(index) = self.values.iter().position(|item| item.value == value) {
            let cols = self.get_cols().max(1);
            self.row_pos = index as u16 / cols;
            self.col_pos = index as u16 % cols;
        }
    }

    pub(crate) fn set_columns(&mut self, columns: u16) {
        self.default_details.columns = columns.max(1);
    }

    pub(crate) fn set_column_padding(&mut self, col_padding: usize) {
        self.default_details.col_padding = col_padding;
    }

    pub(crate) fn set_max_rows(&mut self, max_rows: u16) {
        self.default_details.max_rows = max_rows.max(1);
    }

    pub(crate) fn set_description_rows(&mut self, description_rows: usize) {
        self.default_details.description_rows = description_rows;
    }

    pub(crate) fn update_layout(&mut self, screen_width: u16, input_indent: u16) {
        let max_indent = screen_width.saturating_sub(1);
        self.input_indent = input_indent.min(max_indent);
        let available_width = screen_width.saturating_sub(self.input_indent).max(1) as usize;
        let marker_width = marker_width_for_layout(available_width);
        let (cols, col_widths) = self.compute_column_layout(available_width, marker_width);
        self.working_details.columns = cols.max(1);
        self.working_details.col_widths = col_widths;
    }

    pub(crate) fn menu_required_lines(&self) -> u16 {
        let rows = self.get_rows().min(self.default_details.max_rows).max(1);
        let desc_line = self.description_line().map(|_| 1).unwrap_or(0);
        rows + desc_line
    }

    pub(crate) fn menu_string(
        &self,
        available_lines: u16,
        use_ansi_coloring: bool,
        colors: &MenuTextStyle,
    ) -> String {
        if self.values.is_empty() {
            return String::new();
        }

        let (desc_line, desc_rows) = self.description_plan(available_lines);
        let available_rows = available_lines.saturating_sub(desc_rows).max(1);
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
                self.create_entry_string(
                    suggestion,
                    index,
                    column,
                    last_index,
                    use_ansi_coloring,
                    colors,
                )
            })
            .collect();

        let output = if let Some(description) = desc_line {
            let menu_width = self.working_details.col_widths.iter().sum::<usize>();
            let description_line = self.create_description_string(
                &description,
                menu_width.max(1),
                use_ansi_coloring,
                colors,
            );
            format!("{selection_values}{description_line}")
        } else {
            selection_values
        };

        indent_lines(&output, self.input_indent as usize)
    }

    pub(crate) fn debug_snapshot(
        &mut self,
        colors: &MenuTextStyle,
        screen_width: u16,
        screen_height: u16,
        input_indent: u16,
        ansi: bool,
    ) -> MenuDebug {
        let styles = menu_styles_debug(colors);
        self.update_layout(screen_width, input_indent);

        let (desc_line, desc_rows) = self.description_plan(screen_height);
        let available_rows = screen_height.saturating_sub(desc_rows).max(1);
        let total_rows = self.get_rows();
        let visible_rows = available_rows
            .min(total_rows)
            .min(self.default_details.max_rows);

        let description_rendered = if let Some(description) = desc_line.as_deref() {
            let menu_width = self.working_details.col_widths.iter().sum::<usize>();
            Some(self.create_description_string(description, menu_width.max(1), ansi, colors))
        } else {
            None
        };

        let rendered = if self.active && !self.values.is_empty() {
            self.menu_string(screen_height, ansi, colors)
                .split_terminator("\r\n")
                .map(|line| line.to_string())
                .collect()
        } else {
            Vec::new()
        };

        let rows = self.get_rows();
        let selected_index = self.selected_index().map(|idx| idx as i64).unwrap_or(-1);

        MenuDebug {
            columns: self.working_details.columns.max(1),
            rows,
            visible_rows,
            indent: self.input_indent,
            selected_index,
            selected_row: self.row_pos,
            selected_col: self.col_pos,
            description: desc_line,
            description_rendered,
            styles,
            rendered,
        }
    }

    #[cfg(test)]
    pub(crate) fn columns_for_test(&self) -> u16 {
        self.working_details.columns.max(1)
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
        if !values.is_multiple_of(cols) {
            rows + 1
        } else {
            rows
        }
    }

    fn compute_column_layout(
        &self,
        available_width: usize,
        marker_width: usize,
    ) -> (u16, Vec<usize>) {
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
                let w = display_text(suggestion)
                    .width()
                    .saturating_add(marker_width);
                if w > widths[col] {
                    widths[col] = w;
                }
            }
            let total_width =
                widths.iter().sum::<usize>() + pad.saturating_mul(cols.saturating_sub(1));
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

    fn move_next(&mut self) {
        let cols = self.get_cols().max(1);
        let rows = self.get_rows().max(1);
        let total = self.values.len() as u16;
        if total == 0 {
            return;
        }

        let mut new_col = self.col_pos + 1;
        let mut new_row = self.row_pos;
        if new_col >= cols {
            new_col = 0;
            new_row = new_row.saturating_add(1);
            if new_row >= rows {
                new_row = 0;
            }
        }

        let position = new_row * cols + new_col;
        if position >= total {
            self.col_pos = 0;
            self.row_pos = 0;
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
        } else {
            self.rightmost_valid_col_for_row(self.row_pos)
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

    fn move_page_down(&mut self) {
        self.move_rows(self.default_details.max_rows.max(1) as i32);
    }

    fn move_page_up(&mut self) {
        self.move_rows(-(self.default_details.max_rows.max(1) as i32));
    }

    fn move_rows(&mut self, delta: i32) {
        let rows = self.get_rows();
        if rows == 0 || self.values.is_empty() {
            return;
        }

        let rows_i32 = rows as i32;
        let mut new_row = self.row_pos as i32 + delta;
        new_row = ((new_row % rows_i32) + rows_i32) % rows_i32;
        self.row_pos = new_row as u16;

        let max_col = self.rightmost_valid_col_for_row(self.row_pos);
        if self.col_pos > max_col {
            self.col_pos = max_col;
        }
    }

    fn description_line(&self) -> Option<String> {
        if self.just_activated {
            return None;
        }
        let description = self
            .selected_value()
            .and_then(|suggestion| suggestion.description.clone())
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

    fn description_plan(&self, available_lines: u16) -> (Option<String>, u16) {
        let mut desc_line = self.description_line();
        let desc_rows = if desc_line.is_some() {
            self.default_details.description_rows.max(1)
        } else {
            0
        };
        let use_description = desc_rows > 0 && available_lines > desc_rows as u16;
        if !use_description {
            desc_line = None;
        }
        let desc_rows = if use_description { desc_rows } else { 0 };
        (desc_line, desc_rows as u16)
    }

    fn create_entry_string(
        &self,
        suggestion: &Suggestion,
        index: usize,
        column: u16,
        last_index: usize,
        use_ansi_coloring: bool,
        colors: &MenuTextStyle,
    ) -> String {
        let display = display_text(suggestion);
        let col_width = self.get_col_width(column);
        let display_width = display.width();
        let selected = index == self.index() && !self.just_activated;
        let styled = if selected {
            colors.selected_text_style.prefix()
        } else {
            colors.text_style.prefix()
        };

        if use_ansi_coloring {
            let empty_space = col_width.saturating_sub(display_width);
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
            let (marker, marker_width) = marker_prefix(selected, col_width);
            let max_text_width = col_width.saturating_sub(marker_width);
            let display = truncate_to_width(display, max_text_width);
            let display_width = display.width();
            let empty_space = col_width
                .saturating_sub(marker_width)
                .saturating_sub(display_width);
            format!(
                "{}{}{:>empty$}{}",
                marker,
                display,
                "",
                self.end_of_line(column, index, last_index),
                empty = empty_space,
            )
        }
    }

    fn rightmost_valid_col_for_row(&self, row: u16) -> u16 {
        let cols = self.get_cols().max(1);
        let row_start = row.saturating_mul(cols) as usize;
        let row_end = row_start
            .saturating_add(cols as usize)
            .min(self.values.len());
        let row_len = row_end.saturating_sub(row_start);
        if row_len == 0 {
            0
        } else {
            row_len.saturating_sub(1) as u16
        }
    }

    fn create_description_string(
        &self,
        description: &str,
        width: usize,
        use_ansi_coloring: bool,
        colors: &MenuTextStyle,
    ) -> String {
        let truncated = truncate_to_width(description, width);
        let padding = width.saturating_sub(truncated.width());
        if use_ansi_coloring {
            format!(
                "{}{}{:>pad$}{}",
                colors.description_style.prefix(),
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

pub(crate) fn display_text(suggestion: &Suggestion) -> &str {
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

fn marker_prefix(selected: bool, col_width: usize) -> (&'static str, usize) {
    if col_width == 0 {
        return ("", 0);
    }
    if col_width == 1 {
        return if selected { (">", 1) } else { (" ", 1) };
    }
    if selected { ("> ", 2) } else { ("  ", 2) }
}

fn marker_width_for_layout(available_width: usize) -> usize {
    if available_width <= 1 { 1 } else { 2 }
}

fn color_to_string(color: Color) -> String {
    match color {
        Color::Black => "black".to_string(),
        Color::DarkGray => "dark_gray".to_string(),
        Color::Red => "red".to_string(),
        Color::LightRed => "light_red".to_string(),
        Color::Green => "green".to_string(),
        Color::LightGreen => "light_green".to_string(),
        Color::Yellow => "yellow".to_string(),
        Color::LightYellow => "light_yellow".to_string(),
        Color::Blue => "blue".to_string(),
        Color::LightBlue => "light_blue".to_string(),
        Color::Purple => "purple".to_string(),
        Color::LightPurple => "light_purple".to_string(),
        Color::Magenta => "magenta".to_string(),
        Color::LightMagenta => "light_magenta".to_string(),
        Color::Cyan => "cyan".to_string(),
        Color::LightCyan => "light_cyan".to_string(),
        Color::White => "white".to_string(),
        Color::LightGray => "light_gray".to_string(),
        Color::Fixed(value) => format!("fixed:{value}"),
        Color::Rgb(r, g, b) => format!("rgb:{r},{g},{b}"),
        Color::Default => "default".to_string(),
    }
}

fn style_to_debug(style: Style) -> StyleDebug {
    StyleDebug {
        foreground: style.foreground.map(color_to_string),
        background: style.background.map(color_to_string),
        bold: style.is_bold,
        dimmed: style.is_dimmed,
        italic: style.is_italic,
        underline: style.is_underline,
        blink: style.is_blink,
        reverse: style.is_reverse,
        hidden: style.is_hidden,
        strikethrough: style.is_strikethrough,
        reset: style.prefix_with_reset,
    }
}

fn menu_styles_debug(colors: &MenuTextStyle) -> MenuStyleDebug {
    MenuStyleDebug {
        text: style_to_debug(colors.text_style),
        selected_text: style_to_debug(colors.selected_text_style),
        description: style_to_debug(colors.description_style),
        match_text: style_to_debug(colors.match_style),
        selected_match: style_to_debug(colors.selected_match_style),
    }
}

#[cfg(test)]
mod tests;
