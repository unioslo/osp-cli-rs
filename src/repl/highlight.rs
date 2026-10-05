use std::collections::BTreeSet;
use std::sync::Arc;

use crate::completion::{CommandLineParser, CompletionNode, CompletionTree, TokenSpan};
use nu_ansi_term::Color;
use reedline::{Highlighter, StyledText};
use serde::Serialize;

use crate::repl::LineProjection;

/// Highlighting intentionally stays small and opinionated:
/// - color only the visible command path the tree can resolve
/// - mark the first word the tree proves wrong, never one still being typed
/// - keep partial tokens, values, and known flags plain
/// - preserve `help <command>` as a first-class alias case
/// - self-highlight hex color literals
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HighlightTokenKind {
    Plain,
    CommandValid,
    Invalid,
    ColorLiteral(Color),
}

/// The first word on a line that cannot be valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineProblem {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: LineProblemKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LineProblemKind {
    /// Not a subcommand of `parent`, which takes no free positionals.
    UnknownCommand { parent: Vec<String> },
    /// Not a flag of the command path before it.
    UnknownFlag { command: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightedSpan {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightTokenKind,
}

/// Debug-friendly view of one highlighted span in the REPL line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HighlightDebugSpan {
    /// Byte offset where the span starts.
    pub start: usize,
    /// Byte offset where the span ends.
    pub end: usize,
    /// Raw text contained in the span.
    pub text: String,
    /// Stable debug classification name.
    pub kind: String,
    /// RGB color used for the span, when highlighting applied a literal color.
    pub rgb: Option<[u8; 3]>,
}

pub(crate) type LineProjector = Arc<dyn Fn(&str) -> LineProjection + Send + Sync>;

pub(crate) struct ReplHighlighter {
    tree: CompletionTree,
    parser: CommandLineParser,
    command_color: Color,
    error_color: Color,
    line_projector: Option<LineProjector>,
}

impl ReplHighlighter {
    pub(crate) fn new(
        tree: CompletionTree,
        command_color: Color,
        error_color: Color,
        line_projector: Option<LineProjector>,
    ) -> Self {
        Self {
            tree,
            parser: CommandLineParser,
            command_color,
            error_color,
            line_projector,
        }
    }

    fn walk(&self, line: &str, cursor: usize) -> LineWalk {
        let projected = self
            .line_projector
            .as_ref()
            .map(|project| project(line))
            .unwrap_or_else(|| LineProjection::passthrough(line));
        walk_line(&self.tree.root, &self.parser, &projected.line, cursor)
    }

    /// Returns the first word on `line` that cannot be valid, if any.
    pub(crate) fn problem(&self, line: &str, cursor: usize) -> Option<LineProblem> {
        self.walk(line, cursor).problem
    }

    pub(crate) fn classify(&self, line: &str, cursor: usize) -> Vec<HighlightedSpan> {
        if line.is_empty() {
            return Vec::new();
        }

        let raw_spans = self.parser.tokenize_with_spans(line);
        if raw_spans.is_empty() {
            return Vec::new();
        }

        let projected = self
            .line_projector
            .as_ref()
            .map(|project| project(line))
            .unwrap_or_else(|| LineProjection::passthrough(line));
        let LineWalk {
            mut commands,
            problem,
        } = self.walk(line, cursor);
        if let Some(range) = blanked_help_keyword_range(&raw_spans, &projected.line) {
            commands.insert(range);
        }
        let invalid = problem.map(|problem| (problem.start, problem.end));

        raw_spans
            .into_iter()
            .map(|span| HighlightedSpan {
                start: span.start,
                end: span.end,
                kind: if commands.contains(&(span.start, span.end)) {
                    HighlightTokenKind::CommandValid
                } else if invalid == Some((span.start, span.end)) {
                    HighlightTokenKind::Invalid
                } else if let Some(color) = parse_hex_color_token(&span.value) {
                    HighlightTokenKind::ColorLiteral(color)
                } else {
                    HighlightTokenKind::Plain
                },
            })
            .collect()
    }

    fn classify_debug(&self, line: &str) -> Vec<HighlightDebugSpan> {
        self.classify(line, line.len())
            .into_iter()
            .map(|span| HighlightDebugSpan {
                start: span.start,
                end: span.end,
                text: line[span.start..span.end].to_string(),
                kind: debug_kind_name(span.kind).to_string(),
                rgb: debug_kind_rgb(span.kind),
            })
            .collect()
    }
}

impl Highlighter for ReplHighlighter {
    fn highlight(&self, line: &str, cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        if line.is_empty() {
            return styled;
        }

        let spans = self.classify(line, cursor);
        if spans.is_empty() {
            styled.push((nu_ansi_term::Style::new(), line.to_string()));
            return styled;
        }

        let mut pos = 0usize;
        for span in spans {
            if span.start > pos {
                styled.push((
                    nu_ansi_term::Style::new(),
                    line[pos..span.start].to_string(),
                ));
            }

            let style = match span.kind {
                HighlightTokenKind::Plain => nu_ansi_term::Style::new(),
                HighlightTokenKind::CommandValid => {
                    nu_ansi_term::Style::new().fg(self.command_color)
                }
                HighlightTokenKind::Invalid => nu_ansi_term::Style::new().fg(self.error_color),
                HighlightTokenKind::ColorLiteral(color) => nu_ansi_term::Style::new().fg(color),
            };
            styled.push((style, line[span.start..span.end].to_string()));
            pos = span.end;
        }

        if pos < line.len() {
            styled.push((nu_ansi_term::Style::new(), line[pos..].to_string()));
        }

        styled
    }
}

/// Classifies a REPL line and returns serializable highlight spans for debugging tools.
pub fn debug_highlight(
    tree: &CompletionTree,
    line: &str,
    command_color: Color,
    line_projector: Option<LineProjector>,
) -> Vec<HighlightDebugSpan> {
    ReplHighlighter::new(tree.clone(), command_color, Color::Red, line_projector)
        .classify_debug(line)
}

/// Command words to color and the first word that cannot be valid.
#[derive(Default)]
struct LineWalk {
    commands: BTreeSet<(usize, usize)>,
    problem: Option<LineProblem>,
}

/// Walks the command path, then checks flags against the commands they
/// follow.
///
/// A word is only wrong when the tree is sure: an unknown subcommand under a
/// command that takes no free positionals, or a flag no command on the path
/// declares. A word at the cursor that still prefixes a valid choice is being
/// typed and is left alone. Pipe stages are the DSL's business.
fn walk_line(
    root: &CompletionNode,
    parser: &CommandLineParser,
    projected_line: &str,
    cursor: usize,
) -> LineWalk {
    let mut walk = LineWalk::default();
    let mut path = Vec::new();
    let mut scopes = vec![root];
    let mut node = root;
    let mut in_path = true;
    let mut skip_value = false;

    for span in parser.tokenize_with_spans(projected_line) {
        let token = span.value.as_str();
        if token == "|" {
            break;
        }
        if token.is_empty() || std::mem::take(&mut skip_value) {
            continue;
        }
        let typing = span.end == cursor;

        if token == "--" {
            break;
        }
        if token.starts_with('-') && token != "-" {
            let (name, inline_value) = match token.split_once('=') {
                Some((name, _)) => (name, true),
                None => (token, false),
            };
            match scopes.iter().rev().find_map(|scope| scope.flags.get(name)) {
                Some(flag) => skip_value = !flag.flag_only && !inline_value,
                None => {
                    let prefixes_a_flag = scopes
                        .iter()
                        .any(|scope| scope.flags.keys().any(|known| known.starts_with(name)));
                    if !(typing && prefixes_a_flag) {
                        walk.problem = Some(LineProblem {
                            start: span.start,
                            end: span.end,
                            kind: LineProblemKind::UnknownFlag {
                                command: path.clone(),
                            },
                        });
                    }
                    // Without the flag's arity the rest of the line is guesswork.
                    break;
                }
            }
            continue;
        }

        if !in_path {
            continue;
        }
        if let Some(child) = node.children.get(token) {
            walk.commands.insert((span.start, span.end));
            path.push(token.to_string());
            scopes.push(child);
            node = child;
            continue;
        }
        in_path = false;

        // The root's positional is the synthetic command list, so any word
        // there must name a command; elsewhere positionals are free values.
        let takes_values = !path.is_empty() && !node.args.is_empty();
        let names_root_word = node
            .args
            .iter()
            .any(|arg| arg.suggestions.iter().any(|entry| entry.value == token));
        if node.children.is_empty() || node.value_key || takes_values || names_root_word {
            continue;
        }
        let prefixes_a_command = node.children.keys().any(|name| name.starts_with(token));
        if !(typing && prefixes_a_command) {
            walk.problem = Some(LineProblem {
                start: span.start,
                end: span.end,
                kind: LineProblemKind::UnknownCommand {
                    parent: path.clone(),
                },
            });
            break;
        }
    }

    walk
}

// `help <command>` is projected to a blanked keyword plus the target path.
// Preserve highlighting for the hidden keyword itself when that projection applies.
fn blanked_help_keyword_range(
    raw_spans: &[TokenSpan],
    projected_line: &str,
) -> Option<(usize, usize)> {
    raw_spans
        .iter()
        .find(|span| {
            span.value == "help"
                && projected_line
                    .get(span.start..span.end)
                    .is_some_and(|segment| segment.trim().is_empty())
        })
        .map(|span| (span.start, span.end))
}

fn parse_hex_color_token(token: &str) -> Option<Color> {
    let normalized = token.trim();
    let hex = normalized.strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    if hex.len() == 3 {
        let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
        let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
        let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
        return Some(Color::Rgb(
            r.saturating_mul(17),
            g.saturating_mul(17),
            b.saturating_mul(17),
        ));
    }
    None
}

fn debug_kind_name(kind: HighlightTokenKind) -> &'static str {
    match kind {
        HighlightTokenKind::Plain => "plain",
        HighlightTokenKind::CommandValid => "command_valid",
        HighlightTokenKind::Invalid => "invalid",
        HighlightTokenKind::ColorLiteral(_) => "color_literal",
    }
}

fn debug_kind_rgb(kind: HighlightTokenKind) -> Option<[u8; 3]> {
    let color = match kind {
        HighlightTokenKind::ColorLiteral(color) => color,
        _ => return None,
    };

    let rgb = match color {
        Color::Black => [0, 0, 0],
        Color::DarkGray => [128, 128, 128],
        Color::Red => [128, 0, 0],
        Color::Green => [0, 128, 0],
        Color::Yellow => [128, 128, 0],
        Color::Blue => [0, 0, 128],
        Color::Purple => [128, 0, 128],
        Color::Magenta => [128, 0, 128],
        Color::Cyan => [0, 128, 128],
        Color::White => [192, 192, 192],
        Color::Fixed(_) => return None,
        Color::LightRed => [255, 0, 0],
        Color::LightGreen => [0, 255, 0],
        Color::LightYellow => [255, 255, 0],
        Color::LightBlue => [0, 0, 255],
        Color::LightPurple => [255, 0, 255],
        Color::LightMagenta => [255, 0, 255],
        Color::LightCyan => [0, 255, 255],
        Color::LightGray => [255, 255, 255],
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Default => return None,
    };
    Some(rgb)
}

#[cfg(test)]
mod tests {
    use super::{ReplHighlighter, debug_highlight};
    use crate::completion::{CompletionNode, CompletionTree};
    use crate::repl::LineProjection;
    use nu_ansi_term::Color;
    use reedline::Highlighter;
    use std::sync::Arc;

    fn token_styles(styled: &StyledText) -> Vec<(String, Option<Color>)> {
        styled
            .buffer
            .iter()
            .filter_map(|(style, text)| {
                if text.chars().all(|ch| ch.is_whitespace()) {
                    None
                } else {
                    Some((text.clone(), style.foreground))
                }
            })
            .collect()
    }

    use reedline::StyledText;

    fn completion_tree_with_config_show() -> CompletionTree {
        let mut config = CompletionNode::default();
        config
            .children
            .insert("show".to_string(), CompletionNode::default());
        CompletionTree {
            root: CompletionNode::default().with_child("config", config),
            ..CompletionTree::default()
        }
    }

    #[test]
    fn colors_full_command_chain_only_unit() {
        let tree = completion_tree_with_config_show();
        let highlighter = ReplHighlighter::new(tree, Color::Green, Color::Red, None);

        let tokens = token_styles(&highlighter.highlight("config show", 0));
        assert_eq!(
            tokens,
            vec![
                ("config".to_string(), Some(Color::Green)),
                ("show".to_string(), Some(Color::Green)),
            ]
        );
    }

    #[test]
    fn marks_wrong_words_but_not_words_being_typed_unit() {
        let tree = completion_tree_with_config_show();
        let highlighter = ReplHighlighter::new(tree, Color::Green, Color::Red, None);
        let styles = |line: &str, cursor: usize| token_styles(&highlighter.highlight(line, cursor));

        assert_eq!(
            styles("config sho", 10),
            vec![
                ("config".to_string(), Some(Color::Green)),
                ("sho".to_string(), None),
            ]
        );
        assert_eq!(
            styles("config sho ", 11),
            vec![
                ("config".to_string(), Some(Color::Green)),
                ("sho".to_string(), Some(Color::Red)),
            ]
        );
        assert_eq!(
            styles("config --flag", 13),
            vec![
                ("config".to_string(), Some(Color::Green)),
                ("--flag".to_string(), Some(Color::Red)),
            ]
        );
    }

    #[test]
    fn colors_help_alias_keyword_and_target_unit() {
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("history", CompletionNode::default()),
            ..CompletionTree::default()
        };
        let projector =
            Arc::new(|line: &str| LineProjection::passthrough(line.replacen("help", "    ", 1)));
        let highlighter = ReplHighlighter::new(tree, Color::Green, Color::Red, Some(projector));

        let tokens = token_styles(&highlighter.highlight("help history", 0));
        assert_eq!(
            tokens,
            vec![
                ("help".to_string(), Some(Color::Green)),
                ("history".to_string(), Some(Color::Green)),
            ]
        );

        let tokens = token_styles(&highlighter.highlight("help his", 8));
        assert_eq!(
            tokens,
            vec![
                ("help".to_string(), Some(Color::Green)),
                ("his".to_string(), None),
            ]
        );
    }

    #[test]
    fn highlights_hex_color_literals_unit() {
        let highlighter =
            ReplHighlighter::new(CompletionTree::default(), Color::Green, Color::Red, None);
        let spans = debug_highlight(&CompletionTree::default(), "#ff00cc", Color::Green, None);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].kind, "color_literal");
        assert_eq!(spans[0].rgb, Some([255, 0, 204]));
        let tokens = token_styles(&highlighter.highlight("#ff00cc", 0));
        assert_eq!(
            tokens,
            vec![("#ff00cc".to_string(), Some(Color::Rgb(255, 0, 204)))]
        );
    }

    #[test]
    fn debug_spans_preserve_help_alias_ranges_unit() {
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("history", CompletionNode::default()),
            ..CompletionTree::default()
        };
        let projector =
            Arc::new(|line: &str| LineProjection::passthrough(line.replacen("help", "    ", 1)));
        let spans = debug_highlight(&tree, "help history -", Color::Green, Some(projector));

        assert_eq!(
            spans
                .into_iter()
                .filter(|span| span.kind == "command_valid")
                .map(|span| (span.start, span.end, span.text))
                .collect::<Vec<_>>(),
            vec![(0, 4, "help".to_string()), (5, 12, "history".to_string())]
        );
    }

    #[test]
    fn three_digit_hex_and_invalid_tokens_cover_debug_paths_unit() {
        let spans = debug_highlight(&CompletionTree::default(), "#0af", Color::Green, None);
        assert_eq!(spans[0].rgb, Some([0, 170, 255]));

        let highlighter =
            ReplHighlighter::new(CompletionTree::default(), Color::Green, Color::Red, None);
        let tokens = token_styles(&highlighter.highlight("unknown #nope", 0));
        assert_eq!(
            tokens,
            vec![("unknown".to_string(), None), ("#nope".to_string(), None),]
        );
    }
}
