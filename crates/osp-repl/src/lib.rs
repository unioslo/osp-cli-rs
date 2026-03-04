use std::borrow::Cow;
use std::path::PathBuf;

use anyhow::Result;
use reedline::{
    DefaultCompleter, FileBackedHistory, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, Signal,
};

#[derive(Debug, Clone)]
pub struct ReplPrompt {
    pub left: String,
    pub indicator: String,
}

impl ReplPrompt {
    pub fn simple(left: impl Into<String>) -> Self {
        Self {
            left: left.into(),
            indicator: String::new(),
        }
    }
}

pub fn run_repl<F>(
    prompt: ReplPrompt,
    mut completion_words: Vec<String>,
    help_text: String,
    history_path: PathBuf,
    history_max_entries: usize,
    mut execute: F,
) -> Result<i32>
where
    F: FnMut(&str) -> Result<String>,
{
    completion_words.extend(
        ["help", "exit", "quit", "P", "F", "V", "|"]
            .iter()
            .map(|s| s.to_string()),
    );
    completion_words.sort();
    completion_words.dedup();

    let completer = Box::new(DefaultCompleter::new_with_wordlen(completion_words, 2));
    let mut editor = Reedline::create().with_completer(completer);
    if let Some(parent) = history_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let history = FileBackedHistory::with_file(history_max_entries.max(1), history_path)?;
    editor = editor.with_history(Box::new(history));

    let prompt = OspPrompt::new(prompt.left, prompt.indicator);

    let mut command_history: Vec<String> = Vec::new();

    loop {
        match editor.read_line(&prompt)? {
            Signal::Success(line) => {
                let raw = line.trim();
                if raw.is_empty() {
                    continue;
                }

                let expanded = expand_history(raw, &command_history);
                let Some(command_line) = expanded else {
                    eprintln!("No history match for: {raw}");
                    continue;
                };

                match command_line.as_str() {
                    "exit" | "quit" => return Ok(0),
                    "help" => {
                        print!("{help_text}");
                    }
                    _ => match execute(&command_line) {
                        Ok(output) => print!("{output}"),
                        Err(err) => eprintln!("{err}"),
                    },
                }

                command_history.push(command_line);
            }
            Signal::CtrlD => return Ok(0),
            Signal::CtrlC => continue,
        }
    }
}

struct OspPrompt {
    left: String,
    indicator: String,
}

impl OspPrompt {
    fn new(left: String, indicator: String) -> Self {
        Self { left, indicator }
    }
}

impl Prompt for OspPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.left.as_str())
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed(self.indicator.as_str())
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({prefix}reverse-search: {}) ",
            history_search.term
        ))
    }
}

fn expand_history(input: &str, history: &[String]) -> Option<String> {
    if !input.starts_with('!') {
        return Some(input.to_string());
    }

    if input == "!!" {
        return history.last().cloned();
    }

    if let Some(rest) = input.strip_prefix("!-") {
        let idx = rest.parse::<usize>().ok()?;
        if idx == 0 || idx > history.len() {
            return None;
        }
        return history.get(history.len() - idx).cloned();
    }

    let rest = input.strip_prefix('!')?;
    if let Ok(abs_id) = rest.parse::<usize>() {
        if abs_id == 0 || abs_id > history.len() {
            return None;
        }
        return history.get(abs_id - 1).cloned();
    }

    history
        .iter()
        .rev()
        .find(|cmd| cmd.starts_with(rest))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::expand_history;

    #[test]
    fn expands_double_bang() {
        let history = vec!["ldap user oistes".to_string()];
        assert_eq!(
            expand_history("!!", &history),
            Some("ldap user oistes".to_string())
        );
    }

    #[test]
    fn expands_relative() {
        let history = vec![
            "ldap user oistes".to_string(),
            "ldap netgroup ucore".to_string(),
        ];
        assert_eq!(
            expand_history("!-1", &history),
            Some("ldap netgroup ucore".to_string())
        );
    }

    #[test]
    fn expands_prefix() {
        let history = vec![
            "ldap user oistes".to_string(),
            "ldap netgroup ucore".to_string(),
        ];
        assert_eq!(
            expand_history("!ldap user", &history),
            Some("ldap user oistes".to_string())
        );
    }
}
