use super::{
    MenuAction, MenuCore, color_to_string, display_text, indent_lines, marker_prefix,
    marker_width_for_layout, style_to_debug, truncate_to_width,
};
use nu_ansi_term::{Color, Style};
use reedline::{MenuEvent, MenuTextStyle, Span, Suggestion};

fn suggestion(value: &str) -> Suggestion {
    Suggestion {
        value: value.to_string(),
        span: Span { start: 0, end: 0 },
        ..Suggestion::default()
    }
}

fn described_suggestion(value: &str, description: &str) -> Suggestion {
    let mut suggestion = suggestion(value);
    suggestion.description = Some(description.to_string());
    suggestion
}

fn numbered_suggestions(count: usize) -> Vec<Suggestion> {
    (0..count)
        .map(|idx| suggestion(&format!("item{idx}")))
        .collect()
}

#[test]
fn event_state_machine_covers_reactivate_edit_and_deactivate() {
    let values = vec![
        suggestion("config"),
        suggestion("doctor"),
        suggestion("history"),
    ];
    let mut core = MenuCore::default();
    core.set_values(values.clone());

    core.pre_event(&MenuEvent::Activate(false));
    assert!(core.is_active());
    assert!(core.values().is_empty());
    assert_eq!(
        core.handle_event(MenuEvent::Activate(false)),
        MenuAction::UpdateValues
    );
    assert!(core.just_activated());

    core.set_values(values.clone());
    assert_eq!(core.selected_index(), Some(0));

    core.pre_event(&MenuEvent::Activate(false));
    assert_eq!(core.values().len(), 3);
    assert_eq!(
        core.handle_event(MenuEvent::Activate(false)),
        MenuAction::ApplySelection
    );
    assert!(!core.just_activated());
    assert_eq!(core.selected_index(), Some(0));

    assert_eq!(
        core.handle_event(MenuEvent::NextElement),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_index(), Some(1));

    core.pre_event(&MenuEvent::Activate(false));
    assert_eq!(
        core.handle_event(MenuEvent::Activate(false)),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_index(), Some(2));

    assert_eq!(
        core.handle_event(MenuEvent::PreviousElement),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_index(), Some(1));

    assert_eq!(
        core.handle_event(MenuEvent::Edit(false)),
        MenuAction::UpdateValues
    );
    assert!(core.just_activated());
    assert_eq!(core.selected_index(), Some(0));

    core.pre_event(&MenuEvent::Deactivate);
    assert!(!core.is_active());
    assert!(core.values().is_empty());
    assert_eq!(core.handle_event(MenuEvent::Deactivate), MenuAction::None);
}

#[test]
fn navigation_wraps_and_clamps_on_sparse_last_row() {
    let mut core = MenuCore::default();
    core.set_columns(4);
    core.set_values(numbered_suggestions(6));
    core.update_layout(80, 0);

    assert_eq!(
        core.handle_event(MenuEvent::PreviousElement),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 1);
    assert_eq!(core.selected_col(), 1);
    assert_eq!(core.selected_index(), Some(5));

    assert_eq!(
        core.handle_event(MenuEvent::MoveRight),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 1);
    assert_eq!(core.selected_col(), 0);
    assert_eq!(core.selected_index(), Some(4));

    assert_eq!(
        core.handle_event(MenuEvent::NextElement),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 1);
    assert_eq!(core.selected_col(), 1);
    assert_eq!(core.selected_index(), Some(5));

    assert_eq!(
        core.handle_event(MenuEvent::NextElement),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 0);
    assert_eq!(core.selected_col(), 0);
    assert_eq!(core.selected_index(), Some(0));

    assert_eq!(
        core.handle_event(MenuEvent::MoveRight),
        MenuAction::ApplySelection
    );
    assert_eq!(
        core.handle_event(MenuEvent::MoveRight),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 0);
    assert_eq!(core.selected_col(), 2);
    assert_eq!(core.selected_index(), Some(2));

    assert_eq!(
        core.handle_event(MenuEvent::MoveDown),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 0);
    assert_eq!(core.selected_col(), 2);
    assert_eq!(core.selected_index(), Some(2));

    assert_eq!(
        core.handle_event(MenuEvent::MoveUp),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 0);
    assert_eq!(core.selected_col(), 2);
    assert_eq!(core.selected_index(), Some(2));

    assert_eq!(
        core.handle_event(MenuEvent::MoveLeft),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_row(), 0);
    assert_eq!(core.selected_col(), 1);
    assert!(core.selected_value().is_some());
}

#[test]
fn debug_snapshot_scrolls_descriptions_and_hides_when_inactive() {
    let mut core = MenuCore::default();
    core.set_columns(1);
    core.set_max_rows(2);
    core.set_description_rows(1);

    core.pre_event(&MenuEvent::Activate(false));
    assert_eq!(
        core.handle_event(MenuEvent::Activate(false)),
        MenuAction::UpdateValues
    );
    core.set_values(vec![
        described_suggestion("alpha", "alpha description"),
        described_suggestion("bravo", "bravo description"),
        described_suggestion("charlie", "charlie description"),
        described_suggestion("delta", "delta description"),
        described_suggestion("echo", "echo description"),
    ]);
    core.update_layout(40, 3);

    assert_eq!(
        core.handle_event(MenuEvent::NextElement),
        MenuAction::ApplySelection
    );
    assert_eq!(
        core.handle_event(MenuEvent::MoveDown),
        MenuAction::ApplySelection
    );
    assert_eq!(
        core.handle_event(MenuEvent::MoveDown),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_index(), Some(2));

    let colors = MenuTextStyle::default();
    let snapshot = core.debug_snapshot(&colors, 40, 3, 3, false);
    assert_eq!(snapshot.visible_rows, 2);
    assert_eq!(snapshot.description.as_deref(), Some("charlie description"));
    assert!(snapshot.description_rendered.is_some());
    assert_eq!(snapshot.rendered.len(), 3);
    assert!(snapshot.rendered.iter().any(|line| line.contains("bravo")));
    assert!(
        snapshot
            .rendered
            .iter()
            .any(|line| line.contains("charlie"))
    );
    assert!(!snapshot.rendered.iter().any(|line| line.contains("alpha")));

    let tight = core.debug_snapshot(&colors, 40, 1, 3, false);
    assert_eq!(tight.visible_rows, 1);
    assert!(tight.description.is_none());
    assert!(tight.description_rendered.is_none());

    core.pre_event(&MenuEvent::Deactivate);
    let inactive = core.debug_snapshot(&colors, 40, 3, 3, false);
    assert!(inactive.rendered.is_empty());
    assert_eq!(core.menu_string(3, false, &colors), "");
}

#[test]
fn helper_paths_cover_layout_render_and_style_utilities() {
    let mut decorated = suggestion("value");
    decorated.extra = Some(vec!["shown".to_string()]);
    assert_eq!(display_text(&decorated), "shown");

    assert_eq!(truncate_to_width("alpha", 0), "");
    assert_eq!(truncate_to_width("a界z", 3), "a界");
    assert_eq!(indent_lines("alpha\r\nbeta", 2), "  alpha\r\n  beta");
    assert_eq!(marker_prefix(false, 0), ("", 0));
    assert_eq!(marker_prefix(true, 1), (">", 1));
    assert_eq!(marker_prefix(true, 4), ("> ", 2));
    assert_eq!(marker_width_for_layout(1), 1);
    assert_eq!(marker_width_for_layout(8), 2);
    assert_eq!(color_to_string(Color::Default), "default");
    assert_eq!(color_to_string(Color::Fixed(7)), "fixed:7");
    assert_eq!(color_to_string(Color::Rgb(1, 2, 3)), "rgb:1,2,3");

    let style = Style::new()
        .fg(Color::LightBlue)
        .on(Color::Black)
        .bold()
        .italic()
        .underline()
        .blink()
        .reverse()
        .hidden()
        .strikethrough();
    let style_debug = style_to_debug(style);
    assert_eq!(style_debug.foreground.as_deref(), Some("light_blue"));
    assert_eq!(style_debug.background.as_deref(), Some("black"));
    assert!(style_debug.bold);
    assert!(style_debug.italic);
    assert!(style_debug.underline);
    assert!(style_debug.blink);
    assert!(style_debug.reverse);
    assert!(style_debug.hidden);
    assert!(style_debug.strikethrough);
}

#[test]
fn layout_and_render_helpers_cover_empty_narrow_and_ansi_paths() {
    let mut core = MenuCore::default();
    core.update_layout(1, 99);
    assert_eq!(core.input_indent(), 0);
    assert_eq!(core.columns_for_test(), 1);
    assert_eq!(core.menu_required_lines(), 1);

    let (cols, widths) = core.compute_column_layout(0, marker_width_for_layout(1));
    assert_eq!(cols, 1);
    assert_eq!(widths, vec![1]);

    core.set_columns(4);
    core.set_column_padding(3);
    core.set_values(vec![
        described_suggestion("alphabet", "Inspect runtime configuration"),
        suggestion("bravo"),
    ]);
    core.update_layout(8, 0);
    assert_eq!(core.columns_for_test(), 1);
    assert_eq!(
        core.description_line().as_deref(),
        Some("Inspect runtime configuration")
    );

    let colors = MenuTextStyle {
        text_style: Style::new().fg(Color::Cyan),
        selected_text_style: Style::new().fg(Color::Black).on(Color::White),
        description_style: Style::new().fg(Color::Yellow),
        ..MenuTextStyle::default()
    };

    let ansi_entry = core.create_entry_string(&core.values()[0], 0, 0, 1, true, &colors);
    let ascii_entry = core.create_entry_string(&core.values()[0], 0, 0, 1, false, &colors);
    let ansi_description =
        core.create_description_string("Inspect runtime configuration", 12, true, &colors);
    let ascii_description =
        core.create_description_string("Inspect runtime configuration", 12, false, &colors);

    assert!(ansi_entry.contains("\u{1b}["));
    assert!(ascii_entry.starts_with("> "));
    assert!(ansi_description.contains("\u{1b}["));
    assert_eq!(ascii_description.len(), 12);

    core.set_max_rows(1);
    assert_eq!(
        core.handle_event(MenuEvent::NextPage),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_index(), Some(1));
    assert_eq!(
        core.handle_event(MenuEvent::PreviousPage),
        MenuAction::ApplySelection
    );
    assert_eq!(core.selected_index(), Some(0));
}
