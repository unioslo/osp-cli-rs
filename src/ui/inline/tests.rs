use super::{line_from_inline, parts_from_inline, render_inline};
use crate::ui::{StyleOverrides, StyleToken, resolve_theme};

#[test]
fn parts_from_inline_preserves_escaped_and_unmatched_markers_unit() {
    let parts = parts_from_inline(r"Use \*literal\* and \`code\` and *open");

    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].token, None);
    assert_eq!(parts[0].text, "Use *literal* and `code` and *open");
}

#[test]
fn parts_from_inline_supports_double_backtick_fences_unit() {
    let parts = parts_from_inline("``uid value``");

    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].token, Some(StyleToken::Key));
    assert_eq!(parts[0].text, "uid value");
}

#[test]
fn line_from_inline_wraps_parts_without_changing_markup_semantics_unit() {
    let line = line_from_inline("*muted*");

    assert_eq!(line.parts.len(), 1);
    assert_eq!(line.parts[0].token, Some(StyleToken::Muted));
    assert_eq!(line.parts[0].text, "muted");
}

#[test]
fn render_inline_without_color_strips_markup_to_plain_text_unit() {
    let rendered = render_inline(
        "Use `uid` and **flags**",
        false,
        &resolve_theme("dracula"),
        &StyleOverrides::default(),
    );

    assert_eq!(rendered, "Use uid and flags");
}
