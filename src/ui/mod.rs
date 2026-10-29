#![allow(missing_docs)]

//! Canonical UI pipeline.
//!
//! Commands, services and the DSL produce canonical data. The UI does not
//! re-resolve config, re-run commands or invent data semantics.
//!
//! ```text
//! OutputResult / Document
//!    ▼ plan_format   effective format, provenance, resolved settings
//!    ├ JSON/CSV/TSV   typed machine writers, without human projection or IR
//!    ▼ project       human data, labels, styles, limits, sanitization
//!    ▼ plan_layout   table fitting and table/card decisions
//!    ▼ lower         obey the plan; construct one doc.rs IR
//!    ▼ emit          render terminal or Markdown text
//! ```
//!
//! [`crate::ui::render`] takes canonical output and a [`crate::ui::RenderRequest`]. One planner
//! decides format and layout, one human projection supplies content, and one
//! lowering pass creates the private `doc.rs` IR. Emitters stay about rendering
//! only: semantic tokens defer palette resolution to `ThemeStyler`.
//! Producer document roles carry intent; ordinary fields and English titles
//! never select it. Shared chrome geometry decides how, while callers decide
//! when headings and rules appear. Messages retain their event/severity model.

pub(crate) mod clipboard;
pub(crate) mod messages;
pub(crate) mod section_chrome;
pub(crate) mod style;
pub(crate) mod theme;
pub(crate) mod theme_catalog;

mod chrome;
mod doc;
mod emit;
mod lower;
mod plan;
mod projection;
pub(crate) mod settings;
mod text;

use crate::core::document::DocBlock;
use crate::core::output::OutputFormat;
use crate::core::output_model::{DisplayRule, OutputResult};

pub(crate) use emit::json::json_value;
pub(crate) use emit::shared::fit_prose;
pub(crate) use messages::{MessageRequest, render as render_messages};
pub(crate) use plan::plan_format;
pub use section_chrome::{RuledSectionPolicy, SectionFrameStyle};
pub(crate) use settings::RenderBackend;
pub use settings::{
    HelpLayout, RenderRuntime, RenderSettings, RenderTarget, TableBorderOverride, TableBorderStyle,
    TableOverflow, TableTooWide, UiPresentation,
};
pub use settings::{ResolvedRenderSettings, resolve_settings};
pub(crate) use settings::{
    build_presentation_defaults_layer, explain_presentation_effect, help_layout_from_config,
};
pub use style::StyleOverrides;
#[cfg(test)]
pub(crate) use style::ThemeStyler;
pub(crate) use text::{
    crop_display_width, display_width, sanitize_human_text, visible_inline_text, wrap_display_width,
};
#[cfg(test)]
pub(crate) use theme::DEFAULT_THEME_NAME;

const LOCAL_TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M %:z";

/// Formats an RFC3339 timestamp in local time at minute precision.
pub fn format_local_timestamp(value: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| format_local_utc_timestamp(time.with_timezone(&chrono::Utc)))
}

/// Formats an RFC3339 timestamp as a compact duration from the current time.
pub fn format_relative_timestamp(value: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| format_relative_unix_timestamp(time.timestamp()))
}

/// Formats a byte count with the largest useful binary unit.
pub(crate) fn format_binary_bytes(bytes: u64) -> String {
    for (unit, divisor) in [
        ("TiB", 1_u64 << 40),
        ("GiB", 1_u64 << 30),
        ("MiB", 1_u64 << 20),
        ("KiB", 1_u64 << 10),
    ] {
        if bytes >= divisor {
            return if bytes.is_multiple_of(divisor) {
                format!("{} {unit}", bytes / divisor)
            } else {
                format!("{:.1} {unit}", bytes as f64 / divisor as f64)
            };
        }
    }
    format!("{bytes} B")
}

fn format_local_utc_timestamp(time: chrono::DateTime<chrono::Utc>) -> String {
    time.with_timezone(&chrono::Local)
        .format(LOCAL_TIMESTAMP_FORMAT)
        .to_string()
}

pub(crate) fn format_local_unix_timestamp(seconds: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(seconds, 0).map(format_local_utc_timestamp)
}

pub(crate) fn format_relative_unix_timestamp(seconds: i64) -> String {
    let delta = seconds.saturating_sub(chrono::Utc::now().timestamp());
    if delta == 0 {
        return "now".to_string();
    }
    let magnitude = delta.unsigned_abs();
    let (amount, unit) = if magnitude < 60 {
        (magnitude, "s")
    } else if magnitude < 60 * 60 {
        (magnitude / 60, "m")
    } else if magnitude < 24 * 60 * 60 {
        (magnitude / (60 * 60), "h")
    } else {
        (magnitude / (24 * 60 * 60), "d")
    };
    if delta < 0 {
        format!("{amount}{unit} ago")
    } else {
        format!("in {amount}{unit}")
    }
}

/// Settings and human presentation for one render operation.
#[derive(Debug, Clone, Copy)]
pub struct RenderRequest<'a> {
    /// Effective output and terminal settings.
    pub settings: &'a RenderSettings,
    /// Destination policy for width, colour, margin and display limits.
    pub target: RenderTarget,
    /// Optional help layout for an unstaged semantic document.
    pub layout: Option<HelpLayout>,
    /// Prefix for the first semantic section title.
    pub title_prefix: Option<&'a str>,
    /// Override the layout's default closing rule.
    pub show_footer_rule: Option<bool>,
    /// Trusted artwork beside an opening paragraph, with prose fitted to the screen.
    /// An empty slice fits prose without artwork; exports ignore this decoration.
    pub leading_decoration: Option<&'a [String]>,
}

impl<'a> RenderRequest<'a> {
    /// Uses ordinary output rendering without additional help chrome.
    pub fn new(settings: &'a RenderSettings) -> Self {
        Self {
            settings,
            target: RenderTarget::screen(settings),
            layout: None,
            title_prefix: None,
            show_footer_rule: None,
            leading_decoration: None,
        }
    }
}

/// Renders canonical data, optionally using its unstaged semantic document.
///
/// ```
/// use osp_cli::{row, core::OutputResult};
/// use osp_cli::core::OutputFormat;
/// use osp_cli::ui::{render, RenderRequest, RenderSettings};
///
/// let output = OutputResult::from_rows(vec![row! { "uid" => "alice" }]);
/// let settings = RenderSettings::test_plain(OutputFormat::Markdown);
/// let rendered = render(&output, &RenderRequest::new(&settings));
/// assert!(rendered.contains("alice"));
/// ```
pub fn render(output: &OutputResult, request: &RenderRequest<'_>) -> String {
    let plan = plan_format(output, request.settings, request.target);
    if matches!(
        plan.format,
        OutputFormat::Markdown | OutputFormat::Json | OutputFormat::Csv | OutputFormat::Tsv
    ) {
        let margin = plan.settings.margin;
        emit::shared::apply_margin(render_planned(output, request, plan), margin)
    } else {
        render_planned(output, request, plan)
    }
}

fn render_planned(
    output: &OutputResult,
    request: &RenderRequest<'_>,
    mut plan: plan::RenderPlan,
) -> String {
    let canonical = output;
    match plan.format {
        OutputFormat::Csv | OutputFormat::Tsv => {
            return emit::delimited::render(output, plan.format);
        }
        OutputFormat::Json => return emit::json::emit_value(&json_value(output)),
        _ => {}
    }
    let projection = projection::project(output, plan.format, &plan.settings);
    plan::plan_layout(&projection, &mut plan);
    let output = &projection.output;
    let mut doc = if matches!(
        plan.format,
        OutputFormat::Auto | OutputFormat::Guide | OutputFormat::Markdown
    ) && let Some(layout) = request.layout
        && let Some(document) = output
            .document
            .as_ref()
            .and_then(crate::core::output_model::OutputDocument::semantic)
    {
        let mut document = document.clone();
        if let Some(prefix) = request.title_prefix
            && let Some(DocBlock::Section { title, .. }) = document
                .blocks
                .iter_mut()
                .find(|block| matches!(block, DocBlock::Section { .. }))
        {
            *title = format!("{} · {title}", sanitize_human_text(prefix));
        }
        if plan.format == OutputFormat::Markdown {
            lower::lower_document(&document, &plan)
        } else {
            lower::lower_document_help_layout(
                &document,
                &plan,
                layout,
                request
                    .show_footer_rule
                    .unwrap_or_else(|| default_show_footer_rule(layout)),
            )
        }
    } else {
        lower::lower_output(&projection, &plan)
    };
    if request.target == RenderTarget::Clipboard && plan.format != OutputFormat::Markdown {
        remove_terminal_chrome(&mut doc.blocks);
    }
    let text = if matches!(plan.format, OutputFormat::Auto | OutputFormat::Guide)
        && request.target.is_screen()
        && request.settings.runtime.stdout_is_tty
        && canonical
            .document
            .as_ref()
            .and_then(crate::core::output_model::OutputDocument::semantic)
            .is_some()
        && let Some(decoration) = request.leading_decoration
    {
        emit::emit_with_leading_decoration(&doc, plan.format, &plan.settings, decoration)
    } else {
        emit::emit_doc(&doc, plan.format, &plan.settings)
    };
    if plan.settings.backend == RenderBackend::Rich
        && request.settings.runtime.stdout_is_tty
        && request.target.is_screen()
        && !matches!(plan.format, OutputFormat::Json | OutputFormat::Markdown)
    {
        return link_urls(text, canonical);
    }
    text
}

fn remove_terminal_chrome(blocks: &mut Vec<doc::Block>) {
    blocks.retain(|block| !matches!(block, doc::Block::Rule));
    for block in blocks {
        if let doc::Block::Section(section) = block {
            if section.title_chrome == doc::SectionTitleChrome::Ruled {
                section.title_chrome = doc::SectionTitleChrome::Plain;
            }
            remove_terminal_chrome(&mut section.blocks);
        }
    }
}

fn link_urls(text: String, output: &OutputResult) -> String {
    let mut urls = std::collections::BTreeSet::new();
    for row in output.as_rows().unwrap_or_default() {
        let row = serde_json::Value::Object(row.clone());
        for rule in &output.meta.display_rules {
            if let DisplayRule::Hyperlink { field } = rule
                && let Some(url) =
                    projection::display_path(&row, field).and_then(serde_json::Value::as_str)
                && (url.starts_with("https://") || url.starts_with("http://"))
                && !url.chars().any(char::is_control)
            {
                urls.insert(url.to_owned());
            }
        }
    }
    let mut rendered = String::new();
    let mut remaining = text.as_str();
    while let Some((offset, url)) = urls
        .iter()
        .filter_map(|url| remaining.find(url.as_str()).map(|offset| (offset, url)))
        .min_by_key(|(offset, url)| (*offset, std::cmp::Reverse(url.len())))
    {
        rendered.push_str(&remaining[..offset]);
        rendered.push_str(&format!("\x1b]8;;{url}\x1b\\{url}\x1b]8;;\x1b\\"));
        remaining = &remaining[offset + url.len()..];
    }
    rendered.push_str(remaining);
    rendered
}

fn default_show_footer_rule(layout: HelpLayout) -> bool {
    matches!(layout, HelpLayout::Full)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod document_tests;
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;

pub mod interrupt;
pub mod picker;
pub mod prompt;

/// Renders one levelled message through the shared message renderer.
/// Interactive lesson screens may select plain layout to avoid extra chrome.
pub fn render_message(
    level: crate::core::MessageLevel,
    text: impl Into<String>,
    settings: &RenderSettings,
    verbosity: crate::core::MessageLevel,
) -> String {
    let text = text.into();
    if level > verbosity {
        return String::new();
    }
    if settings.requested_format() == OutputFormat::Json {
        return format!(
            "{}\n",
            serde_json::json!({"messages": [{"level": level, "text": text}]})
        );
    }
    let mut messages = messages::MessageBuffer::default();
    messages.push(level, text);
    let mut request = MessageRequest::new(settings, verbosity);
    request.layout = messages::MessageLayout::Plain;
    render_messages(&messages, &request)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod rules_tests;
