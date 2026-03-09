use std::{error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSegment {
    pub raw: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Word,
    Op(Op),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexerError {
    UnterminatedSingleQuote { start: usize },
    UnterminatedDoubleQuote { start: usize },
    TrailingEscape { index: usize },
}

impl fmt::Display for LexerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnterminatedSingleQuote { start } => {
                write!(f, "unterminated single quote starting at byte {start}")
            }
            Self::UnterminatedDoubleQuote { start } => {
                write!(f, "unterminated double quote starting at byte {start}")
            }
            Self::TrailingEscape { index } => {
                write!(f, "trailing escape at byte {index}")
            }
        }
    }
}

impl Error for LexerError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    SingleQuote,
    DoubleQuote,
    EscapeNormal,
    EscapeDouble,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanTransition {
    Structural,
    NormalChar(char),
    QuotedChar(char),
    EscapedChar(char),
}

#[derive(Debug, Clone, Copy)]
struct QuoteScanner {
    state: State,
    base_offset: usize,
    single_quote_start: usize,
    double_quote_start: usize,
}

impl QuoteScanner {
    fn new(base_offset: usize) -> Self {
        Self {
            state: State::Normal,
            base_offset,
            single_quote_start: 0,
            double_quote_start: 0,
        }
    }

    fn is_normal(&self) -> bool {
        matches!(self.state, State::Normal)
    }

    fn advance(&mut self, index: usize, ch: char) -> ScanTransition {
        match self.state {
            State::Normal => match ch {
                '\\' => {
                    self.state = State::EscapeNormal;
                    ScanTransition::Structural
                }
                '\'' => {
                    self.single_quote_start = self.base_offset + index;
                    self.state = State::SingleQuote;
                    ScanTransition::Structural
                }
                '"' => {
                    self.double_quote_start = self.base_offset + index;
                    self.state = State::DoubleQuote;
                    ScanTransition::Structural
                }
                _ => ScanTransition::NormalChar(ch),
            },
            State::SingleQuote => {
                if ch == '\'' {
                    self.state = State::Normal;
                    ScanTransition::Structural
                } else {
                    ScanTransition::QuotedChar(ch)
                }
            }
            State::DoubleQuote => {
                if ch == '"' {
                    self.state = State::Normal;
                    ScanTransition::Structural
                } else if ch == '\\' {
                    self.state = State::EscapeDouble;
                    ScanTransition::Structural
                } else {
                    ScanTransition::QuotedChar(ch)
                }
            }
            State::EscapeNormal => {
                self.state = State::Normal;
                ScanTransition::EscapedChar(ch)
            }
            State::EscapeDouble => {
                self.state = State::DoubleQuote;
                ScanTransition::EscapedChar(ch)
            }
        }
    }

    fn finish(&self, input_len: usize) -> Result<(), LexerError> {
        match self.state {
            State::Normal => Ok(()),
            State::SingleQuote => Err(LexerError::UnterminatedSingleQuote {
                start: self.single_quote_start,
            }),
            State::DoubleQuote => Err(LexerError::UnterminatedDoubleQuote {
                start: self.double_quote_start,
            }),
            State::EscapeNormal | State::EscapeDouble => Err(LexerError::TrailingEscape {
                index: self.base_offset + input_len,
            }),
        }
    }
}

/// Split a full `command | stage | stage` string while respecting quotes.
pub fn split_pipeline(input: &str) -> Result<Vec<StageSegment>, LexerError> {
    let mut out = Vec::new();
    let mut scanner = QuoteScanner::new(0);
    let mut segment_start = 0usize;

    for (index, ch) in input.char_indices() {
        if matches!(scanner.advance(index, ch), ScanTransition::NormalChar('|')) {
            push_segment(input, segment_start, index, &mut out);
            segment_start = index + ch.len_utf8();
        }
    }

    scanner.finish(input.len())?;
    push_segment(input, segment_start, input.len(), &mut out);
    Ok(out)
}

/// Tokenize one stage into words/operators while preserving token spans.
pub fn tokenize_stage(segment: &StageSegment) -> Result<Vec<Token>, LexerError> {
    let mut words = tokenize_words(&segment.raw, segment.span.start)?;
    let mut out = Vec::new();
    for word in words.drain(..) {
        split_word_token(word, segment, &mut out);
    }
    Ok(out)
}

/// Tokenize one stage into plain words using both commas and whitespace as
/// delimiters while still respecting shell-style quoting and escaping.
pub fn tokenize_stage_terms(segment: &StageSegment) -> Result<Vec<Token>, LexerError> {
    tokenize_delimited_words(&segment.raw, segment.span.start, |ch| {
        ch == ',' || ch.is_whitespace()
    })
}

fn tokenize_words(input: &str, base_offset: usize) -> Result<Vec<Token>, LexerError> {
    tokenize_delimited_words(input, base_offset, char::is_whitespace)
}

fn tokenize_delimited_words(
    input: &str,
    base_offset: usize,
    is_delimiter: impl Fn(char) -> bool,
) -> Result<Vec<Token>, LexerError> {
    let mut scanner = QuoteScanner::new(base_offset);
    let mut words = Vec::new();
    let mut current = String::new();
    let mut token_start: Option<usize> = None;

    for (index, ch) in input.char_indices() {
        if scanner.is_normal() && is_delimiter(ch) {
            finish_word(
                &mut words,
                &mut current,
                &mut token_start,
                index,
                base_offset,
            );
            continue;
        }

        if scanner.is_normal() && token_start.is_none() {
            token_start = Some(index);
        }

        match scanner.advance(index, ch) {
            ScanTransition::NormalChar(ch)
            | ScanTransition::QuotedChar(ch)
            | ScanTransition::EscapedChar(ch) => {
                current.push(ch);
            }
            ScanTransition::Structural => {}
        }
    }

    scanner.finish(input.len())?;
    finish_word(
        &mut words,
        &mut current,
        &mut token_start,
        input.len(),
        base_offset,
    );

    Ok(words)
}

fn finish_word(
    out: &mut Vec<Token>,
    current: &mut String,
    token_start: &mut Option<usize>,
    end_index: usize,
    base_offset: usize,
) {
    if let Some(start_index) = token_start.take() {
        out.push(Token {
            kind: TokenKind::Word,
            span: Span {
                start: base_offset + start_index,
                end: base_offset + end_index,
            },
            text: std::mem::take(current),
        });
    }
}

fn split_word_token(token: Token, segment: &StageSegment, out: &mut Vec<Token>) {
    if token.kind != TokenKind::Word {
        out.push(token);
        return;
    }

    let relative_start = token.span.start.saturating_sub(segment.span.start);
    let relative_end = token.span.end.saturating_sub(segment.span.start);
    let raw = &segment.raw[relative_start..relative_end];

    if let Some(op) = parse_full_operator(raw) {
        out.push(Token {
            kind: TokenKind::Op(op),
            ..token
        });
        return;
    }

    let mut state = State::Normal;
    let mut split_happened = false;
    let mut current_text = String::new();
    let mut current_raw_start: Option<usize> = None;
    let mut cursor = 0usize;

    while cursor < raw.len() {
        let tail = &raw[cursor..];
        let ch = tail
            .chars()
            .next()
            .expect("cursor should always point at a valid character boundary");
        let width = ch.len_utf8();

        match state {
            State::Normal => {
                if current_raw_start.is_none()
                    && current_text.is_empty()
                    && cursor == 0
                    && !raw.is_empty()
                {
                    let protected_prefix_len = protected_prefix_len(raw);
                    if protected_prefix_len > 0 && protected_prefix_len < raw.len() {
                        current_raw_start = Some(0);
                        current_text.push_str(&raw[..protected_prefix_len]);
                        cursor += protected_prefix_len;
                        continue;
                    }
                }

                match ch {
                    '\\' => {
                        current_raw_start.get_or_insert(cursor);
                        state = State::EscapeNormal;
                    }
                    '\'' => {
                        current_raw_start.get_or_insert(cursor);
                        state = State::SingleQuote;
                    }
                    '"' => {
                        current_raw_start.get_or_insert(cursor);
                        state = State::DoubleQuote;
                    }
                    _ => {
                        if let Some((op, op_width)) = parse_operator_at(raw, cursor) {
                            push_split_word(
                                out,
                                token.span.start,
                                current_raw_start.take(),
                                cursor,
                                &mut current_text,
                            );
                            out.push(Token {
                                kind: TokenKind::Op(op),
                                span: Span {
                                    start: token.span.start + cursor,
                                    end: token.span.start + cursor + op_width,
                                },
                                text: raw[cursor..cursor + op_width].to_string(),
                            });
                            split_happened = true;
                            cursor += op_width;
                            continue;
                        }

                        current_raw_start.get_or_insert(cursor);
                        current_text.push(ch);
                    }
                }
            }
            State::SingleQuote => {
                if ch == '\'' {
                    state = State::Normal;
                } else {
                    current_text.push(ch);
                }
            }
            State::DoubleQuote => {
                if ch == '"' {
                    state = State::Normal;
                } else if ch == '\\' {
                    state = State::EscapeDouble;
                } else {
                    current_text.push(ch);
                }
            }
            State::EscapeNormal => {
                current_text.push(ch);
                state = State::Normal;
            }
            State::EscapeDouble => {
                current_text.push(ch);
                state = State::DoubleQuote;
            }
        }

        cursor += width;
    }

    if !split_happened {
        out.push(token);
        return;
    }

    push_split_word(
        out,
        token.span.start,
        current_raw_start,
        raw.len(),
        &mut current_text,
    );
}

fn push_split_word(
    out: &mut Vec<Token>,
    base_start: usize,
    raw_start: Option<usize>,
    raw_end: usize,
    text: &mut String,
) {
    let Some(raw_start) = raw_start else {
        return;
    };

    out.push(Token {
        kind: TokenKind::Word,
        span: Span {
            start: base_start + raw_start,
            end: base_start + raw_end,
        },
        text: std::mem::take(text),
    });
}

fn parse_full_operator(text: &str) -> Option<Op> {
    match text {
        "=" => Some(Op::Eq),
        "==" => Some(Op::EqEq),
        "!=" => Some(Op::Ne),
        "<" => Some(Op::Lt),
        "<=" => Some(Op::Le),
        ">" => Some(Op::Gt),
        ">=" => Some(Op::Ge),
        _ => None,
    }
}

fn protected_prefix_len(text: &str) -> usize {
    // DSL prefix sigils such as `!`, `?`, `==`, and `!=` can be part of a
    // single search token; do not split them off as standalone operators.
    if text.starts_with("!?") || text.starts_with("==") || text.starts_with("!=") {
        2
    } else if text.starts_with('!') || text.starts_with('?') || text.starts_with('=') {
        1
    } else {
        0
    }
}

fn parse_operator_at(text: &str, offset: usize) -> Option<(Op, usize)> {
    let tail = text.get(offset..)?;
    if tail.starts_with("<=") {
        return Some((Op::Le, 2));
    }
    if tail.starts_with(">=") {
        return Some((Op::Ge, 2));
    }
    if tail.starts_with("==") {
        return Some((Op::EqEq, 2));
    }
    if tail.starts_with("!=") {
        return Some((Op::Ne, 2));
    }
    if tail.starts_with('<') {
        return Some((Op::Lt, 1));
    }
    if tail.starts_with('>') {
        return Some((Op::Gt, 1));
    }
    if tail.starts_with('=') {
        return Some((Op::Eq, 1));
    }
    None
}

fn push_segment(input: &str, start: usize, end: usize, out: &mut Vec<StageSegment>) {
    let (trimmed_start, trimmed_end) = trim_span(input, start, end);
    if trimmed_start >= trimmed_end {
        return;
    }

    out.push(StageSegment {
        raw: input[trimmed_start..trimmed_end].to_string(),
        span: Span {
            start: trimmed_start,
            end: trimmed_end,
        },
    });
}

fn trim_span(input: &str, start: usize, end: usize) -> (usize, usize) {
    if start >= end {
        return (start, start);
    }

    let mut trimmed_start = start;
    while trimmed_start < end {
        let Some(ch) = input[trimmed_start..].chars().next() else {
            break;
        };
        if ch.is_whitespace() {
            trimmed_start += ch.len_utf8();
        } else {
            break;
        }
    }

    let mut trimmed_end = end;
    while trimmed_end > trimmed_start {
        let Some(ch) = input[..trimmed_end].chars().next_back() else {
            break;
        };
        if ch.is_whitespace() {
            trimmed_end -= ch.len_utf8();
        } else {
            break;
        }
    }

    (trimmed_start, trimmed_end)
}

#[cfg(test)]
mod tests {
    use super::{LexerError, Op, Span, StageSegment, TokenKind, split_pipeline, tokenize_stage};

    #[test]
    fn split_pipeline_respects_quoted_pipes() {
        let segments = split_pipeline("ldap user 'foo|bar' | P uid | F uid=oistes")
            .expect("pipeline should parse");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].raw, "ldap user 'foo|bar'");
        assert_eq!(segments[1].raw, "P uid");
        assert_eq!(segments[2].raw, "F uid=oistes");
    }

    #[test]
    fn split_pipeline_reports_unterminated_quote() {
        let error = split_pipeline("ldap user 'foo|bar | P uid").expect_err("should fail");
        assert_eq!(error, LexerError::UnterminatedSingleQuote { start: 10 });
    }

    #[test]
    fn split_pipeline_reports_trailing_escape() {
        let input = "ldap user foo\\";
        let error = split_pipeline(input).expect_err("trailing escape should fail");
        assert_eq!(error, LexerError::TrailingEscape { index: input.len() });
    }

    #[test]
    fn tokenize_stage_splits_inline_operators() {
        let stage = StageSegment {
            raw: "F uid>=5".to_string(),
            span: Span { start: 0, end: 8 },
        };

        let tokens = tokenize_stage(&stage).expect("tokenization should work");
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].text, "F");
        assert_eq!(tokens[1].text, "uid");
        assert_eq!(tokens[2].kind, TokenKind::Op(Op::Ge));
        assert_eq!(tokens[3].text, "5");
    }

    #[test]
    fn tokenize_stage_keeps_prefix_operators_in_single_token() {
        let stage = StageSegment {
            raw: "Q ==online !?interfaces".to_string(),
            span: Span { start: 0, end: 22 },
        };

        let tokens = tokenize_stage(&stage).expect("tokenization should work");
        assert_eq!(tokens[1].text, "==online");
        assert_eq!(tokens[2].text, "!?interfaces");
    }

    #[test]
    fn tokenize_stage_handles_quotes_and_escapes() {
        let stage = StageSegment {
            raw: "F cn=\"foo bar\"".to_string(),
            span: Span { start: 0, end: 14 },
        };

        let tokens = tokenize_stage(&stage).expect("tokenization should work");
        assert_eq!(tokens[0].text, "F");
        assert_eq!(tokens[1].text, "cn");
        assert_eq!(tokens[2].kind, TokenKind::Op(Op::Eq));
        assert_eq!(tokens[3].text, "foo bar");
    }

    #[test]
    fn tokenize_stage_keeps_operator_chars_inside_quoted_value() {
        let stage = StageSegment {
            raw: "F note=\"a=b>=c\"".to_string(),
            span: Span { start: 0, end: 15 },
        };

        let tokens = tokenize_stage(&stage).expect("tokenization should work");
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].text, "F");
        assert_eq!(tokens[1].text, "note");
        assert_eq!(tokens[2].kind, TokenKind::Op(Op::Eq));
        assert_eq!(tokens[3].text, "a=b>=c");
    }

    #[test]
    fn tokenize_stage_reports_trailing_escape() {
        let stage = StageSegment {
            raw: "F path=C:\\Temp\\".to_string(),
            span: Span { start: 7, end: 22 },
        };

        let error = tokenize_stage(&stage).expect_err("trailing escape should fail");
        assert_eq!(error, LexerError::TrailingEscape { index: 22 });
    }
}
