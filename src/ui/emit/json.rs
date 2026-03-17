use serde_json::Value;

use crate::ui::settings::ResolvedRenderSettings;
use crate::ui::style::{StyleToken, ThemeStyler};

use super::shared::indent_lines;

pub(super) fn emit_value(value: &Value, settings: &ResolvedRenderSettings) -> String {
    let styler = ThemeStyler::new(settings.color, &settings.theme, &settings.style_overrides);
    let rendered = render_json_value(value, &styler, 0);
    let rendered = indent_lines(&rendered, settings.margin);
    if rendered.is_empty() || rendered.ends_with('\n') {
        rendered
    } else {
        format!("{rendered}\n")
    }
}

fn render_json_value(value: &Value, styler: &ThemeStyler<'_>, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let next_indent = "  ".repeat(depth + 1);

    match value {
        Value::Object(map) => {
            if map.is_empty() {
                return punct(styler, "{}");
            }

            let mut out = punct(styler, "{");
            out.push('\n');
            for (index, (key, item)) in map.iter().enumerate() {
                let key = serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""));
                out.push_str(&next_indent);
                out.push_str(&styler.paint(&key, StyleToken::JsonKey));
                out.push_str(&punct(styler, ":"));
                out.push(' ');
                out.push_str(&render_json_value(item, styler, depth + 1));
                if index + 1 < map.len() {
                    out.push_str(&punct(styler, ","));
                }
                out.push('\n');
            }
            out.push_str(&indent);
            out.push_str(&punct(styler, "}"));
            out
        }
        Value::Array(items) => {
            if items.is_empty() {
                return punct(styler, "[]");
            }

            let mut out = punct(styler, "[");
            out.push('\n');
            for (index, item) in items.iter().enumerate() {
                out.push_str(&next_indent);
                out.push_str(&render_json_value(item, styler, depth + 1));
                if index + 1 < items.len() {
                    out.push_str(&punct(styler, ","));
                }
                out.push('\n');
            }
            out.push_str(&indent);
            out.push_str(&punct(styler, "]"));
            out
        }
        Value::String(raw) => {
            let quoted = serde_json::to_string(raw).unwrap_or_else(|_| format!("\"{raw}\""));
            styler.paint(&quoted, StyleToken::Value)
        }
        Value::Number(number) => styler.paint(&number.to_string(), StyleToken::ValueNumber),
        Value::Bool(true) => styler.paint("true", StyleToken::Success),
        Value::Bool(false) => styler.paint("false", StyleToken::Error),
        Value::Null => styler.paint("null", StyleToken::TextMuted),
    }
}

fn punct(styler: &ThemeStyler<'_>, text: &str) -> String {
    styler.paint(text, StyleToken::Punctuation)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use crate::ui::settings::{RenderProfile, RenderSettings, resolve_settings};

    use super::emit_value;

    #[test]
    fn rich_json_emitter_styles_keys_punctuation_and_scalars_unit() {
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.mode = RenderMode::Rich;
        settings.color = ColorMode::Always;
        settings.unicode = UnicodeMode::Always;
        settings.runtime.stdout_is_tty = true;
        settings.theme_name = "dracula".to_string();

        let resolved = resolve_settings(&settings, RenderProfile::Normal);
        let rendered = emit_value(
            &json!({
                "count": 42,
                "ok": true,
                "missing": null,
            }),
            &resolved,
        );

        assert!(rendered.contains("\x1b[38;2;104;121;173m{\x1b[0m"));
        assert!(rendered.contains("\x1b[38;2;189;147;249m\"count\"\x1b[0m"));
        assert!(rendered.contains("\x1b[38;2;255;121;198m42\x1b[0m"));
        assert!(rendered.contains("\x1b[38;2;80;250;123mtrue\x1b[0m"));
        assert!(rendered.contains("\x1b[38;2;104;121;173mnull\x1b[0m"));
    }

    #[test]
    fn copy_safe_json_emitter_stays_plain_unit() {
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.mode = RenderMode::Rich;
        settings.color = ColorMode::Always;
        settings.runtime.stdout_is_tty = true;
        settings.theme_name = "dracula".to_string();

        let resolved = resolve_settings(&settings, RenderProfile::CopySafe);
        let rendered = emit_value(&json!({"count": 42}), &resolved);

        assert!(rendered.contains("\"count\": 42"));
        assert!(!rendered.contains("\x1b["));
    }
}
