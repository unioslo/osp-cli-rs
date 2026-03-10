use crate::completion::model::{CommandLine, CursorState, FlagOccurrence, ParsedLine, QuoteStyle};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Token value with byte offsets into the original input line.
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

#[derive(Debug, Clone, Default)]
/// Shell-like parser used by the completion engine.
pub struct CommandLineParser;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parsed command-line state for the full line and the cursor position.
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
    /// Tokenization is intentionally permissive for interactive use.
    ///
    /// If the user is mid-quote while pressing tab, we retry with a synthetic
    /// closing quote before finally falling back to whitespace splitting.
    pub fn tokenize(&self, line: &str) -> Vec<String> {
        self.tokenize_inner(line)
            .or_else(|| self.tokenize_inner(&format!("{line}\"")))
            .or_else(|| self.tokenize_inner(&format!("{line}'")))
            .unwrap_or_else(|| line.split_whitespace().map(str::to_string).collect())
    }

    /// Tokenizes `line` and preserves byte spans for each token when possible.
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
    pub fn analyze(&self, line: &str, cursor: usize) -> ParsedCursorLine {
        let safe_cursor = clamp_to_char_boundary(line, cursor.min(line.len()));
        let before_cursor = &line[..safe_cursor];

        if let Some(tokenized) = self.tokenize_with_cursor_inner(line, safe_cursor) {
            let full_cmd = self.parse(&tokenized.full_tokens);
            let cursor_cmd = self.parse(&tokenized.cursor_tokens);
            let cursor = self.build_cursor_state(
                before_cursor,
                safe_cursor,
                &tokenized.cursor_tokens,
                tokenized.cursor_quote_style,
            );

            return ParsedCursorLine {
                parsed: ParsedLine {
                    safe_cursor,
                    full_tokens: tokenized.full_tokens,
                    cursor_tokens: tokenized.cursor_tokens,
                    full_cmd,
                    cursor_cmd,
                },
                cursor,
            };
        }

        let full_tokens = self.tokenize(line);
        let cursor_tokens = self.tokenize(before_cursor);
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
    pub fn compute_stub_quote(&self, text_before_cursor: &str) -> Option<QuoteStyle> {
        current_quote_state(text_before_cursor)
    }
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

    #[test]
    fn tokenize_handles_pipes_and_quotes() {
        let parser = CommandLineParser;
        let tokens = parser.tokenize("orch provision --request 'name=a|b' | F name");
        assert_eq!(
            tokens,
            vec![
                "orch",
                "provision",
                "--request",
                "name=a|b",
                "|",
                "F",
                "name"
            ]
        );
    }

    #[test]
    fn tokenize_falls_back_for_unmatched_quotes() {
        let parser = CommandLineParser;
        let tokens = parser.tokenize("--os 'alma");
        assert_eq!(tokens, vec!["--os", "alma"]);
    }

    #[test]
    fn parse_handles_flags_and_pipes() {
        let parser = CommandLineParser;
        let tokens = parser.tokenize("orch provision --provider vmware --os rhel | F name");
        let cmd = parser.parse(&tokens);
        assert_eq!(cmd.head(), ["orch".to_string(), "provision".to_string()]);
        assert_eq!(
            cmd.flag_values("--provider"),
            Some(&["vmware".to_string()][..])
        );
        assert_eq!(cmd.flag_values("--os"), Some(&vec!["rhel".to_string()][..]));
        assert!(cmd.has_pipe());
        assert_eq!(cmd.pipes(), ["F".to_string(), "name".to_string()]);
    }

    #[test]
    fn parse_handles_end_of_options_and_negative_numbers() {
        let parser = CommandLineParser;

        let tokens = parser.tokenize("cmd -- --not-a-flag");
        let cmd = parser.parse(&tokens);
        assert_eq!(cmd.head(), ["cmd".to_string()]);
        assert_eq!(
            cmd.positional_args().cloned().collect::<Vec<_>>(),
            vec!["--not-a-flag".to_string()]
        );

        let tokens = parser.tokenize("cmd --count -5");
        let cmd = parser.parse(&tokens);
        assert_eq!(
            cmd.flag_values("--count"),
            Some(&vec!["-5".to_string()][..])
        );

        let tokens = parser.tokenize("cmd --os=");
        let cmd = parser.parse(&tokens);
        assert_eq!(cmd.flag_values("--os"), Some(&[][..]));
    }

    #[test]
    fn parse_preserves_repeated_flag_occurrence_boundaries() {
        let parser = CommandLineParser;
        let tokens = parser.tokenize("cmd --tag red --mode fast --tag blue");
        let cmd = parser.parse(&tokens);
        let occurrences = cmd.flag_occurrences().cloned().collect::<Vec<_>>();

        assert_eq!(occurrences.len(), 3);
        assert_eq!(occurrences[0].name, "--tag");
        assert_eq!(occurrences[0].values, vec!["red".to_string()]);
        assert_eq!(occurrences[1].name, "--mode");
        assert_eq!(occurrences[1].values, vec!["fast".to_string()]);
        assert_eq!(occurrences[2].name, "--tag");
        assert_eq!(occurrences[2].values, vec!["blue".to_string()]);
    }

    #[test]
    fn compute_stub_respects_equals_boundary() {
        let parser = CommandLineParser;
        let before = "cmd --flag=";
        let cursor = parser.cursor_state(before, before.len());
        assert_eq!(cursor.token_stub, "");
    }

    #[test]
    fn compute_stub_quote_tracks_unfinished_quotes() {
        let parser = CommandLineParser;
        assert_eq!(
            parser.compute_stub_quote("cmd --name \"al"),
            Some(QuoteStyle::Double)
        );
        assert_eq!(
            parser.compute_stub_quote("cmd --name 'al"),
            Some(QuoteStyle::Single)
        );
        assert_eq!(parser.compute_stub_quote("cmd --name al"), None);
    }

    #[test]
    fn cursor_state_tracks_replace_range_inside_open_quotes() {
        let parser = CommandLineParser;
        let line = "ldap user \"oi";
        let cursor = parser.cursor_state(line, line.len());

        assert_eq!(cursor.token_stub, "oi");
        assert_eq!(cursor.raw_stub, "oi");
        assert_eq!(cursor.replace_range, 11..13);
        assert_eq!(cursor.quote_style, Some(QuoteStyle::Double));
    }

    #[test]
    fn analyze_reuses_one_cursor_snapshot_for_full_and_prefix_parse() {
        let parser = CommandLineParser;
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
    }

    #[test]
    fn analyze_preserves_cursor_quote_state_inside_balanced_line() {
        let parser = CommandLineParser;
        let line = r#"ldap user "oi ste" --format json"#;
        let cursor = r#"ldap user "oi"#.len();
        let analyzed = parser.analyze(line, cursor);

        assert_eq!(analyzed.cursor.token_stub, "oi");
        assert_eq!(analyzed.cursor.raw_stub, "oi");
        assert_eq!(analyzed.cursor.quote_style, Some(QuoteStyle::Double));
    }

    #[test]
    fn tokenize_with_spans_preserves_offsets_for_quotes_and_pipes() {
        let parser = CommandLineParser;
        let source = r#"ldap user "alice smith" | P uid"#;
        let spans = parser.tokenize_with_spans(source);

        assert_eq!(spans[0].value, "ldap");
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[2].value, "alice smith");
        assert_eq!(&source[spans[2].start..spans[2].end], "\"alice smith\"");
        assert_eq!(spans[3].value, "|");
    }

    #[test]
    fn parse_keeps_inline_flag_values_and_repeated_empty_occurrences() {
        let parser = CommandLineParser;
        let tokens = parser.tokenize("cmd --format=json --os= --format=table");
        let cmd = parser.parse(&tokens);

        assert_eq!(
            cmd.flag_occurrences().cloned().collect::<Vec<_>>(),
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
    fn analyze_clamps_non_char_boundary_cursor_back_to_safe_boundary() {
        let parser = CommandLineParser;
        let line = "ldap user å";
        let analyzed = parser.analyze(line, line.len() - 1);

        assert!(analyzed.parsed.safe_cursor < line.len());
        assert_eq!(analyzed.cursor.token_stub, "");
    }

    #[test]
    fn parse_switches_to_tail_mode_after_first_flag_like_token() {
        let parser = CommandLineParser;
        let tokens = parser.tokenize("ldap user --provider vmware region eu-central");
        let cmd = parser.parse(&tokens);

        assert_eq!(cmd.head(), ["ldap".to_string(), "user".to_string()]);
        assert_eq!(
            cmd.flag_values("--provider"),
            Some(
                &[
                    "vmware".to_string(),
                    "region".to_string(),
                    "eu-central".to_string()
                ][..]
            )
        );
    }

    #[test]
    fn parse_treats_pipe_after_double_dash_as_dsl_boundary() {
        let parser = CommandLineParser;
        let tokens = parser.tokenize("cmd -- literal | F name");
        let cmd = parser.parse(&tokens);

        assert_eq!(cmd.head(), ["cmd".to_string()]);
        assert_eq!(
            cmd.positional_args().cloned().collect::<Vec<_>>(),
            vec!["literal".to_string()]
        );
        assert!(cmd.has_pipe());
        assert_eq!(cmd.pipes(), ["F".to_string(), "name".to_string()]);
    }

    #[test]
    fn tokenize_with_spans_falls_back_for_unmatched_quotes() {
        let parser = CommandLineParser;
        let spans = parser.tokenize_with_spans("cmd --name 'alice");

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].value, "cmd");
        assert_eq!(spans[1].value, "--name");
        assert_eq!(spans[2].value, "'alice");
    }

    #[test]
    fn analyze_falls_back_when_cursor_is_inside_unbalanced_quote() {
        let parser = CommandLineParser;
        let line = r#"ldap user "alice"#;
        let analyzed = parser.analyze(line, line.len());

        assert_eq!(analyzed.parsed.full_tokens, vec!["ldap", "user", "alice"]);
        assert_eq!(analyzed.parsed.cursor_tokens, vec!["ldap", "user", "alice"]);
        assert_eq!(analyzed.cursor.quote_style, Some(QuoteStyle::Double));
        assert_eq!(analyzed.cursor.token_stub, "alice");
    }
}
