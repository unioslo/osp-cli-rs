use std::time::Duration;

use osp_ui::ResolvedRenderSettings;
use osp_ui::style::{StyleToken, apply_style_with_theme_overrides};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimingSummary {
    pub(crate) total: Duration,
    pub(crate) parse: Option<Duration>,
    pub(crate) execute: Option<Duration>,
    pub(crate) render: Option<Duration>,
}

pub(crate) fn format_timing_badge(
    summary: TimingSummary,
    debug_level: u8,
    resolved: &ResolvedRenderSettings,
) -> String {
    if debug_level == 0 {
        return String::new();
    }

    let mut text = format_duration(summary.total, debug_level);
    if debug_level >= 3 {
        let mut parts = Vec::new();
        if let Some(parse) = summary.parse {
            parts.push(format!("p{}", format_duration(parse, 2)));
        }
        if let Some(execute) = summary.execute {
            parts.push(format!("e{}", format_duration(execute, 2)));
        }
        if let Some(render) = summary.render {
            parts.push(format!("r{}", format_duration(render, 2)));
        }
        if !parts.is_empty() {
            text.push(' ');
            text.push_str(&parts.join(" "));
        }
    }

    apply_style_with_theme_overrides(
        &text,
        timing_style(summary.total),
        resolved.color,
        &resolved.theme,
        &resolved.style_overrides,
    )
}

pub(crate) fn right_align_timing_line(
    summary: TimingSummary,
    debug_level: u8,
    resolved: &ResolvedRenderSettings,
) -> String {
    let badge = format_timing_badge(summary, debug_level, resolved);
    if badge.is_empty() {
        return String::new();
    }

    let width = resolved.width.unwrap_or(80);
    let visible = visible_width(&badge);
    let padding = width.saturating_sub(visible);
    format!("{}{}\n", " ".repeat(padding), badge)
}

fn timing_style(total: Duration) -> StyleToken {
    let total_ms = total.as_millis();
    if total_ms <= 250 {
        StyleToken::MessageSuccess
    } else if total_ms <= 1_000 {
        StyleToken::MessageWarning
    } else {
        StyleToken::MessageError
    }
}

fn format_duration(duration: Duration, debug_level: u8) -> String {
    let secs = duration.as_secs_f64();
    if debug_level <= 1 {
        if duration.as_millis() == 0 && !duration.is_zero() {
            return "<1ms".to_string();
        }
        if secs >= 1.0 {
            return format!("{:.2}s", secs);
        }
        return format!("{}ms", duration.as_millis());
    }

    if secs >= 1.0 {
        return format!("{secs:.2}s");
    }
    let ms = duration.as_secs_f64() * 1_000.0;
    format!("{ms:.1}ms")
}

fn visible_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' && matches!(chars.peek(), Some('[')) {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        width += 1;
    }

    width
}

#[cfg(test)]
mod tests {
    use super::{TimingSummary, format_timing_badge};
    use osp_ui::RenderSettings;
    use std::time::Duration;

    #[test]
    fn level_three_timing_includes_phase_breakdown() {
        let resolved = RenderSettings::test_plain(osp_core::output::OutputFormat::Table)
            .resolve_render_settings();
        let text = format_timing_badge(
            TimingSummary {
                total: Duration::from_millis(187),
                parse: Some(Duration::from_millis(2)),
                execute: Some(Duration::from_millis(180)),
                render: Some(Duration::from_millis(5)),
            },
            3,
            &resolved,
        );

        assert!(text.contains("187.0ms"));
        assert!(text.contains("p2.0ms"));
        assert!(text.contains("e180.0ms"));
        assert!(text.contains("r5.0ms"));
    }
}
