//! Inline suggestion and status line painted after the REPL input.
//!
//! reedline paints a hinter's output right after the input line, so one
//! hinter carries both pieces: muted text that would complete the line
//! (accepted with Right or End, one word with Alt-Right) and, on the line
//! below, what the word at the cursor means or what is wrong with the line.
//! reedline hides hints while a menu is open and when a line is submitted, so
//! neither reaches the scrollback.
//!
//! The hinter is the only editor hook that sees every painted line, so it also
//! records that line for the edit mode in [`PaintedLine`].

use nu_ansi_term::Style;
use reedline::{Completer, Hinter, History, SearchQuery, Suggestion};
use std::ops::Range;
use unicode_width::UnicodeWidthChar;

use super::adapter::ReplCompleter;
use super::editor::PaintedLine;
use crate::completion::CompletionRequest;
use crate::repl::highlight::{LineProblem, LineProblemKind, ReplHighlighter};

pub(crate) struct ReplHinter {
    completer: ReplCompleter,
    highlighter: ReplHighlighter,
    painted: PaintedLine,
    // `None` turns the inline suggestion off; unstyled it would look typed.
    style: Option<Style>,
    error_style: Style,
    inline: String,
}

impl ReplHinter {
    pub(crate) fn new(
        completer: ReplCompleter,
        highlighter: ReplHighlighter,
        painted: PaintedLine,
        style: Option<Style>,
        error_style: Style,
    ) -> Self {
        Self {
            completer,
            highlighter,
            painted,
            style,
            error_style,
            inline: String::new(),
        }
    }

    /// The rest of the line the user most likely wants.
    ///
    /// The newest history line extending the input wins when it agrees with
    /// the command tree; otherwise the tree's only candidate for the current
    /// word does.
    fn inline_suggestion(
        &self,
        line: &str,
        history: &dyn History,
        candidates: &[Suggestion],
    ) -> String {
        let from_tree = only_extension(line, candidates);
        let from_history = history
            .search(SearchQuery::last_with_prefix(line.to_string(), None))
            .ok()
            .and_then(|items| items.into_iter().next())
            .and_then(|item| item.command_line.get(line.len()..).map(str::to_string))
            .filter(|rest| !rest.is_empty());
        match (from_history, from_tree) {
            (Some(past), Some(tree)) if past.starts_with(&tree) => past,
            (_, Some(tree)) => tree,
            (Some(past), None) => past,
            (None, None) => String::new(),
        }
    }

    /// One line about the cursor position; `true` marks a problem.
    fn status(
        &mut self,
        line: &str,
        pos: usize,
        candidates: &[Suggestion],
    ) -> Option<(String, bool, Option<Range<usize>>)> {
        if line.trim().is_empty() {
            return None;
        }
        if let Some(problem) = self.highlighter.problem(line, pos) {
            return Some((self.describe_problem(line, &problem), true, None));
        }

        let analysis = self.completer.analyze(line, pos);
        let engine = self.completer.engine();
        if !matches!(analysis.request, CompletionRequest::FlagValues { .. })
            && let Some((usage, active)) = engine.positional_usage(&analysis)
        {
            return Some((usage, false, active));
        }
        if !analysis.cursor.token_stub.is_empty()
            && let Some(candidate) = only_candidate(line, candidates)
            && let Some(description) = candidate.description.as_deref()
        {
            return Some((format!("{}  {description}", candidate.value), false, None));
        }
        if let CompletionRequest::FlagValues {
            flag_scope_path,
            flag,
        } = &analysis.request
        {
            let tooltip = engine
                .node_at(flag_scope_path)
                .and_then(|node| node.flags.get(flag))
                .and_then(|meta| meta.tooltip.as_deref())?;
            return Some((format!("{flag}  {tooltip}"), false, None));
        }

        let path = &analysis.context.matched_path;
        if path.is_empty() {
            return None;
        }
        let mut status = path.join(" ");
        if let Some(tooltip) = engine
            .node_at(path)
            .and_then(|node| node.tooltip.as_deref())
        {
            status.push_str("  ");
            status.push_str(tooltip);
        }
        let missing = engine.missing_required_flags(&analysis);
        if !missing.is_empty() {
            status.push_str(" · needs ");
            status.push_str(&missing.join(", "));
        }
        Some((status, false, None))
    }

    fn describe_problem(&mut self, line: &str, problem: &LineProblem) -> String {
        let word = line.get(problem.start..problem.end).unwrap_or_default();
        let mut message = match &problem.kind {
            LineProblemKind::UnknownCommand { parent } if parent.is_empty() => {
                format!("unknown command '{word}'")
            }
            LineProblemKind::UnknownCommand { parent } => {
                format!("unknown command '{word}' for '{}'", parent.join(" "))
            }
            LineProblemKind::UnknownFlag { command } if command.is_empty() => {
                format!("unknown flag '{word}'")
            }
            LineProblemKind::UnknownFlag { command } => {
                format!("unknown flag '{word}' for '{}'", command.join(" "))
            }
        };
        // The completer's fuzzy rescue ranks the closest real word first.
        let prefix = line.get(..problem.end).unwrap_or(line);
        let flag_name = word.split_once('=').map_or(word, |(name, _)| name);
        if let Some(closest) = self
            .completer
            .complete(prefix, prefix.len())
            .into_iter()
            .map(|suggestion| suggestion.value)
            .find(|value| value != flag_name)
        {
            message.push_str(&format!("; did you mean '{closest}'?"));
        }
        message
    }
}

impl Hinter for ReplHinter {
    fn handle(
        &mut self,
        line: &str,
        pos: usize,
        history: &dyn History,
        use_ansi_coloring: bool,
        _cwd: &str,
    ) -> String {
        self.painted.set(line, pos);
        let candidates = if line.trim().is_empty() {
            Vec::new()
        } else {
            self.completer.complete(line, pos)
        };
        self.inline = if use_ansi_coloring
            && self.style.is_some()
            && pos == line.len()
            && !line.trim().is_empty()
        {
            self.inline_suggestion(line, history, &candidates)
        } else {
            String::new()
        };

        let paint = |text: &str, style: Option<Style>| match style {
            Some(style) if use_ansi_coloring => style.paint(text).to_string(),
            _ => text.to_string(),
        };
        let mut out = paint(&self.inline, self.style);
        if let Some((status, is_problem, active)) = self.status(line, pos, &candidates) {
            let style = if is_problem {
                Some(self.error_style)
            } else {
                self.style
            };
            // Reedline's CRLF normalization drops the text after a leading LF.
            out.push_str("\r\n");
            let status = fit_terminal_width(&status);
            if let Some(active) = active
                .map(|range| range.start..range.end.min(status.len()))
                .filter(|range| range.start < range.end && status.get(range.clone()).is_some())
            {
                let end = active.end;
                out.push_str(&paint(&status[..active.start], style));
                out.push_str(&paint(
                    &status[active.start..end],
                    style.map(|style| style.bold()),
                ));
                out.push_str(&paint(&status[end..], style));
            } else {
                out.push_str(&paint(&status, style));
            }
        }
        out
    }

    fn complete_hint(&self) -> String {
        self.inline.clone()
    }

    fn next_hint_token(&self) -> String {
        let word = self.inline.trim_start();
        let lead = self.inline.len() - word.len();
        let end = word.find(char::is_whitespace).unwrap_or(word.len());
        self.inline[..lead + end].to_string()
    }
}

/// Candidates that extend the word before the cursor as typed.
fn extending<'a>(
    line: &'a str,
    candidates: &'a [Suggestion],
) -> impl Iterator<Item = &'a Suggestion> {
    candidates.iter().filter(move |candidate| {
        let typed = line
            .get(candidate.span.start..candidate.span.end)
            .unwrap_or_default();
        candidate.span.end == line.len()
            && candidate.value.len() > typed.len()
            && candidate.value.starts_with(typed)
    })
}

fn only_candidate<'a>(line: &'a str, candidates: &'a [Suggestion]) -> Option<&'a Suggestion> {
    let mut extending = extending(line, candidates);
    let first = extending.next()?;
    extending
        .all(|other| other.value == first.value)
        .then_some(first)
}

fn only_extension(line: &str, candidates: &[Suggestion]) -> Option<String> {
    let candidate = only_candidate(line, candidates)?;
    let typed = candidate.span.end - candidate.span.start;
    Some(candidate.value[typed..].to_string())
}

/// Keeps the status on one terminal row; a wrapped status would push the
/// prompt around as it changes length.
fn fit_terminal_width(text: &str) -> String {
    let width = terminal_size::terminal_size()
        .map(|(width, _)| usize::from(width.0))
        .unwrap_or(80)
        .saturating_sub(1);
    let mut used = 0;
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take_while(|ch| {
            used += ch.width().unwrap_or(0);
            used <= width
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ReplHinter, only_extension};
    use crate::completion::{ArgNode, CompletionNode, CompletionTree, FlagNode, SuggestionEntry};
    use crate::repl::engine::adapter::ReplCompleter;
    use crate::repl::engine::editor::PaintedLine;
    use crate::repl::highlight::ReplHighlighter;
    use nu_ansi_term::{Color, Style};
    use reedline::{FileBackedHistory, Hinter, Span, Suggestion};

    fn tree() -> CompletionTree {
        let create = CompletionNode {
            tooltip: Some("Create a VM".to_string()),
            ..CompletionNode::default()
        }
        .with_flag("--name", FlagNode::new().tooltip("VM name"));
        let vm = CompletionNode::default()
            .with_child("create", create)
            .with_child("find", CompletionNode::default())
            .with_child(
                "power",
                CompletionNode {
                    tooltip: Some("Control VM power".to_string()),
                    args: vec![
                        ArgNode {
                            required: true,
                            ..ArgNode::named("HOSTNAME")
                        },
                        ArgNode::named("OPERATION").suggestions([
                            SuggestionEntry::value("on"),
                            SuggestionEntry::value("off"),
                        ]),
                        ArgNode::named("EXTRA").multi(),
                    ],
                    ..CompletionNode::default()
                },
            );
        CompletionTree {
            root: CompletionNode::default()
                .with_child("orch", CompletionNode::default().with_child("vm", vm)),
            ..CompletionTree::default()
        }
    }

    fn hinter() -> ReplHinter {
        ReplHinter::new(
            ReplCompleter::new(tree(), None),
            ReplHighlighter::new(tree(), Color::Green, Color::Red, None),
            PaintedLine::default(),
            Some(Style::new()),
            Style::new(),
        )
    }

    fn paint(line: &str) -> (String, String) {
        let mut hinter = hinter();
        let history = FileBackedHistory::new(10).expect("history");
        let out = hinter.handle(line, line.len(), &history, true, "");
        (hinter.complete_hint(), out)
    }

    #[test]
    fn hint_completes_the_only_candidate_and_names_it_unit() {
        let (inline, out) = paint("orch vm cr");
        assert_eq!(inline, "eate");
        assert_eq!(out, "eate\r\ncreate  Create a VM");
        for (line, expected, slot) in [
            (
                "orch vm power ",
                "\r\norch vm power <HOSTNAME> [OPERATION] [EXTRA]… · Control VM power",
                "<HOSTNAME>",
            ),
            (
                "orch vm power web01 ",
                "\r\norch vm power HOSTNAME [OPERATION] [EXTRA]… · on | off",
                "[OPERATION]",
            ),
            (
                "orch vm power web01 on ",
                "\r\norch vm power HOSTNAME OPERATION [EXTRA]… · Control VM power",
                "[EXTRA]…",
            ),
        ] {
            let (_, out) = paint(line);
            assert_eq!(
                out,
                expected.replace(slot, &Style::new().bold().paint(slot).to_string())
            );
        }
    }

    #[test]
    fn hint_reports_unknown_words_with_the_closest_match_unit() {
        let (inline, out) = paint("orch vm fnd ");
        assert_eq!(inline, "");
        assert_eq!(
            out,
            "\r\nunknown command 'fnd' for 'orch vm'; did you mean 'find'?"
        );
    }

    #[test]
    fn hint_describes_the_flag_being_given_a_value_unit() {
        let (_, out) = paint("orch vm create --name ");
        assert_eq!(out, "\r\n--name  VM name");
    }

    #[test]
    fn only_extension_ignores_candidates_that_do_not_extend_the_word_unit() {
        let span = Span { start: 0, end: 2 };
        let candidate = |value: &str| Suggestion {
            value: value.to_string(),
            span,
            ..Suggestion::default()
        };
        assert_eq!(
            only_extension("ld", &[candidate("ldap"), candidate("old")]),
            Some("ap".to_string())
        );
        assert_eq!(
            only_extension("ld", &[candidate("ldap"), candidate("ldif")]),
            None
        );
    }
}
