//! Shell-like tokenization and cursor analysis for completion.
//!
//! This module exists to turn a partially typed input line plus a cursor offset
//! into the structured data the completion engine actually needs: command path,
//! tail items, pipe mode, and the active replacement span.
//!
//! Contract:
//!
//! - parsing here stays permissive for interactive use
//! - the parser owns lexical structure, not suggestion ranking
//! - callers should rely on `ParsedCursorLine` and `CursorState` rather than
//!   re-deriving cursor spans themselves

use crate::completion::model::{CommandLine, CursorState, FlagOccurrence, ParsedLine, QuoteStyle};
use std::collections::BTreeMap;

/// Token value with byte offsets into the original input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSpan {
    /// Unescaped token text.
    pub value: String,
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexState {
    Normal,
    SingleQuote,
    DoubleQuote,
    EscapeNormal,
    EscapeDouble,
}

/// Parsed line assembly after tokenization.
///
/// The parser keeps the command head separate until it sees the first option-like
/// token. After that point the rest of the line is interpreted as flags, args,
/// or pipes. That mirrors how the completer reasons about scope: command path
/// first, then option/value mode.
#[derive(Debug, Default)]
struct ParseState {
    head: Vec<String>,
    tail: Vec<crate::completion::model::TailItem>,
    flag_values: BTreeMap<String, Vec<String>>,
    pipes: Vec<String>,
    has_pipe: bool,
}

impl ParseState {
    fn finish(self) -> CommandLine {
        CommandLine {
            head: self.head,
            tail: self.tail,
            flag_values: self.flag_values,
            pipes: self.pipes,
            has_pipe: self.has_pipe,
        }
    }

    fn start_pipe<'a>(&mut self, iter: &mut std::iter::Peekable<std::slice::Iter<'a, String>>) {
        self.has_pipe = true;
        self.pipes.extend(iter.cloned());
    }

    fn collect_positional_tail<'a>(
        &mut self,
        iter: &mut std::iter::Peekable<std::slice::Iter<'a, String>>,
    ) {
        while let Some(next) = iter.next() {
            if next == "|" {
                self.start_pipe(iter);
                break;
            }
            self.tail
                .push(crate::completion::model::TailItem::Positional(next.clone()));
        }
    }

    fn parse_flag_tail<'a>(
        &mut self,
        first_token: String,
        iter: &mut std::iter::Peekable<std::slice::Iter<'a, String>>,
    ) {
        // Once the parser has seen the first flag-like token, the rest of the
        // line stays in "tail mode". From that point on we only distinguish
        // between more flags, their values, `--`, and a pipe into DSL mode.
        let mut current = first_token;
        loop {
            if current == "|" {
                self.start_pipe(iter);
                return;
            }

            if current == "--" {
                self.collect_positional_tail(iter);
                return;
            }

            if let Some((flag, value)) = split_inline_flag_value(&current) {
                let mut occurrence_values = Vec::new();
                if !value.is_empty() {
                    self.flag_values
                        .entry(flag.clone())
                        .or_default()
                        .push(value.clone());
                    occurrence_values.push(value);
                } else {
                    self.flag_values.entry(flag.clone()).or_default();
                }
                self.tail
                    .push(crate::completion::model::TailItem::Flag(FlagOccurrence {
                        name: flag.clone(),
                        values: occurrence_values,
                    }));
                let Some(next) = iter.next().cloned() else {
                    break;
                };
                current = next;
                continue;
            }

            let flag = current;
            let values = self.consume_flag_values(iter);
            self.tail
                .push(crate::completion::model::TailItem::Flag(FlagOccurrence {
                    name: flag.clone(),
                    values: values.clone(),
                }));
            self.flag_values
                .entry(flag.clone())
                .or_default()
                .extend(values);

            let Some(next) = iter.next().cloned() else {
                break;
            };
            current = next;
        }
    }

    fn consume_flag_values<'a>(
        &mut self,
        iter: &mut std::iter::Peekable<std::slice::Iter<'a, String>>,
    ) -> Vec<String> {
        let mut values = Vec::new();

        while let Some(next) = iter.peek() {
            if *next == "|" || *next == "--" {
                break;
            }
            if looks_like_flag_start(next) {
                break;
            }

            values.push((*next).clone());
            iter.next();
        }

        values
    }
}

/// Shell-like parser used by the completion engine.
#[derive(Debug, Clone, Default)]
pub struct CommandLineParser;

/// Parsed command-line state for the full line and the cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCursorLine {
    /// Parsed tokens and command structure.
    pub parsed: ParsedLine,
    /// Cursor-local replacement information.
    pub cursor: CursorState,
}

#[derive(Debug, Clone)]
struct CursorTokenization {
    full_tokens: Vec<String>,
    cursor_tokens: Vec<String>,
    cursor_quote_style: Option<QuoteStyle>,
}

impl CommandLineParser {
    /// Tokenizes a line using shell-like quoting rules.
    ///
    /// Tokenization is intentionally permissive for interactive use. If the
    /// user is mid-quote while pressing tab, we retry with a synthetic closing
    /// quote before finally falling back to whitespace splitting.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::CommandLineParser;
    ///
    /// let parser = CommandLineParser;
    ///
    /// assert_eq!(
    ///     parser.tokenize(r#"ldap user "alice smith""#),
    ///     vec!["ldap", "user", "alice smith"]
    /// );
    /// ```
    pub fn tokenize(&self, line: &str) -> Vec<String> {
        self.tokenize_inner(line)
            .or_else(|| self.tokenize_inner(&format!("{line}\"")))
            .or_else(|| self.tokenize_inner(&format!("{line}'")))
            .unwrap_or_else(|| line.split_whitespace().map(str::to_string).collect())
    }

    /// Tokenizes `line` and preserves byte spans for each token when possible.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::CommandLineParser;
    ///
    /// let spans = CommandLineParser.tokenize_with_spans("ldap user alice");
    ///
    /// assert_eq!(spans[0].value, "ldap");
    /// assert_eq!(spans[1].start, 5);
    /// assert_eq!(spans[2].end, 15);
    /// ```
    pub fn tokenize_with_spans(&self, line: &str) -> Vec<TokenSpan> {
        self.tokenize_with_spans_inner(line)
            .or_else(|| self.tokenize_with_spans_fallback(line))
            .unwrap_or_default()
    }

    /// Parse the full line and the cursor-local prefix from one lexical walk.
    ///
    /// The common case keeps completion analysis in one tokenization pass. If
    /// the line ends in an unmatched quote we fall back to the permissive
    /// tokenization path so interactive behavior stays unchanged.
    ///
    /// `cursor` is clamped to the input length and to a valid UTF-8 character
    /// boundary before the parser slices the line.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::CommandLineParser;
    ///
    /// let parsed = CommandLineParser.analyze("ldap user ali", 13);
    ///
    /// assert_eq!(parsed.parsed.cursor_tokens, vec!["ldap", "user", "ali"]);
    /// assert_eq!(parsed.cursor.token_stub, "ali");
    /// ```
    pub fn analyze(&self, line: &str, cursor: usize) -> ParsedCursorLine {
        let safe_cursor = clamp_to_char_boundary(line, cursor.min(line.len()));
        let before_cursor = &line[..safe_cursor];
        let lexical = self.lex_cursor_line(line, before_cursor, safe_cursor);
        self.assemble_parsed_cursor_line(before_cursor, safe_cursor, lexical)
    }

    fn tokenize_inner(&self, line: &str) -> Option<Vec<String>> {
        let mut out = Vec::new();
        let mut state = LexState::Normal;
        let mut current = String::new();

        for ch in line.chars() {
            match state {
                LexState::Normal => {
                    if ch.is_whitespace() {
                        push_current(&mut out, &mut current);
                    } else {
                        match ch {
                            '|' => {
                                push_current(&mut out, &mut current);
                                out.push("|".to_string());
                            }
                            '\\' => state = LexState::EscapeNormal,
                            '\'' => state = LexState::SingleQuote,
                            '"' => state = LexState::DoubleQuote,
                            _ => current.push(ch),
                        }
                    }
                }
                LexState::SingleQuote => {
                    if ch == '\'' {
                        state = LexState::Normal;
                    } else {
                        current.push(ch);
                    }
                }
                LexState::DoubleQuote => match ch {
                    '"' => state = LexState::Normal,
                    '\\' => state = LexState::EscapeDouble,
                    _ => current.push(ch),
                },
                LexState::EscapeNormal => {
                    current.push(ch);
                    state = LexState::Normal;
                }
                LexState::EscapeDouble => {
                    current.push(ch);
                    state = LexState::DoubleQuote;
                }
            }
        }

        match state {
            LexState::Normal => {
                push_current(&mut out, &mut current);
                Some(out)
            }
            _ => None,
        }
    }

    fn tokenize_with_spans_inner(&self, line: &str) -> Option<Vec<TokenSpan>> {
        let mut out = Vec::new();
        let mut state = LexState::Normal;
        let mut current = String::new();
        let mut current_start = None;

        for (idx, ch) in line.char_indices() {
            match state {
                LexState::Normal => {
                    if ch.is_whitespace() {
                        push_current_span(&mut out, &mut current, &mut current_start, idx);
                    } else {
                        match ch {
                            '|' => {
                                push_current_span(&mut out, &mut current, &mut current_start, idx);
                                out.push(TokenSpan {
                                    value: "|".to_string(),
                                    start: idx,
                                    end: idx + ch.len_utf8(),
                                });
                            }
                            '\\' => {
                                current_start.get_or_insert(idx);
                                state = LexState::EscapeNormal;
                            }
                            '\'' => {
                                current_start.get_or_insert(idx);
                                state = LexState::SingleQuote;
                            }
                            '"' => {
                                current_start.get_or_insert(idx);
                                state = LexState::DoubleQuote;
                            }
                            _ => {
                                current_start.get_or_insert(idx);
                                current.push(ch);
                            }
                        }
                    }
                }
                LexState::SingleQuote => {
                    if ch == '\'' {
                        state = LexState::Normal;
                    } else {
                        current.push(ch);
                    }
                }
                LexState::DoubleQuote => match ch {
                    '"' => state = LexState::Normal,
                    '\\' => state = LexState::EscapeDouble,
                    _ => current.push(ch),
                },
                LexState::EscapeNormal => {
                    current.push(ch);
                    state = LexState::Normal;
                }
                LexState::EscapeDouble => {
                    current.push(ch);
                    state = LexState::DoubleQuote;
                }
            }
        }

        match state {
            LexState::Normal => {
                push_current_span(&mut out, &mut current, &mut current_start, line.len());
                Some(out)
            }
            _ => None,
        }
    }

    fn tokenize_with_spans_fallback(&self, line: &str) -> Option<Vec<TokenSpan>> {
        let mut out = Vec::new();
        let mut search_from = 0usize;
        for token in line.split_whitespace() {
            let rel = line.get(search_from..)?.find(token)?;
            let start = search_from + rel;
            let end = start + token.len();
            out.push(TokenSpan {
                value: token.to_string(),
                start,
                end,
            });
            search_from = end;
        }
        Some(out)
    }

    /// Parses tokens into command-path, flag, positional, and pipe segments.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::CommandLineParser;
    ///
    /// let tokens = vec![
    ///     "ldap".to_string(),
    ///     "user".to_string(),
    ///     "--json".to_string(),
    ///     "|".to_string(),
    ///     "P".to_string(),
    /// ];
    /// let parsed = CommandLineParser.parse(&tokens);
    ///
    /// assert_eq!(parsed.head(), &["ldap".to_string(), "user".to_string()]);
    /// assert!(parsed.has_pipe());
    /// ```
    pub fn parse(&self, tokens: &[String]) -> CommandLine {
        let mut state = ParseState::default();
        let mut iter = tokens.iter().peekable();

        while let Some(token) = iter.next() {
            if token == "|" {
                state.start_pipe(&mut iter);
                return state.finish();
            }
            if token == "--" {
                state.collect_positional_tail(&mut iter);
                return state.finish();
            }
            if token.starts_with('-') {
                state.parse_flag_tail(token.clone(), &mut iter);
                return state.finish();
            }
            state.head.push(token.clone());
        }

        state.finish()
    }

    /// Computes the cursor replacement range and current token stub.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::CommandLineParser;
    ///
    /// let cursor = CommandLineParser.cursor_state("ldap user ali", 13);
    ///
    /// assert_eq!(cursor.token_stub, "ali");
    /// assert_eq!(cursor.replace_range, 10..13);
    /// ```
    pub fn cursor_state(&self, text_before_cursor: &str, safe_cursor: usize) -> CursorState {
        let tokens = self.tokenize(text_before_cursor);
        self.build_cursor_state(
            text_before_cursor,
            safe_cursor,
            &tokens,
            self.compute_stub_quote(text_before_cursor),
        )
    }

    fn build_cursor_state(
        &self,
        text_before_cursor: &str,
        safe_cursor: usize,
        tokens: &[String],
        quote_style: Option<QuoteStyle>,
    ) -> CursorState {
        let token_stub = self.compute_stub(text_before_cursor, tokens);
        let replace_start = token_replace_start(text_before_cursor, safe_cursor, quote_style);
        let raw_stub = text_before_cursor
            .get(replace_start..safe_cursor)
            .unwrap_or("")
            .to_string();

        CursorState::new(
            token_stub,
            raw_stub,
            replace_start..safe_cursor,
            quote_style,
        )
    }

    fn tokenize_with_cursor_inner(
        &self,
        line: &str,
        safe_cursor: usize,
    ) -> Option<CursorTokenization> {
        let mut out = Vec::new();
        let mut state = LexState::Normal;
        let mut current = String::new();
        let mut cursor_tokens = None;
        let mut cursor_quote_style = None;

        for (idx, ch) in line.char_indices() {
            if idx == safe_cursor && cursor_tokens.is_none() {
                cursor_tokens = Some(snapshot_tokens(&out, &current));
                cursor_quote_style = Some(quote_style_for_state(state));
            }

            match state {
                LexState::Normal => {
                    if ch.is_whitespace() {
                        push_current(&mut out, &mut current);
                    } else {
                        match ch {
                            '|' => {
                                push_current(&mut out, &mut current);
                                out.push("|".to_string());
                            }
                            '\\' => state = LexState::EscapeNormal,
                            '\'' => state = LexState::SingleQuote,
                            '"' => state = LexState::DoubleQuote,
                            _ => current.push(ch),
                        }
                    }
                }
                LexState::SingleQuote => {
                    if ch == '\'' {
                        state = LexState::Normal;
                    } else {
                        current.push(ch);
                    }
                }
                LexState::DoubleQuote => match ch {
                    '"' => state = LexState::Normal,
                    '\\' => state = LexState::EscapeDouble,
                    _ => current.push(ch),
                },
                LexState::EscapeNormal => {
                    current.push(ch);
                    state = LexState::Normal;
                }
                LexState::EscapeDouble => {
                    current.push(ch);
                    state = LexState::DoubleQuote;
                }
            }
        }

        if safe_cursor == line.len() && cursor_tokens.is_none() {
            cursor_tokens = Some(snapshot_tokens(&out, &current));
            cursor_quote_style = Some(quote_style_for_state(state));
        }

        match state {
            LexState::Normal => {
                push_current(&mut out, &mut current);
                Some(CursorTokenization {
                    full_tokens: out,
                    cursor_tokens: cursor_tokens.unwrap_or_default(),
                    cursor_quote_style: cursor_quote_style.unwrap_or(None),
                })
            }
            _ => None,
        }
    }

    fn compute_stub(&self, text_before_cursor: &str, tokens: &[String]) -> String {
        if text_before_cursor.is_empty() || text_before_cursor.ends_with(' ') {
            return String::new();
        }
        let Some(last) = tokens.last() else {
            return String::new();
        };

        if last.starts_with("--") && last.ends_with('=') && last.contains('=') {
            return String::new();
        }

        last.clone()
    }

    /// Returns the active quote style for the token being edited, if any.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{CommandLineParser, QuoteStyle};
    ///
    /// assert_eq!(
    ///     CommandLineParser.compute_stub_quote(r#"ldap user "ali"#),
    ///     Some(QuoteStyle::Double)
    /// );
    /// ```
    pub fn compute_stub_quote(&self, text_before_cursor: &str) -> Option<QuoteStyle> {
        current_quote_state(text_before_cursor)
    }

    fn lex_cursor_line(
        &self,
        line: &str,
        before_cursor: &str,
        safe_cursor: usize,
    ) -> CursorLexicalState {
        match self.tokenize_with_cursor_inner(line, safe_cursor) {
            Some(tokenized) => CursorLexicalState::Structured(tokenized),
            None => CursorLexicalState::Fallback {
                full_tokens: self.tokenize(line),
                cursor_tokens: self.tokenize(before_cursor),
            },
        }
    }

    fn assemble_parsed_cursor_line(
        &self,
        before_cursor: &str,
        safe_cursor: usize,
        lexical: CursorLexicalState,
    ) -> ParsedCursorLine {
        match lexical {
            CursorLexicalState::Structured(tokenized) => {
                let full_cmd = self.parse(&tokenized.full_tokens);
                let cursor_cmd = self.parse(&tokenized.cursor_tokens);
                let cursor = self.build_cursor_state(
                    before_cursor,
                    safe_cursor,
                    &tokenized.cursor_tokens,
                    tokenized.cursor_quote_style,
                );

                ParsedCursorLine {
                    parsed: ParsedLine {
                        safe_cursor,
                        full_tokens: tokenized.full_tokens,
                        cursor_tokens: tokenized.cursor_tokens,
                        full_cmd,
                        cursor_cmd,
                    },
                    cursor,
                }
            }
            CursorLexicalState::Fallback {
                full_tokens,
                cursor_tokens,
            } => {
                let full_cmd = self.parse(&full_tokens);
                let cursor_cmd = self.parse(&cursor_tokens);
                let cursor = self.cursor_state(before_cursor, safe_cursor);

                ParsedCursorLine {
                    parsed: ParsedLine {
                        safe_cursor,
                        full_tokens,
                        cursor_tokens,
                        full_cmd,
                        cursor_cmd,
                    },
                    cursor,
                }
            }
        }
    }
}

enum CursorLexicalState {
    Structured(CursorTokenization),
    Fallback {
        full_tokens: Vec<String>,
        cursor_tokens: Vec<String>,
    },
}

fn snapshot_tokens(out: &[String], current: &str) -> Vec<String> {
    let mut tokens = out.to_vec();
    if !current.is_empty() {
        tokens.push(current.to_string());
    }
    tokens
}

fn clamp_to_char_boundary(input: &str, cursor: usize) -> usize {
    if input.is_char_boundary(cursor) {
        return cursor;
    }
    let mut safe = cursor;
    while safe > 0 && !input.is_char_boundary(safe) {
        safe -= 1;
    }
    safe
}

fn quote_style_for_state(state: LexState) -> Option<QuoteStyle> {
    match state {
        LexState::SingleQuote => Some(QuoteStyle::Single),
        LexState::DoubleQuote | LexState::EscapeDouble => Some(QuoteStyle::Double),
        LexState::Normal | LexState::EscapeNormal => None,
    }
}

fn split_inline_flag_value(token: &str) -> Option<(String, String)> {
    if !token.starts_with("--") || !token.contains('=') {
        return None;
    }

    let mut split = token.splitn(2, '=');
    let flag = split.next().unwrap_or_default().to_string();
    let value = split.next().unwrap_or_default().to_string();
    Some((flag, value))
}

fn push_current(out: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        out.push(std::mem::take(current));
    }
}

fn push_current_span(
    out: &mut Vec<TokenSpan>,
    current: &mut String,
    current_start: &mut Option<usize>,
    end: usize,
) {
    if !current.is_empty() {
        out.push(TokenSpan {
            value: std::mem::take(current),
            start: current_start.take().unwrap_or(end),
            end,
        });
    } else {
        *current_start = None;
    }
}

fn looks_like_flag_start(token: &str) -> bool {
    token.starts_with('-') && token != "-" && !is_number(token)
}

fn is_number(text: &str) -> bool {
    text.parse::<f64>().is_ok()
}

fn current_quote_state(text: &str) -> Option<QuoteStyle> {
    let mut state = LexState::Normal;

    for ch in text.chars() {
        match state {
            LexState::Normal => match ch {
                '\\' => state = LexState::EscapeNormal,
                '\'' => state = LexState::SingleQuote,
                '"' => state = LexState::DoubleQuote,
                _ => {}
            },
            LexState::SingleQuote => {
                if ch == '\'' {
                    state = LexState::Normal;
                }
            }
            LexState::DoubleQuote => match ch {
                '"' => state = LexState::Normal,
                '\\' => state = LexState::EscapeDouble,
                _ => {}
            },
            LexState::EscapeNormal => state = LexState::Normal,
            LexState::EscapeDouble => state = LexState::DoubleQuote,
        }
    }

    match state {
        LexState::SingleQuote => Some(QuoteStyle::Single),
        LexState::DoubleQuote | LexState::EscapeDouble => Some(QuoteStyle::Double),
        LexState::Normal | LexState::EscapeNormal => None,
    }
}

fn token_replace_start(
    text_before_cursor: &str,
    safe_cursor: usize,
    quote_style: Option<QuoteStyle>,
) -> usize {
    if text_before_cursor.is_empty() || text_before_cursor.ends_with(' ') {
        return safe_cursor;
    }

    let mut state = LexState::Normal;
    let mut token_start = 0usize;
    let mut token_active = false;
    let mut quote_start = None;

    for (idx, ch) in text_before_cursor.char_indices() {
        match state {
            LexState::Normal => {
                if ch.is_whitespace() {
                    token_active = false;
                    token_start = idx + ch.len_utf8();
                    quote_start = None;
                    continue;
                }

                if !token_active {
                    token_active = true;
                    token_start = idx;
                }

                match ch {
                    '\'' => {
                        quote_start = Some(idx + ch.len_utf8());
                        state = LexState::SingleQuote;
                    }
                    '"' => {
                        quote_start = Some(idx + ch.len_utf8());
                        state = LexState::DoubleQuote;
                    }
                    '\\' => state = LexState::EscapeNormal,
                    _ => {}
                }
            }
            LexState::SingleQuote => {
                if ch == '\'' {
                    state = LexState::Normal;
                }
            }
            LexState::DoubleQuote => match ch {
                '"' => state = LexState::Normal,
                '\\' => state = LexState::EscapeDouble,
                _ => {}
            },
            LexState::EscapeNormal => state = LexState::Normal,
            LexState::EscapeDouble => state = LexState::DoubleQuote,
        }
    }

    match quote_style {
        Some(_) => quote_start.unwrap_or(token_start),
        None => token_start,
    }
}

#[cfg(test)]
mod tests {
    use crate::completion::model::{FlagOccurrence, QuoteStyle};

    use super::CommandLineParser;

    fn parser() -> CommandLineParser {
        CommandLineParser
    }

    mod scanner_contracts {
        use super::*;

        #[test]
        fn scanner_preserves_token_values_offsets_and_unmatched_quote_recovery() {
            let parser = parser();

            assert_eq!(
                parser.tokenize("orch provision --request 'name=a|b' | F name"),
                vec![
                    "orch",
                    "provision",
                    "--request",
                    "name=a|b",
                    "|",
                    "F",
                    "name",
                ]
            );
            assert_eq!(parser.tokenize("--os 'alma"), vec!["--os", "alma"]);

            let spans = parser.tokenize_with_spans("cmd --name 'alice");
            assert_eq!(spans.len(), 3);
            assert_eq!(spans[0].value, "cmd");
            assert_eq!(spans[1].value, "--name");
            assert_eq!(spans[2].value, "'alice");
            let source = r#"ldap user "alice smith" | P uid"#;
            let spans = parser.tokenize_with_spans(source);

            assert_eq!(spans[0].value, "ldap");
            assert_eq!(spans[0].start, 0);
            assert_eq!(spans[2].value, "alice smith");
            assert_eq!(&source[spans[2].start..spans[2].end], "\"alice smith\"");
            assert_eq!(spans[3].value, "|");
        }
    }

    mod command_shape_contracts {
        use super::*;

        #[test]
        fn parse_tracks_flag_values_pipes_and_repeated_occurrence_boundaries() {
            let parser = parser();

            let tokens = parser.tokenize("orch provision --provider vmware --os rhel | F name");
            let cmd = parser.parse(&tokens);
            assert_eq!(cmd.head(), ["orch".to_string(), "provision".to_string()]);
            assert_eq!(
                cmd.flag_values("--provider"),
                Some(&["vmware".to_string()][..])
            );
            assert_eq!(cmd.flag_values("--os"), Some(&["rhel".to_string()][..]));
            assert!(cmd.has_pipe());
            assert_eq!(cmd.pipes(), ["F".to_string(), "name".to_string()]);

            let repeated = parser.parse(&parser.tokenize("cmd --tag red --mode fast --tag blue"));
            assert_eq!(
                repeated.flag_occurrences().cloned().collect::<Vec<_>>(),
                vec![
                    FlagOccurrence {
                        name: "--tag".to_string(),
                        values: vec!["red".to_string()],
                    },
                    FlagOccurrence {
                        name: "--mode".to_string(),
                        values: vec!["fast".to_string()],
                    },
                    FlagOccurrence {
                        name: "--tag".to_string(),
                        values: vec!["blue".to_string()],
                    },
                ]
            );
        }

        #[test]
        fn parse_respects_option_boundaries_inline_values_and_negative_numbers() {
            let parser = parser();

            let after_double_dash = parser.parse(&parser.tokenize("cmd -- --not-a-flag"));
            assert_eq!(after_double_dash.head(), ["cmd".to_string()]);
            assert_eq!(
                after_double_dash
                    .positional_args()
                    .cloned()
                    .collect::<Vec<_>>(),
                vec!["--not-a-flag".to_string()]
            );

            let negative_value = parser.parse(&parser.tokenize("cmd --count -5"));
            assert_eq!(
                negative_value.flag_values("--count"),
                Some(&["-5".to_string()][..])
            );

            let inline = parser.parse(&parser.tokenize("cmd --format=json --os= --format=table"));
            assert_eq!(inline.flag_values("--os"), Some(&[][..]));
            assert_eq!(
                inline.flag_occurrences().cloned().collect::<Vec<_>>(),
                vec![
                    FlagOccurrence {
                        name: "--format".to_string(),
                        values: vec!["json".to_string()],
                    },
                    FlagOccurrence {
                        name: "--os".to_string(),
                        values: vec![],
                    },
                    FlagOccurrence {
                        name: "--format".to_string(),
                        values: vec!["table".to_string()],
                    },
                ]
            );
        }

        #[test]
        fn parse_distinguishes_tail_mode_from_dsl_boundaries() {
            let parser = parser();

            let tail =
                parser.parse(&parser.tokenize("ldap user --provider vmware region eu-central"));
            assert_eq!(tail.head(), ["ldap".to_string(), "user".to_string()]);
            assert_eq!(
                tail.flag_values("--provider"),
                Some(
                    &[
                        "vmware".to_string(),
                        "region".to_string(),
                        "eu-central".to_string(),
                    ][..]
                )
            );

            let dsl = parser.parse(&parser.tokenize("cmd -- literal | F name"));
            assert_eq!(dsl.head(), ["cmd".to_string()]);
            assert_eq!(
                dsl.positional_args().cloned().collect::<Vec<_>>(),
                vec!["literal".to_string()]
            );
            assert!(dsl.has_pipe());
            assert_eq!(dsl.pipes(), ["F".to_string(), "name".to_string()]);
        }
    }

    mod cursor_analysis_contracts {
        use super::*;

        #[test]
        fn cursor_state_tracks_equals_boundaries_and_open_quote_ranges() {
            let parser = parser();

            let cursor = parser.cursor_state("cmd --flag=", "cmd --flag=".len());
            assert_eq!(cursor.token_stub, "");

            assert_eq!(
                parser.compute_stub_quote("cmd --name \"al"),
                Some(QuoteStyle::Double)
            );
            assert_eq!(
                parser.compute_stub_quote("cmd --name 'al"),
                Some(QuoteStyle::Single)
            );
            assert_eq!(parser.compute_stub_quote("cmd --name al"), None);

            let line = "ldap user \"oi";
            let cursor = parser.cursor_state(line, line.len());
            assert_eq!(cursor.token_stub, "oi");
            assert_eq!(cursor.raw_stub, "oi");
            assert_eq!(cursor.replace_range, 11..13);
            assert_eq!(cursor.quote_style, Some(QuoteStyle::Double));
        }

        #[test]
        fn analyze_reuses_safe_cursor_snapshots_for_prefix_and_balanced_quotes() {
            let parser = parser();

            let line = "orch provision --provider vmware --os rhel | F name";
            let cursor = "orch provision --provider vmware".len();
            let analyzed = parser.analyze(line, cursor);
            assert_eq!(
                analyzed.parsed.full_tokens,
                vec![
                    "orch",
                    "provision",
                    "--provider",
                    "vmware",
                    "--os",
                    "rhel",
                    "|",
                    "F",
                    "name",
                ]
            );
            assert_eq!(
                analyzed.parsed.cursor_tokens,
                vec!["orch", "provision", "--provider", "vmware"]
            );
            assert_eq!(
                analyzed.parsed.cursor_cmd.flag_values("--provider"),
                Some(&["vmware".to_string()][..])
            );

            let balanced = parser.analyze(
                r#"ldap user "oi ste" --format json"#,
                r#"ldap user "oi"#.len(),
            );
            assert_eq!(balanced.cursor.token_stub, "oi");
            assert_eq!(balanced.cursor.raw_stub, "oi");
            assert_eq!(balanced.cursor.quote_style, Some(QuoteStyle::Double));
        }

        #[test]
        fn analyze_recovers_from_unbalanced_quotes_and_non_char_boundaries() {
            let parser = parser();

            let unbalanced = parser.analyze(r#"ldap user "alice"#, r#"ldap user "alice"#.len());
            assert_eq!(unbalanced.parsed.full_tokens, vec!["ldap", "user", "alice"]);
            assert_eq!(
                unbalanced.parsed.cursor_tokens,
                vec!["ldap", "user", "alice"]
            );
            assert_eq!(unbalanced.cursor.quote_style, Some(QuoteStyle::Double));
            assert_eq!(unbalanced.cursor.token_stub, "alice");

            let line = "ldap user å";
            let analyzed = parser.analyze(line, line.len() - 1);
            assert!(analyzed.parsed.safe_cursor < line.len());
            assert_eq!(analyzed.cursor.token_stub, "");
        }
    }
}
