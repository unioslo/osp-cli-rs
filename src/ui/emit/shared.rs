use crate::ui::visible_inline_text;

pub(super) fn indent_lines(text: &str, margin: usize) -> String {
    let prefix = " ".repeat(margin);
    text.lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn format_list_item(item: &str, inline_markup: bool) -> String {
    if inline_markup {
        visible_inline_text(item)
    } else {
        item.to_string()
    }
}
