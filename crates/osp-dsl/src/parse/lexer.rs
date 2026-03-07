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

/// Split a full `command | stage | stage` string while respecting quotes.
pub fn split_pipeline(input: &str) -> Result<Vec<StageSegment>, LexerError> {
    let mut out = Vec::new();
    let mut state = State::Normal;
    let mut segment_start = 0usize;
    let mut single_quote_start = 0usize;
    let mut double_quote_start = 0usize;

    for (index, ch) in input.char_indices() {
        match state {
            State::Normal => match ch {
                '\\' => state = State::EscapeNormal,
                '\'' => {
                    single_quote_start = index;
                    state = State::SingleQuote;
                }
                '"' => {
                    double_quote_start = index;
                    state = State::DoubleQuote;
                }
                '|' => {
                    push_segment(input, segment_start, index, &mut out);
                    segment_start = index + ch.len_utf8();
                }
                _ => {}
            },
            State::SingleQuote => {
                if ch == '\'' {
                    state = State::Normal;
                }
            }
            State::DoubleQuote => {
                if ch == '"' {
                    state = State::Normal;
                } else if ch == '\\' {
                    state = State::EscapeDouble;
                }
            }
            State::EscapeNormal => state = State::Normal,
            State::EscapeDouble => state = State::DoubleQuote,
        }
    }

    match state {
        State::Normal => {}
        State::SingleQuote => {
            return Err(LexerError::UnterminatedSingleQuote {
                start: single_quote_start,
            });
        }
        State::DoubleQuote => {
            return Err(LexerError::UnterminatedDoubleQuote {
                start: double_quote_start,
            });
        }
        State::EscapeNormal | State::EscapeDouble => {
            return Err(LexerError::TrailingEscape { index: input.len() });
        }
    }

    push_segment(input, segment_start, input.len(), &mut out);
    Ok(out)
}

/// Tokenize one stage into words/operators while preserving token spans.
pub fn tokenize_stage(segment: &StageSegment) -> Result<Vec<Token>, LexerError> {
    let mut words = tokenize_words(&segment.raw, segment.span.start)?;
    let mut out = Vec::new();
    for word in words.drain(..) {
        split_word_token(word, &mut out);
    }
    Ok(out)
}

fn tokenize_words(input: &str, base_offset: usize) -> Result<Vec<Token>, LexerError> {
    let mut state = State::Normal;
    let mut words = Vec::new();
    let mut current = String::new();
    let mut token_start: Option<usize> = None;
    let mut single_quote_start = 0usize;
    let mut double_quote_start = 0usize;

    for (index, ch) in input.char_indices() {
        match state {
            State::Normal => {
                if ch.is_whitespace() {
                    finish_word(
                        &mut words,
                        &mut current,
                        &mut token_start,
                        index,
                        base_offset,
                    );
                    continue;
                }

                if token_start.is_none() {
                    token_start = Some(index);
                }

                match ch {
                    '\\' => state = State::EscapeNormal,
                    '\'' => {
                        single_quote_start = base_offset + index;
                        state = State::SingleQuote;
                    }
                    '"' => {
                        double_quote_start = base_offset + index;
                        state = State::DoubleQuote;
                    }
                    _ => current.push(ch),
                }
            }
            State::SingleQuote => {
                if ch == '\'' {
                    state = State::Normal;
                } else {
                    current.push(ch);
                }
            }
            State::DoubleQuote => {
                if ch == '"' {
                    state = State::Normal;
                } else if ch == '\\' {
                    state = State::EscapeDouble;
                } else {
                    current.push(ch);
                }
            }
            State::EscapeNormal => {
                current.push(ch);
                state = State::Normal;
            }
            State::EscapeDouble => {
                current.push(ch);
                state = State::DoubleQuote;
            }
        }
    }

    match state {
        State::Normal => {}
        State::SingleQuote => {
            return Err(LexerError::UnterminatedSingleQuote {
                start: single_quote_start,
            });
        }
        State::DoubleQuote => {
            return Err(LexerError::UnterminatedDoubleQuote {
                start: double_quote_start,
            });
        }
        State::EscapeNormal | State::EscapeDouble => {
            return Err(LexerError::TrailingEscape {
                index: base_offset + input.len(),
            });
        }
    }

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

fn split_word_token(token: Token, out: &mut Vec<Token>) {
    if token.kind != TokenKind::Word {
        out.push(token);
        return;
    }

    if let Some(op) = parse_full_operator(&token.text) {
        out.push(Token {
            kind: TokenKind::Op(op),
            ..token
        });
        return;
    }

    let protected_prefix_len = protected_prefix_len(&token.text);
    let mut cursor = protected_prefix_len;
    let mut chunk_start = 0usize;
    let mut split_happened = false;

    while cursor < token.text.len() {
        if let Some((op, width)) = parse_operator_at(&token.text, cursor) {
            if cursor > chunk_start {
                out.push(Token {
                    kind: TokenKind::Word,
                    span: Span {
                        start: token.span.start + chunk_start,
                        end: token.span.start + cursor,
                    },
                    text: token.text[chunk_start..cursor].to_string(),
                });
            }

            out.push(Token {
                kind: TokenKind::Op(op),
                span: Span {
                    start: token.span.start + cursor,
                    end: token.span.start + cursor + width,
                },
                text: token.text[cursor..cursor + width].to_string(),
            });

            cursor += width;
            chunk_start = cursor;
            split_happened = true;
            continue;
        }
        cursor += 1;
    }

    if !split_happened {
        out.push(token);
        return;
    }

    if chunk_start < token.text.len() {
        out.push(Token {
            kind: TokenKind::Word,
            span: Span {
                start: token.span.start + chunk_start,
                end: token.span.end,
            },
            text: token.text[chunk_start..].to_string(),
        });
    }
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
}
