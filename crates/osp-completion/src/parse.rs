use crate::model::CommandLine;
use std::collections::BTreeMap;

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
    args: Vec<String>,
    flags: BTreeMap<String, Vec<String>>,
    flag_order: Vec<String>,
    pipes: Vec<String>,
    has_pipe: bool,
}

impl ParseState {
    fn finish(self) -> CommandLine {
        CommandLine {
            head: self.head,
            args: self.args,
            flags: self.flags,
            flag_order: self.flag_order,
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
            self.args.push(next.clone());
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
        let mut token = Some(first_token);

        while let Some(current) = token.take() {
            if current == "|" {
                self.start_pipe(iter);
                return;
            }

            if current == "--" {
                self.collect_positional_tail(iter);
                return;
            }

            if let Some((flag, value)) = split_inline_flag_value(&current) {
                if !value.is_empty() {
                    self.flags.entry(flag.clone()).or_default().push(value);
                } else {
                    self.flags.entry(flag.clone()).or_default();
                }
                self.flag_order.push(flag);
                token = iter.next().cloned();
                continue;
            }

            let flag = current;
            let values = self.consume_flag_values(iter);
            self.flags.entry(flag.clone()).or_default().extend(values);
            self.flag_order.push(flag);
            token = iter.next().cloned();
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
pub struct CommandLineParser;

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

    pub fn compute_stub(&self, text_before_cursor: &str, tokens: &[String]) -> String {
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

fn looks_like_flag_start(token: &str) -> bool {
    token.starts_with('-') && token != "-" && !is_number(token)
}

fn is_number(text: &str) -> bool {
    text.parse::<f64>().is_ok()
}

#[cfg(test)]
mod tests {
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
        assert_eq!(cmd.head, vec!["orch", "provision"]);
        assert_eq!(
            cmd.flags.get("--provider"),
            Some(&vec!["vmware".to_string()])
        );
        assert_eq!(cmd.flags.get("--os"), Some(&vec!["rhel".to_string()]));
        assert!(cmd.has_pipe);
        assert_eq!(cmd.pipes, vec!["F", "name"]);
    }

    #[test]
    fn parse_handles_end_of_options_and_negative_numbers() {
        let parser = CommandLineParser;

        let tokens = parser.tokenize("cmd -- --not-a-flag");
        let cmd = parser.parse(&tokens);
        assert_eq!(cmd.head, vec!["cmd"]);
        assert_eq!(cmd.args, vec!["--not-a-flag"]);

        let tokens = parser.tokenize("cmd --count -5");
        let cmd = parser.parse(&tokens);
        assert_eq!(cmd.flags.get("--count"), Some(&vec!["-5".to_string()]));

        let tokens = parser.tokenize("cmd --os=");
        let cmd = parser.parse(&tokens);
        assert_eq!(cmd.flags.get("--os"), Some(&Vec::new()));
    }

    #[test]
    fn compute_stub_respects_equals_boundary() {
        let parser = CommandLineParser;
        let before = "cmd --flag=";
        let tokens = parser.tokenize(before);
        assert_eq!(parser.compute_stub(before, &tokens), "");
    }
}
