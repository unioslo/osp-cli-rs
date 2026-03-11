//! Internal completion, history, highlight, and trace adapters for the REPL
//! engine.
//!
//! These helpers sit at the boundary where the semantic REPL surface meets
//! editor-facing mechanics such as reedline suggestions, path enumeration,
//! menu styling, and trace payloads.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::completion::{
    ArgNode, CompletionEngine, CompletionNode, CompletionTree, SuggestionEntry, SuggestionOutput,
};
use crate::core::fuzzy::fold_case;
use crate::core::shell_words::{QuoteStyle, escape_for_shell, quote_for_shell};
use crate::repl::highlight::ReplHighlighter;
use nu_ansi_term::{Color, Style};
use reedline::{Completer, Span, Suggestion};
use serde::Serialize;

use super::config::DEFAULT_HISTORY_MENU_ROWS;
use super::{HistoryEntry, LineProjection, LineProjector, ReplAppearance, SharedHistory};

pub(crate) struct ReplCompleter {
    engine: CompletionEngine,
    line_projector: Option<LineProjector>,
}

impl ReplCompleter {
    pub(crate) fn new(
        mut words: Vec<String>,
        completion_tree: Option<CompletionTree>,
        line_projector: Option<LineProjector>,
    ) -> Self {
        words.sort();
        words.dedup();
        let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&words));
        Self {
            engine: CompletionEngine::new(tree),
            line_projector,
        }
    }
}

impl Completer for ReplCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        debug_assert!(
            pos <= line.len(),
            "completer received pos {pos} beyond line length {}",
            line.len()
        );
        let projected = self
            .line_projector
            .as_ref()
            .map(|project| project(line))
            .unwrap_or_else(|| LineProjection::passthrough(line));
        // Completion runs against the projected line so host-only flags and
        // aliases do not distort command-path or DSL suggestions.
        let (cursor_state, outputs) = self.engine.complete(&projected.line, pos);
        let span = Span {
            start: cursor_state.replace_range.start,
            end: cursor_state.replace_range.end,
        };

        let mut ranked = Vec::new();
        let mut has_path_sentinel = false;
        for output in outputs {
            match output {
                SuggestionOutput::Item(item) => ranked.push(item),
                SuggestionOutput::PathSentinel => has_path_sentinel = true,
            }
        }

        let mut hidden_suggestions = projected.hidden_suggestions.clone();
        if !cursor_state.token_stub.is_empty() {
            // Keep the actively edited token visible even when projection-level
            // policy would normally hide it. This prevents menu refresh from
            // dropping the exact value that is already inserted in the prompt
            // until a real delimiter commits the token's scope.
            hidden_suggestions
                .retain(|value| !value.eq_ignore_ascii_case(cursor_state.token_stub.as_str()));
        }

        let mut suggestions = ranked
            .into_iter()
            .filter(|item| !hidden_suggestions.contains(&item.text))
            .map(|item| Suggestion {
                value: item.text,
                description: item.meta,
                extra: item.display.map(|display| vec![display]),
                span,
                append_whitespace: true,
                ..Suggestion::default()
            })
            .collect::<Vec<_>>();

        if has_path_sentinel {
            // The pure completion engine reports that this slot expects a path;
            // filesystem enumeration happens here at the editor boundary.
            suggestions.extend(path_suggestions(
                &cursor_state.raw_stub,
                &cursor_state.token_stub,
                cursor_state.quote_style,
                span,
            ));
        }

        suggestions
    }
}

pub(crate) struct ReplHistoryCompleter {
    history: SharedHistory,
}

impl ReplHistoryCompleter {
    pub(crate) fn new(history: SharedHistory) -> Self {
        Self { history }
    }
}

impl Completer for ReplHistoryCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let query = line
            .get(..pos.min(line.len()))
            .unwrap_or(line)
            .trim()
            .to_string();
        let query_folded = fold_case(&query);
        let replace_span = Span {
            start: 0,
            end: line.len(),
        };

        let mut seen = BTreeSet::new();
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        let mut substring = Vec::new();
        let mut recent = Vec::new();

        for entry in self.history.list_entries().into_iter().rev() {
            if !seen.insert(entry.command.clone()) {
                continue;
            }

            if query_folded.is_empty() {
                recent.push(history_suggestion(entry, replace_span));
                if recent.len() >= DEFAULT_HISTORY_MENU_ROWS as usize {
                    break;
                }
                continue;
            }

            let command_folded = fold_case(&entry.command);
            let suggestion = history_suggestion(entry.clone(), replace_span);
            if command_folded == query_folded {
                exact.push(suggestion);
            } else if command_folded.starts_with(&query_folded) {
                prefix.push(suggestion);
            } else if command_folded.contains(&query_folded) {
                substring.push(suggestion);
            }
        }

        if query_folded.is_empty() {
            return recent;
        }

        exact
            .into_iter()
            .chain(prefix)
            .chain(substring)
            .take(DEFAULT_HISTORY_MENU_ROWS as usize)
            .collect()
    }
}

fn history_suggestion(entry: HistoryEntry, span: Span) -> Suggestion {
    Suggestion {
        value: entry.command.clone(),
        extra: Some(vec![format!("{}  {}", entry.id, entry.command)]),
        span,
        append_whitespace: false,
        ..Suggestion::default()
    }
}

/// Returns the default DSL verbs exposed after `|` in the REPL.
pub fn default_pipe_verbs() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("F".to_string(), "Filter rows".to_string()),
        ("P".to_string(), "Project columns".to_string()),
        ("S".to_string(), "Sort rows".to_string()),
        ("G".to_string(), "Group rows".to_string()),
        ("A".to_string(), "Aggregate rows/groups".to_string()),
        ("L".to_string(), "Limit rows".to_string()),
        ("Z".to_string(), "Collapse grouped output".to_string()),
        ("C".to_string(), "Count rows".to_string()),
        ("Y".to_string(), "Mark output for copy".to_string()),
        ("H".to_string(), "Show DSL help".to_string()),
        ("V".to_string(), "Value-only quick search".to_string()),
        ("K".to_string(), "Key-only quick search".to_string()),
        ("?".to_string(), "Clean rows / exists filter".to_string()),
        ("U".to_string(), "Unroll list field".to_string()),
        ("JQ".to_string(), "Run jq-like expression".to_string()),
        ("VAL".to_string(), "Extract values".to_string()),
        ("VALUE".to_string(), "Extract values".to_string()),
    ])
}

pub(crate) fn build_repl_tree(words: &[String]) -> CompletionTree {
    let suggestions = words
        .iter()
        .map(|word| SuggestionEntry::value(word.clone()))
        .collect::<Vec<_>>();
    let args = (0..12)
        .map(|_| ArgNode {
            suggestions: suggestions.clone(),
            ..ArgNode::default()
        })
        .collect::<Vec<_>>();

    CompletionTree {
        root: CompletionNode {
            args,
            ..CompletionNode::default()
        },
        pipe_verbs: default_pipe_verbs(),
    }
}

pub(crate) fn build_repl_highlighter(
    tree: &CompletionTree,
    appearance: &ReplAppearance,
    line_projector: Option<LineProjector>,
) -> Option<ReplHighlighter> {
    let command_color = appearance
        .command_highlight_style
        .as_deref()
        .and_then(color_from_style_spec);
    Some(ReplHighlighter::new(
        tree.clone(),
        command_color?,
        line_projector,
    ))
}

pub(crate) fn style_with_fg_bg(fg: Option<Color>, bg: Option<Color>) -> Style {
    let mut style = Style::new();
    if let Some(fg) = fg {
        style = style.fg(fg);
    }
    if let Some(bg) = bg {
        style = style.on(bg);
    }
    style
}

/// Parses a REPL style string and extracts a terminal color.
///
/// The parser accepts simple named colors as well as `#rrggbb`, `#rgb`,
/// `ansiNN`, and `rgb(r,g,b)` forms. Non-color attributes such as `bold` are
/// ignored when selecting the effective color token.
pub fn color_from_style_spec(spec: &str) -> Option<Color> {
    let token = extract_color_token(spec)?;
    parse_color_token(token)
}

fn extract_color_token(spec: &str) -> Option<&str> {
    let attrs = [
        "bold",
        "dim",
        "dimmed",
        "italic",
        "underline",
        "blink",
        "reverse",
        "hidden",
        "strikethrough",
    ];

    let mut last: Option<&str> = None;
    for part in spec.split_whitespace() {
        let token = part
            .trim()
            .strip_prefix("fg:")
            .or_else(|| part.trim().strip_prefix("bg:"))
            .unwrap_or(part.trim());
        if token.is_empty() {
            continue;
        }
        if attrs.iter().any(|attr| token.eq_ignore_ascii_case(attr)) {
            continue;
        }
        last = Some(token);
    }
    last
}

fn parse_color_token(token: &str) -> Option<Color> {
    let normalized = token.trim().to_ascii_lowercase();

    if let Some(value) = normalized.strip_prefix('#') {
        if value.len() == 6 {
            let r = u8::from_str_radix(&value[0..2], 16).ok()?;
            let g = u8::from_str_radix(&value[2..4], 16).ok()?;
            let b = u8::from_str_radix(&value[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        if value.len() == 3 {
            let r = u8::from_str_radix(&value[0..1], 16).ok()?;
            let g = u8::from_str_radix(&value[1..2], 16).ok()?;
            let b = u8::from_str_radix(&value[2..3], 16).ok()?;
            return Some(Color::Rgb(
                r.saturating_mul(17),
                g.saturating_mul(17),
                b.saturating_mul(17),
            ));
        }
    }

    if let Some(value) = normalized.strip_prefix("ansi")
        && let Ok(index) = value.parse::<u8>()
    {
        return Some(Color::Fixed(index));
    }

    if let Some(value) = normalized
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let mut parts = value.split(',').map(|part| part.trim().parse::<u8>().ok());
        if let (Some(Some(r)), Some(Some(g)), Some(Some(b))) =
            (parts.next(), parts.next(), parts.next())
        {
            return Some(Color::Rgb(r, g, b));
        }
    }

    match normalized.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" | "purple" => Some(Color::Purple),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "darkgray" | "dark_gray" | "gray" | "grey" => Some(Color::DarkGray),
        "lightgray" | "light_gray" | "lightgrey" | "light_grey" => Some(Color::LightGray),
        "lightred" | "light_red" => Some(Color::LightRed),
        "lightgreen" | "light_green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" | "lightpurple" | "light_purple" => {
            Some(Color::LightPurple)
        }
        "lightcyan" | "light_cyan" => Some(Color::LightCyan),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompletionTraceMenuState {
    pub selected_index: i64,
    pub selected_row: u16,
    pub selected_col: u16,
    pub active: bool,
    pub just_activated: bool,
    pub columns: u16,
    pub visible_rows: u16,
    pub rows: u16,
    pub menu_indent: u16,
}

#[derive(Debug, Clone, Serialize)]
struct CompletionTracePayload<'a> {
    event: &'a str,
    line: &'a str,
    cursor: usize,
    stub: &'a str,
    matches: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_before: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_after: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_before: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_after: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accepted_value: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replace_range: Option<[usize; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_row: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_col: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    just_activated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    columns: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visible_rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    menu_indent: Option<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct CompletionTraceEvent<'a> {
    pub event: &'a str,
    pub line: &'a str,
    pub cursor: usize,
    pub stub: &'a str,
    pub matches: Vec<String>,
    pub replace_range: Option<[usize; 2]>,
    pub menu: Option<CompletionTraceMenuState>,
    pub buffer_before: Option<&'a str>,
    pub buffer_after: Option<&'a str>,
    pub cursor_before: Option<usize>,
    pub cursor_after: Option<usize>,
    pub accepted_value: Option<&'a str>,
}

pub(crate) fn trace_completion(trace: CompletionTraceEvent<'_>) {
    if !trace_completion_enabled() {
        return;
    }

    let (
        selected_index,
        selected_row,
        selected_col,
        active,
        just_activated,
        columns,
        visible_rows,
        rows,
        menu_indent,
    ) = if let Some(menu) = trace.menu {
        (
            Some(menu.selected_index),
            Some(menu.selected_row),
            Some(menu.selected_col),
            Some(menu.active),
            Some(menu.just_activated),
            Some(menu.columns),
            Some(menu.visible_rows),
            Some(menu.rows),
            Some(menu.menu_indent),
        )
    } else {
        (None, None, None, None, None, None, None, None, None)
    };

    let payload = CompletionTracePayload {
        event: trace.event,
        line: trace.line,
        cursor: trace.cursor,
        stub: trace.stub,
        matches: trace.matches,
        buffer_before: trace.buffer_before,
        buffer_after: trace.buffer_after,
        cursor_before: trace.cursor_before,
        cursor_after: trace.cursor_after,
        accepted_value: trace.accepted_value,
        replace_range: trace.replace_range,
        selected_index,
        selected_row,
        selected_col,
        active,
        just_activated,
        columns,
        visible_rows,
        rows,
        menu_indent,
    };

    let serialized = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
    if let Ok(path) = std::env::var("OSP_REPL_TRACE_PATH")
        && !path.trim().is_empty()
    {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{serialized}");
        }
    } else {
        eprintln!("{serialized}");
    }
}

pub(crate) fn trace_completion_enabled() -> bool {
    let Ok(raw) = std::env::var("OSP_REPL_TRACE_COMPLETION") else {
        return false;
    };
    !matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "off" | "no"
    )
}

pub(crate) fn path_suggestions(
    raw_stub: &str,
    token_stub: &str,
    quote_style: Option<QuoteStyle>,
    span: Span,
) -> Vec<Suggestion> {
    let (lookup, insert_prefix, typed_prefix) = split_path_stub(token_stub);
    let read_dir = std::fs::read_dir(&lookup);
    let Ok(entries) = read_dir else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(&typed_prefix) {
            continue;
        }

        let path = entry.path();
        let is_dir = path.is_dir();
        let suffix = if is_dir { "/" } else { "" };
        let inserted = render_path_completion(
            raw_stub,
            &format!("{insert_prefix}{file_name}{suffix}"),
            quote_style,
        );

        out.push(Suggestion {
            value: inserted,
            description: Some(if is_dir { "dir" } else { "file" }.to_string()),
            span,
            append_whitespace: !is_dir,
            ..Suggestion::default()
        });
    }

    out
}

fn render_path_completion(
    raw_stub: &str,
    candidate: &str,
    quote_style: Option<QuoteStyle>,
) -> String {
    match infer_quote_context(raw_stub, quote_style) {
        PathQuoteContext::Open(style) => quoted_completion_tail(candidate, style),
        PathQuoteContext::Closed(style) => quote_for_shell(candidate, style),
        PathQuoteContext::Unquoted => escape_for_shell(candidate),
    }
}

fn quoted_completion_tail(candidate: &str, style: QuoteStyle) -> String {
    let quoted = quote_for_shell(candidate, style);
    quoted.chars().skip(1).collect()
}

fn infer_quote_context(raw_stub: &str, quote_style: Option<QuoteStyle>) -> PathQuoteContext {
    if let Some(style) = quote_style {
        return PathQuoteContext::Open(style);
    }

    if raw_stub.len() >= 2 && raw_stub.starts_with('\'') && raw_stub.ends_with('\'') {
        return PathQuoteContext::Closed(QuoteStyle::Single);
    }
    if raw_stub.len() >= 2 && raw_stub.starts_with('"') && raw_stub.ends_with('"') {
        return PathQuoteContext::Closed(QuoteStyle::Double);
    }

    PathQuoteContext::Unquoted
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathQuoteContext {
    Unquoted,
    Open(QuoteStyle),
    Closed(QuoteStyle),
}

pub(crate) fn split_path_stub(stub: &str) -> (PathBuf, String, String) {
    if stub.is_empty() {
        return (PathBuf::from("."), String::new(), String::new());
    }

    let expanded = expand_home(stub);
    let mut lookup = PathBuf::from(&expanded);
    if stub.ends_with('/') {
        return (lookup, stub.to_string(), String::new());
    }

    let typed_prefix = Path::new(stub)
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_default();

    let insert_prefix = match stub.rfind('/') {
        Some(index) => stub[..=index].to_string(),
        None => String::new(),
    };

    if let Some(parent) = lookup.parent() {
        if parent.as_os_str().is_empty() {
            lookup = PathBuf::from(".");
        } else {
            lookup = parent.to_path_buf();
        }
    } else {
        lookup = PathBuf::from(".");
    }

    (lookup, insert_prefix, typed_prefix)
}

pub(crate) fn expand_home(path: &str) -> String {
    if path == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    path.to_string()
}
