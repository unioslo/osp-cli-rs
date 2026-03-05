use crate::model::CommandLine;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexState {
    Normal,
    SingleQuote,
    DoubleQuote,
    EscapeNormal,
    EscapeDouble,
}

#[derive(Debug, Clone, Default)]
pub struct CommandLineParser;

impl CommandLineParser {
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
        let mut head = Vec::new();
        let mut args = Vec::new();
        let mut flags = std::collections::BTreeMap::new();
        let mut flag_order = Vec::new();
        let mut pipes = Vec::new();
        let mut has_pipe = false;

        let mut iter = tokens.iter().peekable();

        while let Some(token) = iter.next() {
            if token == "|" {
                has_pipe = true;
                pipes.extend(iter.cloned());
                return CommandLine {
                    head,
                    args,
                    flags,
                    flag_order,
                    pipes,
                    has_pipe,
                };
            }
            if token == "--" {
                while let Some(next) = iter.next() {
                    if next == "|" {
                        has_pipe = true;
                        pipes.extend(iter.cloned());
                        break;
                    }
                    args.push(next.clone());
                }
                return CommandLine {
                    head,
                    args,
                    flags,
                    flag_order,
                    pipes,
                    has_pipe,
                };
            }
            if token.starts_with('-') {
                // parse flags section from this token onward
                parse_flags(
                    token,
                    &mut iter,
                    &mut flags,
                    &mut flag_order,
                    &mut args,
                    &mut pipes,
                    &mut has_pipe,
                );
                return CommandLine {
                    head,
                    args,
                    flags,
                    flag_order,
                    pipes,
                    has_pipe,
                };
            }
            head.push(token.clone());
        }

        CommandLine {
            head,
            args,
            flags,
            flag_order,
            pipes,
            has_pipe,
        }
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

    pub fn is_number(text: &str) -> bool {
        text.parse::<f64>().is_ok()
    }
}

fn parse_flags<'a>(
    first_flag_or_token: &str,
    iter: &mut std::iter::Peekable<std::slice::Iter<'a, String>>,
    flags: &mut std::collections::BTreeMap<String, Vec<String>>,
    flag_order: &mut Vec<String>,
    args: &mut Vec<String>,
    pipes: &mut Vec<String>,
    has_pipe: &mut bool,
) {
    let mut token = Some(first_flag_or_token.to_string());

    while let Some(current) = token.take() {
        if current == "|" {
            *has_pipe = true;
            pipes.extend(iter.cloned());
            return;
        }

        if current == "--" {
            while let Some(next) = iter.next() {
                if next == "|" {
                    *has_pipe = true;
                    pipes.extend(iter.cloned());
                    break;
                }
                args.push(next.clone());
            }
            return;
        }

        if current.starts_with("--") && current.contains('=') {
            let mut split = current.splitn(2, '=');
            let flag = split.next().unwrap_or_default().to_string();
            let value = split.next().unwrap_or_default().to_string();
            let values = flags.entry(flag.clone()).or_default();
            if !value.is_empty() {
                values.push(value);
            }
            flag_order.push(flag);
            token = iter.next().cloned();
            continue;
        }

        let flag = current;
        let mut values = Vec::new();

        while let Some(next) = iter.peek() {
            if *next == "|" {
                break;
            }
            if *next == "--" {
                break;
            }
            if next.starts_with('-') && *next != "-" && !CommandLineParser::is_number(next) {
                break;
            }

            values.push((*next).clone());
            iter.next();
        }

        flags.entry(flag.clone()).or_default().extend(values);
        flag_order.push(flag);

        token = iter.next().cloned();
    }
}

fn push_current(out: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        out.push(std::mem::take(current));
    }
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
