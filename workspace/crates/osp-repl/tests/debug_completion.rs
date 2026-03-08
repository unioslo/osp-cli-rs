use std::collections::BTreeMap;

use osp_completion::{ArgNode, CompletionNode, CompletionTree, SuggestionEntry};
use osp_repl::{
    CompletionDebugOptions, DebugStep, ReplAppearance, debug_completion, debug_completion_steps,
};

#[test]
fn debug_completion_reports_menu_styles_and_selection() {
    let mut root = CompletionNode::default();
    root.children.insert(
        "doctor".to_string(),
        CompletionNode {
            tooltip: Some("Run diagnostics".to_string()),
            ..CompletionNode::default()
        },
    );
    let tree = CompletionTree {
        root,
        pipe_verbs: BTreeMap::new(),
    };

    let appearance = ReplAppearance {
        completion_text_style: Some("fg:cyan".to_string()),
        completion_background_style: Some("bg:#112233".to_string()),
        completion_highlight_style: Some("#ff00ff".to_string()),
        command_highlight_style: None,
    };

    let debug = debug_completion(
        &tree,
        "",
        0,
        CompletionDebugOptions::new(40, 5)
            .ansi(true)
            .unicode(true)
            .appearance(Some(&appearance)),
    );

    assert_eq!(debug.menu_styles.text.foreground.as_deref(), Some("cyan"));
    assert_eq!(
        debug.menu_styles.text.background.as_deref(),
        Some("rgb:17,34,51")
    );
    assert_eq!(
        debug.menu_styles.selected_text.foreground.as_deref(),
        Some("rgb:255,0,255")
    );
    assert_eq!(
        debug.menu_styles.selected_text.background.as_deref(),
        Some("cyan")
    );
    assert_eq!(debug.selected, -1);
    assert_eq!(debug.selected_row, 0);
    assert_eq!(debug.selected_col, 0);
    assert_eq!(debug.menu_description.as_deref(), None);
}

#[test]
fn debug_completion_steps_accepts_after_second_tab() {
    let mut root = CompletionNode::default();
    root.children
        .insert("config".to_string(), CompletionNode::default());
    let tree = CompletionTree {
        root,
        pipe_verbs: BTreeMap::new(),
    };

    let frames = debug_completion_steps(
        &tree,
        "co",
        2,
        CompletionDebugOptions::new(40, 5),
        &[DebugStep::Tab, DebugStep::Tab, DebugStep::Accept],
    );

    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].state.line, "co");
    assert_eq!(frames[0].state.selected, -1);
    assert!(!frames[0].state.rendered.is_empty());
    assert_eq!(frames[2].state.line, "config ");
}

#[test]
fn debug_completion_tracks_replace_range_inside_open_quotes() {
    let user = CompletionNode {
        args: vec![ArgNode {
            name: Some("uid".to_string()),
            suggestions: vec![SuggestionEntry::value("oistes")],
            ..ArgNode::default()
        }],
        ..CompletionNode::default()
    };

    let mut ldap = CompletionNode::default();
    ldap.children.insert("user".to_string(), user);

    let tree = CompletionTree {
        root: CompletionNode::default().with_child("ldap", ldap),
        pipe_verbs: BTreeMap::new(),
    };

    let line = "ldap user \"oi";
    let debug = debug_completion(&tree, line, line.len(), CompletionDebugOptions::new(40, 5));

    let start = line.find("oi").expect("quoted stub should exist");
    assert_eq!(debug.stub, "oi");
    assert_eq!(debug.replace_range, [start, line.len()]);
    assert!(debug.matches.iter().any(|entry| entry.label == "oistes"));
}
