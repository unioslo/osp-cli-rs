use super::super::menu::OspCompletionMenu;
use super::adapter::{color_from_style_spec, style_with_fg_bg};
use super::config::ReplAppearance;
use super::{COMPLETION_MENU_NAME, HISTORY_MENU_NAME, SharedHistory};
use anyhow::Result;
use nu_ansi_term::Color;
use skim::options::MatchScheme;
use skim::prelude::{
    Skim, SkimItem, SkimItemReceiver, SkimItemSender, SkimOptionsBuilder,
    unbounded as skim_unbounded,
};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct HistoryPickerItem {
    pub(crate) label: String,
    pub(crate) command: String,
    pub(crate) matching_range: [(usize, usize); 1],
}

impl SkimItem for HistoryPickerItem {
    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.label)
    }

    fn output(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.command)
    }

    fn get_matching_ranges(&self) -> Option<&[(usize, usize)]> {
        Some(&self.matching_range)
    }
}

pub(crate) fn launch_history_picker(
    history: &SharedHistory,
    appearance: &ReplAppearance,
    current_line: &str,
) -> Result<Option<String>> {
    let items = history_picker_items(history);
    if items.is_empty() {
        return Ok(None);
    }

    let options = build_history_picker_options(appearance, current_line.trim())?;
    let (tx, rx): (SkimItemSender, SkimItemReceiver) = skim_unbounded();
    let payload = items
        .into_iter()
        .map(|item| Arc::new(item) as Arc<dyn SkimItem>)
        .collect::<Vec<_>>();
    let _ = tx.send(payload);
    drop(tx);

    let output =
        Skim::run_with(options, Some(rx)).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    if output.is_abort {
        return Ok(None);
    }

    // Prefer the explicit selection, but fall back to skim's current row so a
    // plain Enter accepts the active history item even if nothing was toggled.
    Ok(output
        .selected_items
        .first()
        .map(|item| item.output().into_owned())
        .or_else(|| output.current.map(|item| item.output().into_owned())))
}

pub(crate) fn history_picker_items(history: &SharedHistory) -> Vec<HistoryPickerItem> {
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();

    for entry in history.list_entries().into_iter().rev() {
        // Keep the newest copy of each unique command so an empty query acts
        // like "recent history", not "all repeats of the last thing I ran".
        if seen.insert(entry.command.clone()) {
            entries.push(entry);
        }
    }

    let number_width = entries
        .iter()
        .map(|entry| entry.id.to_string().len())
        .max()
        .unwrap_or(1);

    entries
        .into_iter()
        .map(|entry| {
            let display_command = single_line_history_label(&entry.command);
            let label = format!("{:>number_width$}  {}", entry.id, display_command);
            let command_start = number_width + 2;
            HistoryPickerItem {
                label,
                command: entry.command,
                // Match/highlight only the command portion. The history number
                // is display chrome and should not influence skim ranking.
                matching_range: [(command_start, command_start + display_command.len())],
            }
        })
        .collect()
}

fn single_line_history_label(command: &str) -> String {
    command.replace("\r\n", " \\n ").replace('\n', " \\n ")
}

pub(crate) fn build_history_picker_options(
    appearance: &ReplAppearance,
    initial_query: &str,
) -> Result<skim::SkimOptions> {
    let height = appearance
        .history_menu_rows
        .max(1)
        .saturating_add(1)
        .to_string();
    let mut builder = SkimOptionsBuilder::default();
    builder
        .height(height)
        .min_height("2")
        .reverse(true)
        .no_info(true)
        .multi(false)
        .no_mouse(true)
        .prompt("(reverse-i-search)> ")
        .query(initial_query)
        .scheme(MatchScheme::History)
        // `Ctrl-R` inside the picker should stay within skim's history mode,
        // not recursively re-enter the REPL host command path.
        .bind(vec!["ctrl-r:toggle-sort".to_string()]);

    if let Some(color) = build_history_picker_color(appearance) {
        builder.color(color);
    }

    builder
        .build()
        .map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn build_history_picker_color(appearance: &ReplAppearance) -> Option<String> {
    let text = appearance
        .completion_text_style
        .as_deref()
        .and_then(color_from_style_spec)
        .and_then(skim_color_value);
    let background = appearance
        .completion_background_style
        .as_deref()
        .and_then(color_from_style_spec)
        .and_then(skim_color_value);
    let highlight = appearance
        .completion_highlight_style
        .as_deref()
        .and_then(color_from_style_spec)
        .and_then(skim_color_value);

    let mut parts = Vec::new();
    if let Some(text) = text.as_deref() {
        // skim has separate color channels for ordinary rows, the active row,
        // prompt/query chrome, and match highlights. Set them all explicitly so
        // the picker does not inherit unreadable defaults from skim itself.
        parts.push(format!("normal:{text}"));
        parts.push(format!("matched:{text}"));
        parts.push(format!("current:{text}"));
        parts.push(format!("current_match:{text}"));
        parts.push(format!("query:{text}"));
        parts.push(format!("prompt:{text}"));
        parts.push(format!("cursor:{text}"));
        parts.push(format!("selected:{text}"));
        parts.push(format!("info:{text}"));
        parts.push(format!("header:{text}"));
        parts.push(format!("spinner:{text}"));
        parts.push(format!("border:{text}"));
    }
    if let Some(background) = background.as_deref() {
        parts.push(format!("bg:{background}"));
        parts.push(format!("matched_bg:{background}"));
    }
    if let Some(highlight) = highlight.as_deref() {
        parts.push(format!("current_bg:{highlight}"));
        parts.push(format!("current_match_bg:{highlight}"));
    }

    (parts.len() > 1).then(|| parts.join(","))
}

fn skim_color_value(color: Color) -> Option<String> {
    match color {
        Color::Black => Some("0".to_string()),
        Color::DarkGray => Some("8".to_string()),
        Color::Red => Some("1".to_string()),
        Color::LightRed => Some("9".to_string()),
        Color::Green => Some("2".to_string()),
        Color::LightGreen => Some("10".to_string()),
        Color::Yellow => Some("3".to_string()),
        Color::LightYellow => Some("11".to_string()),
        Color::Blue => Some("4".to_string()),
        Color::LightBlue => Some("12".to_string()),
        Color::Purple | Color::Magenta => Some("5".to_string()),
        Color::LightPurple | Color::LightMagenta => Some("13".to_string()),
        Color::Cyan => Some("6".to_string()),
        Color::LightCyan => Some("14".to_string()),
        Color::White => Some("7".to_string()),
        Color::LightGray => Some("15".to_string()),
        Color::Fixed(value) => Some(value.to_string()),
        Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Default => None,
    }
}

pub(crate) fn build_completion_menu(appearance: &ReplAppearance) -> OspCompletionMenu {
    build_candidate_menu(appearance, COMPLETION_MENU_NAME)
        .with_only_buffer_difference(false)
        .with_quick_complete(true)
        .with_columns(u16::MAX)
        .with_max_rows(u16::MAX)
}

pub(crate) fn build_history_menu(appearance: &ReplAppearance) -> OspCompletionMenu {
    build_candidate_menu(appearance, HISTORY_MENU_NAME)
        .with_only_buffer_difference(false)
        .with_quick_complete(false)
        .with_columns(1)
        .with_max_rows(appearance.history_menu_rows.max(1))
}

fn build_candidate_menu(appearance: &ReplAppearance, name: &str) -> OspCompletionMenu {
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
        .with_name(name)
        .with_marker("")
        .with_description_rows(1)
        .with_column_padding(2)
        .with_text_style(style_with_fg_bg(text_color, background_color))
        .with_description_text_style(style_with_fg_bg(text_color, highlight_color))
        .with_match_text_style(style_with_fg_bg(text_color, background_color))
        .with_selected_text_style(style_with_fg_bg(text_color, highlight_color))
        .with_selected_match_text_style(style_with_fg_bg(text_color, highlight_color))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repl::engine::HistoryConfig;
    use reedline::Menu;

    fn empty_history() -> SharedHistory {
        SharedHistory::new(
            HistoryConfig::builder()
                .with_enabled(false)
                .with_max_entries(0)
                .build(),
        )
    }

    #[test]
    fn launch_history_picker_skips_empty_history_unit() {
        let result = launch_history_picker(&empty_history(), &ReplAppearance::default(), "doctor")
            .expect("empty history should not launch skim");
        assert!(result.is_none());
    }

    #[test]
    fn overlay_color_and_menu_helpers_cover_completion_and_history_paths_unit() {
        let appearance = ReplAppearance::builder()
            .with_completion_text_style(Some("white".to_string()))
            .with_completion_background_style(Some("black".to_string()))
            .with_completion_highlight_style(Some("cyan".to_string()))
            .with_history_menu_rows(7)
            .build();

        let completion_menu = build_completion_menu(&appearance);
        assert_eq!(completion_menu.name(), COMPLETION_MENU_NAME);
        assert!(completion_menu.can_quick_complete());

        let history_menu = build_history_menu(&appearance);
        assert_eq!(history_menu.name(), HISTORY_MENU_NAME);
        assert!(!history_menu.can_quick_complete());

        let options = build_history_picker_options(&appearance, "needle")
            .expect("history picker options should build");
        assert_eq!(options.height, "8");
        assert_eq!(options.query.as_deref(), Some("needle"));

        let cases = [
            (Color::Black, Some("0")),
            (Color::DarkGray, Some("8")),
            (Color::Red, Some("1")),
            (Color::LightRed, Some("9")),
            (Color::Green, Some("2")),
            (Color::LightGreen, Some("10")),
            (Color::Yellow, Some("3")),
            (Color::LightYellow, Some("11")),
            (Color::Blue, Some("4")),
            (Color::LightBlue, Some("12")),
            (Color::Purple, Some("5")),
            (Color::Magenta, Some("5")),
            (Color::LightPurple, Some("13")),
            (Color::LightMagenta, Some("13")),
            (Color::Cyan, Some("6")),
            (Color::LightCyan, Some("14")),
            (Color::White, Some("7")),
            (Color::LightGray, Some("15")),
            (Color::Fixed(141), Some("141")),
            (Color::Rgb(1, 2, 3), Some("#010203")),
            (Color::Default, None),
        ];

        for (input, expected) in cases {
            assert_eq!(skim_color_value(input).as_deref(), expected);
        }

        let plain_options = build_history_picker_options(&ReplAppearance::default(), "needle")
            .expect("history picker options should build");
        assert!(plain_options.color.is_none());
    }
}
