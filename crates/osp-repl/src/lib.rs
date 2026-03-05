use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
mod history;
mod menu;
pub use history::{
    HistoryConfig, HistoryEntry, HistoryShellContext, OspHistoryStore, SharedHistory,
    expand_history,
};
use menu::OspCompletionMenu;
use nu_ansi_term::{Color, Style};
use osp_completion::{
    ArgNode, CommandLineParser, CompletionEngine, CompletionNode, CompletionTree, SuggestionEntry,
    SuggestionOutput,
};
use reedline::{
    Completer, EditCommand, EditMode, Emacs, Highlighter, KeyCode, KeyModifiers, Prompt,
    PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline, ReedlineEvent,
    ReedlineMenu, ReedlineRawEvent, Signal, Span, StyledText, Suggestion,
    default_emacs_keybindings,
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

#[derive(Debug, Clone, Default)]
pub struct ReplAppearance {
    pub completion_text_style: Option<String>,
    pub completion_background_style: Option<String>,
    pub completion_highlight_style: Option<String>,
    pub command_highlight_style: Option<String>,
}

struct AutoCompleteEmacs {
    inner: Emacs,
    menu_name: String,
}

impl AutoCompleteEmacs {
    fn new(inner: Emacs, menu_name: impl Into<String>) -> Self {
        Self {
            inner,
            menu_name: menu_name.into(),
        }
    }

    fn should_reopen_menu(commands: &[EditCommand]) -> bool {
        commands.iter().any(|cmd| {
            matches!(
                cmd,
                EditCommand::InsertChar(_)
                    | EditCommand::InsertString(_)
                    | EditCommand::ReplaceChar(_)
                    | EditCommand::ReplaceChars(_, _)
                    | EditCommand::Backspace
                    | EditCommand::Delete
                    | EditCommand::CutChar
                    | EditCommand::BackspaceWord
                    | EditCommand::DeleteWord
                    | EditCommand::Clear
                    | EditCommand::ClearToLineEnd
                    | EditCommand::CutCurrentLine
                    | EditCommand::CutFromStart
                    | EditCommand::CutFromLineStart
                    | EditCommand::CutToEnd
                    | EditCommand::CutToLineEnd
                    | EditCommand::CutWordLeft
                    | EditCommand::CutBigWordLeft
                    | EditCommand::CutWordRight
                    | EditCommand::CutBigWordRight
                    | EditCommand::CutWordRightToNext
                    | EditCommand::CutBigWordRightToNext
                    | EditCommand::PasteCutBufferBefore
                    | EditCommand::PasteCutBufferAfter
                    | EditCommand::Undo
                    | EditCommand::Redo
            )
        })
    }
}

impl EditMode for AutoCompleteEmacs {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        let parsed = self.inner.parse_event(event);
        match parsed {
            ReedlineEvent::Edit(commands) if Self::should_reopen_menu(&commands) => {
                ReedlineEvent::Multiple(vec![
                    ReedlineEvent::Edit(commands),
                    ReedlineEvent::Menu(self.menu_name.clone()),
                ])
            }
            other => other,
        }
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.inner.edit_mode()
    }
}

pub fn run_repl<F>(
    prompt: ReplPrompt,
    mut completion_words: Vec<String>,
    completion_tree: Option<CompletionTree>,
    appearance: ReplAppearance,
    help_text: String,
    history_config: HistoryConfig,
    mut execute: F,
) -> Result<i32>
where
    F: FnMut(&str, &SharedHistory) -> Result<String>,
{
    completion_words.extend(
        ["help", "exit", "quit", "P", "F", "V", "|"]
            .iter()
            .map(|s| s.to_string()),
    );
    completion_words.sort();
    completion_words.dedup();

    let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&completion_words));
    let completer = Box::new(ReplCompleter::new(completion_words, Some(tree.clone())));
    let completion_menu = Box::new(build_completion_menu(&appearance));
    let highlighter = build_repl_highlighter(&tree, &appearance);
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::BackTab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuPrevious,
        ]),
    );
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char(' '),
        ReedlineEvent::Menu("completion_menu".to_string()),
    );
    let edit_mode = Box::new(AutoCompleteEmacs::new(
        Emacs::new(keybindings),
        "completion_menu",
    ));

    let mut editor = Reedline::create()
        .with_completer(completer)
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_edit_mode(edit_mode);
    if let Some(highlighter) = highlighter {
        editor = editor.with_highlighter(Box::new(highlighter));
    }
    let history_enabled = history_config.enabled && history_config.max_entries > 0;
    let history_limit = if history_enabled {
        history_config.max_entries
    } else {
        0
    };
    let history_exclude_patterns = history_config.exclude_patterns.clone();
    let shell_context = history_config.shell_context.clone();
    let history_store = SharedHistory::new(history_config)?;
    let mut command_history = history_store.recent_commands();
    editor = editor.with_history(Box::new(history_store.clone()));

    let prompt = OspPrompt::new(prompt.left, prompt.indicator);
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        eprintln!("Warning: Input is not a terminal (fd=0).");
        run_repl_basic(
            &prompt,
            &help_text,
            &mut command_history,
            history_limit,
            &history_exclude_patterns,
            shell_context.as_ref(),
            &history_store,
            &mut execute,
        )?;
        return Ok(0);
    }

    loop {
        let signal = match editor.read_line(&prompt) {
            Ok(signal) => signal,
            Err(err) => {
                if is_cursor_position_error(&err) {
                    eprintln!(
                        "WARNING: terminal does not support cursor position requests; \
falling back to basic input mode."
                    );
                    run_repl_basic(
                        &prompt,
                        &help_text,
                        &mut command_history,
                        history_limit,
                        &history_exclude_patterns,
                        shell_context.as_ref(),
                        &history_store,
                        &mut execute,
                    )?;
                    return Ok(0);
                }
                return Err(err.into());
            }
        };

        match signal {
            Signal::Success(line) => {
                let raw = line.trim();
                if raw.is_empty() {
                    continue;
                }

                if raw.starts_with('!') && !history_enabled {
                    continue;
                }

                let shell_prefix = shell_context
                    .as_ref()
                    .and_then(|ctx| ctx.normalized_prefix());
                let expanded = expand_history(raw, &command_history, shell_prefix.as_deref(), true);
                let Some(command_line) = expanded else {
                    eprintln!("No history match for: {raw}");
                    continue;
                };

                let in_shell = shell_prefix.is_some();
                match command_line.as_str() {
                    "exit" | "quit" if !in_shell => return Ok(0),
                    "help" | "--help" | "-h" if !in_shell => {
                        print!("{help_text}");
                    }
                    _ => match execute(&command_line, &history_store) {
                        Ok(output) => print!("{output}"),
                        Err(err) => eprintln!("{err}"),
                    },
                }

                if history_enabled
                    && history::should_record_command(&command_line, &history_exclude_patterns)
                {
                    let full_command =
                        history::apply_shell_prefix(&command_line, shell_prefix.as_deref());
                    if full_command.is_empty() {
                        continue;
                    }
                    command_history.push(full_command);
                    if history_limit > 0 && command_history.len() > history_limit {
                        let overflow = command_history.len() - history_limit;
                        command_history.drain(0..overflow);
                    }
                }
                if history_enabled && command_line.trim().starts_with("history") {
                    command_history = history_store.recent_commands();
                }
            }
            Signal::CtrlD => return Ok(0),
            Signal::CtrlC => continue,
        }
    }
}

fn is_cursor_position_error(err: &io::Error) -> bool {
    if matches!(err.raw_os_error(), Some(6 | 25)) {
        return true;
    }
    let message = err.to_string().to_ascii_lowercase();
    message.contains("cursor position could not be read")
        || message.contains("no such device or address")
        || message.contains("inappropriate ioctl")
}

fn run_repl_basic<F>(
    prompt: &OspPrompt,
    help_text: &str,
    command_history: &mut Vec<String>,
    history_limit: usize,
    history_exclude_patterns: &[String],
    shell_context: Option<&HistoryShellContext>,
    history_store: &SharedHistory,
    execute: &mut F,
) -> Result<()>
where
    F: FnMut(&str, &SharedHistory) -> Result<String>,
{
    let stdin = io::stdin();
    let history_enabled = history_limit > 0;
    loop {
        print!("{}{}", prompt.left, prompt.indicator);
        io::stdout().flush()?;

        let mut line = String::new();
        let read = stdin.read_line(&mut line)?;
        if read == 0 {
            break;
        }

        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        if raw.starts_with('!') && !history_enabled {
            continue;
        }

        let shell_prefix = shell_context
            .as_ref()
            .and_then(|ctx| ctx.normalized_prefix());
        let expanded = expand_history(raw, command_history, shell_prefix.as_deref(), true);
        let Some(command_line) = expanded else {
            eprintln!("No history match for: {raw}");
            continue;
        };

        let in_shell = shell_prefix.is_some();
        match command_line.as_str() {
            "exit" | "quit" if !in_shell => break,
            "help" | "--help" | "-h" if !in_shell => {
                print!("{help_text}");
            }
            _ => match execute(&command_line, history_store) {
                Ok(output) => print!("{output}"),
                Err(err) => eprintln!("{err}"),
            },
        }

        if history_enabled
            && history::should_record_command(&command_line, history_exclude_patterns)
        {
            let full_command = history::apply_shell_prefix(&command_line, shell_prefix.as_deref());
            if full_command.is_empty() {
                continue;
            }
            command_history.push(full_command);
            if history_limit > 0 && command_history.len() > history_limit {
                let overflow = command_history.len() - history_limit;
                command_history.drain(0..overflow);
            }
        }
        if history_enabled && command_line.trim().starts_with("history") {
            *command_history = history_store.recent_commands();
        }
    }
    Ok(())
}

struct ReplCompleter {
    engine: CompletionEngine,
    fallback_words: Vec<String>,
    use_fallback_words: bool,
}

impl ReplCompleter {
    fn new(mut words: Vec<String>, completion_tree: Option<CompletionTree>) -> Self {
        words.sort();
        words.dedup();
        let use_fallback_words = completion_tree.is_none();
        let tree = completion_tree.unwrap_or_else(|| build_repl_tree(&words));
        Self {
            engine: CompletionEngine::new(tree),
            fallback_words: words,
            use_fallback_words,
        }
    }
}

impl Completer for ReplCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let (stub, outputs) = self.engine.suggestions_with_stub(line, pos);
        let span = Span {
            start: pos.saturating_sub(stub.len()),
            end: pos,
        };

        let mut ranked = Vec::new();
        let mut has_path_sentinel = false;
        for output in outputs {
            match output {
                SuggestionOutput::Item(item) => ranked.push(RankedSuggestion {
                    value: item.text,
                    description: item.meta,
                    display: item.display,
                    sort: item.sort,
                    match_score: item.match_score,
                    is_exact: item.is_exact,
                }),
                SuggestionOutput::PathSentinel => has_path_sentinel = true,
            }
        }

        if ranked.is_empty() && self.use_fallback_words {
            ranked.extend(word_suggestions(&self.fallback_words, &stub));
        }

        ranked = dedupe_ranked_suggestions(ranked);
        sort_ranked_suggestions(&mut ranked);

        let mut suggestions = ranked
            .into_iter()
            .map(|item| Suggestion {
                value: item.value,
                description: item.description,
                extra: item.display.map(|display| vec![display]),
                span,
                append_whitespace: true,
                ..Suggestion::default()
            })
            .collect::<Vec<_>>();

        if has_path_sentinel {
            suggestions.extend(path_suggestions(&stub, span));
        }

        suggestions
    }
}

#[derive(Debug, Clone)]
struct RankedSuggestion {
    value: String,
    description: Option<String>,
    display: Option<String>,
    sort: Option<String>,
    match_score: u32,
    is_exact: bool,
}

fn build_repl_tree(words: &[String]) -> CompletionTree {
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
        pipe_verbs: BTreeMap::from([
            ("F".to_string(), "Filter rows".to_string()),
            ("P".to_string(), "Project columns".to_string()),
            ("S".to_string(), "Sort rows".to_string()),
            ("G".to_string(), "Group rows".to_string()),
            ("A".to_string(), "Aggregate groups".to_string()),
            ("L".to_string(), "Limit rows".to_string()),
            ("C".to_string(), "Count rows".to_string()),
            ("Y".to_string(), "Copy output".to_string()),
            ("H".to_string(), "DSL help".to_string()),
            ("V".to_string(), "Value-only quick filter".to_string()),
        ]),
    }
}

fn build_completion_menu(appearance: &ReplAppearance) -> OspCompletionMenu {
    let text_color = appearance
        .completion_text_style
        .as_deref()
        .and_then(color_from_style_spec);
    let background_color = appearance
        .completion_background_style
        .as_deref()
        .and_then(color_from_style_spec);
    let highlight_color = appearance
        .completion_highlight_style
        .as_deref()
        .and_then(color_from_style_spec);

    OspCompletionMenu::default()
        .with_name("completion_menu")
        .with_only_buffer_difference(false)
        .with_marker("")
        .with_columns(u16::MAX)
        .with_max_rows(u16::MAX)
        .with_description_rows(1)
        .with_column_padding(2)
        .with_text_style(style_with_fg_bg(text_color, background_color))
        .with_description_text_style(style_with_fg_bg(text_color, highlight_color))
        .with_match_text_style(style_with_fg_bg(highlight_color, background_color).underline())
        .with_selected_text_style(style_with_fg_bg(highlight_color, text_color).bold())
        .with_selected_match_text_style(
            style_with_fg_bg(highlight_color, text_color)
                .bold()
                .underline(),
        )
}

fn build_repl_highlighter(
    tree: &CompletionTree,
    appearance: &ReplAppearance,
) -> Option<ReplHighlighter> {
    let command_color = appearance
        .command_highlight_style
        .as_deref()
        .and_then(color_from_style_spec);
    if command_color.is_none() {
        return None;
    }
    Some(ReplHighlighter::new(tree.clone(), command_color))
}

fn style_with_fg_bg(fg: Option<Color>, bg: Option<Color>) -> Style {
    let mut style = Style::new();
    if let Some(fg) = fg {
        style = style.fg(fg);
    }
    if let Some(bg) = bg {
        style = style.on(bg);
    }
    style
}

struct ReplHighlighter {
    tree: CompletionTree,
    command_color: Color,
    parser: CommandLineParser,
}

impl ReplHighlighter {
    fn new(tree: CompletionTree, command_color: Option<Color>) -> Self {
        Self {
            tree,
            command_color: command_color.unwrap_or(Color::Green),
            parser: CommandLineParser,
        }
    }

    fn matched_command_len(&self, tokens: &[String]) -> usize {
        let mut node = &self.tree.root;
        let mut matched = 0usize;

        for token in tokens {
            if token == "|" {
                break;
            }
            if token.starts_with('-') {
                break;
            }
            let Some(child) = node.children.get(token) else {
                break;
            };
            if child.value_key {
                break;
            }
            matched += 1;
            if child.value_leaf {
                break;
            }
            node = child;
        }

        matched
    }
}

fn is_flag_token(token: &str) -> bool {
    if !token.starts_with('-') {
        return false;
    }
    if token == "-" || token == "--" {
        return false;
    }
    if token.starts_with("--") {
        return token.len() > 2;
    }
    token
        .chars()
        .nth(1)
        .is_some_and(|ch| ch.is_ascii_alphabetic())
}

impl Highlighter for ReplHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let mut styled = StyledText::new();
        if line.is_empty() {
            return styled;
        }

        let tokens = self.parser.tokenize(line);
        if tokens.is_empty() {
            styled.push((Style::new(), line.to_string()));
            return styled;
        }

        let matched_len = self.matched_command_len(&tokens);
        let mut pos = 0usize;

        for (index, token) in tokens.iter().enumerate() {
            if token.is_empty() {
                continue;
            }
            let Some(rel_idx) = line[pos..].find(token) else {
                styled.push((Style::new(), line.to_string()));
                return styled;
            };
            let idx = pos + rel_idx;
            if idx > pos {
                styled.push((Style::new(), line[pos..idx].to_string()));
            }

            let style = if index < matched_len {
                Style::new().fg(self.command_color)
            } else if is_flag_token(token) {
                Style::new().fg(self.command_color)
            } else if let Some(color) = parse_hex_color_token(token) {
                Style::new().fg(color)
            } else {
                Style::new()
            };

            styled.push((style, line[idx..idx + token.len()].to_string()));
            pos = idx + token.len();
        }

        if pos < line.len() {
            styled.push((Style::new(), line[pos..].to_string()));
        }

        styled
    }
}

fn parse_hex_color_token(token: &str) -> Option<Color> {
    let normalized = token.trim();
    let Some(hex) = normalized.strip_prefix('#') else {
        return None;
    };
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

fn color_from_style_spec(spec: &str) -> Option<Color> {
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

fn dedupe_ranked_suggestions(items: Vec<RankedSuggestion>) -> Vec<RankedSuggestion> {
    let mut out: Vec<RankedSuggestion> = Vec::new();
    for item in items {
        if let Some(existing) = out.iter_mut().find(|entry| entry.value == item.value) {
            if existing.description.is_none() {
                existing.description = item.description;
            }
            if existing.display.is_none() {
                existing.display = item.display;
            }
            if existing.sort.is_none() {
                existing.sort = item.sort;
            }
            existing.is_exact |= item.is_exact;
            existing.match_score = existing.match_score.min(item.match_score);
            continue;
        }
        out.push(item);
    }
    out
}

fn sort_ranked_suggestions(items: &mut [RankedSuggestion]) {
    let all_numeric =
        !items.is_empty() && items.iter().all(|item| numeric_sort_value(item).is_some());

    if all_numeric {
        items.sort_by(|left, right| {
            let left_sort = numeric_sort_value(left).unwrap_or(f64::MAX);
            let right_sort = numeric_sort_value(right).unwrap_or(f64::MAX);
            (not_exact(left), left.match_score)
                .cmp(&(not_exact(right), right.match_score))
                .then_with(|| {
                    left_sort
                        .partial_cmp(&right_sort)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    left.value
                        .to_ascii_lowercase()
                        .cmp(&right.value.to_ascii_lowercase())
                })
        });
        return;
    }

    items.sort_by(|left, right| {
        (
            not_exact(left),
            left.match_score,
            left.value.to_ascii_lowercase(),
        )
            .cmp(&(
                not_exact(right),
                right.match_score,
                right.value.to_ascii_lowercase(),
            ))
    });
}

fn not_exact(item: &RankedSuggestion) -> bool {
    !item.is_exact
}

fn numeric_sort_value(item: &RankedSuggestion) -> Option<f64> {
    item.sort
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| item.value.parse::<f64>().ok())
        .or_else(|| {
            item.description
                .as_deref()
                .and_then(|value| value.parse::<f64>().ok())
        })
}

fn word_suggestions(words: &[String], stub: &str) -> Vec<RankedSuggestion> {
    words
        .iter()
        .filter_map(|word| {
            let score = fuzzy_match_score(stub, word)?;
            Some(RankedSuggestion {
                value: word.clone(),
                description: None,
                display: None,
                sort: None,
                match_score: score,
                is_exact: score == 0,
            })
        })
        .collect()
}

fn fuzzy_match_score(stub: &str, candidate: &str) -> Option<u32> {
    if stub.is_empty() {
        return Some(1_000);
    }

    let stub_lc = stub.to_ascii_lowercase();
    let candidate_lc = candidate.to_ascii_lowercase();
    if stub_lc == candidate_lc {
        return Some(0);
    }
    if candidate_lc.starts_with(&stub_lc) {
        return Some(100 + (candidate_lc.len().saturating_sub(stub_lc.len())) as u32);
    }

    if let Some(boundary) = boundary_prefix_index(&candidate_lc, &stub_lc) {
        return Some(200 + boundary as u32);
    }

    let fuzzy = fuzzy_matcher().fuzzy_match(&candidate_lc, &stub_lc)?;
    let normalized = fuzzy.max(0) as u32;
    let penalty = 100_000u32.saturating_sub(normalized);
    Some(10_000 + penalty)
}

fn fuzzy_matcher() -> &'static SkimMatcherV2 {
    static MATCHER: OnceLock<SkimMatcherV2> = OnceLock::new();
    MATCHER.get_or_init(SkimMatcherV2::default)
}

fn boundary_prefix_index(candidate: &str, stub: &str) -> Option<usize> {
    candidate
        .match_indices(stub)
        .find(|(idx, _)| {
            *idx == 0
                || candidate
                    .as_bytes()
                    .get(idx.saturating_sub(1))
                    .is_some_and(|byte| matches!(byte, b'-' | b'_' | b'.' | b':' | b'/'))
        })
        .map(|(idx, _)| idx)
}

fn path_suggestions(stub: &str, span: Span) -> Vec<Suggestion> {
    let (lookup, insert_prefix, typed_prefix) = split_path_stub(stub);
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

        out.push(Suggestion {
            value: format!("{insert_prefix}{file_name}{suffix}"),
            description: Some(if is_dir { "dir" } else { "file" }.to_string()),
            span,
            append_whitespace: !is_dir,
            ..Suggestion::default()
        });
    }

    out
}

fn split_path_stub(stub: &str) -> (PathBuf, String, String) {
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
        lookup = parent.to_path_buf();
    } else {
        lookup = PathBuf::from(".");
    }

    (lookup, insert_prefix, typed_prefix)
}

fn expand_home(path: &str) -> String {
    if path == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
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

#[cfg(test)]
mod tests {
    use nu_ansi_term::Color;
    use osp_completion::{CompletionNode, CompletionTree};
    use reedline::Completer;

    use super::{
        RankedSuggestion, ReplCompleter, color_from_style_spec, dedupe_ranked_suggestions,
        expand_history, sort_ranked_suggestions,
    };

    #[test]
    fn expands_double_bang() {
        let history = vec!["ldap user oistes".to_string()];
        assert_eq!(
            expand_history("!!", &history, None, false),
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
            expand_history("!-1", &history, None, false),
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
            expand_history("!ldap user", &history, None, false),
            Some("ldap user oistes".to_string())
        );
    }

    #[test]
    fn completer_suggests_word_prefixes() {
        let mut completer = ReplCompleter::new(
            vec![
                "ldap".to_string(),
                "plugins".to_string(),
                "theme".to_string(),
            ],
            None,
        );

        let completions = completer.complete("ld", 2);
        let values = completions
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();
        assert_eq!(values, vec!["ldap".to_string()]);
    }

    #[test]
    fn completer_supports_fuzzy_word_matching() {
        let mut completer = ReplCompleter::new(
            vec![
                "ldap".to_string(),
                "plugins".to_string(),
                "theme".to_string(),
            ],
            None,
        );

        let completions = completer.complete("lap", 3);
        let values = completions
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();
        assert!(values.contains(&"ldap".to_string()));
    }

    #[test]
    fn completer_suggests_pipe_verbs_after_pipe() {
        let mut completer = ReplCompleter::new(vec!["ldap".to_string()], None);
        let completions = completer.complete("ldap user | F", "ldap user | F".len());
        let values = completions
            .into_iter()
            .map(|suggestion| suggestion.value)
            .collect::<Vec<_>>();
        assert!(values.contains(&"F".to_string()));
    }

    #[test]
    fn completer_with_tree_does_not_fallback_to_word_list() {
        let mut root = CompletionNode::default();
        root.children
            .insert("config".to_string(), CompletionNode::default());
        let tree = CompletionTree {
            root,
            ..CompletionTree::default()
        };

        let mut completer = ReplCompleter::new(vec!["ldap".to_string()], Some(tree));
        let completions = completer.complete("zzz", 3);
        assert!(completions.is_empty());
    }

    #[test]
    fn color_parser_extracts_hex_and_named_colors() {
        assert_eq!(
            color_from_style_spec("bold #ff79c6"),
            Some(Color::Rgb(255, 121, 198))
        );
        assert_eq!(
            color_from_style_spec("fg:cyan underline"),
            Some(Color::Cyan)
        );
        assert_eq!(color_from_style_spec("bg:ansi141"), Some(Color::Fixed(141)));
        assert_eq!(
            color_from_style_spec("fg:rgb(80,250,123)"),
            Some(Color::Rgb(80, 250, 123))
        );
        assert!(color_from_style_spec("not-a-color").is_none());
    }

    #[test]
    fn dedupe_ranked_suggestions_preserves_meta_and_display() {
        let input = vec![
            RankedSuggestion {
                value: "x".to_string(),
                description: Some("meta".to_string()),
                display: None,
                sort: None,
                match_score: 120,
                is_exact: false,
            },
            RankedSuggestion {
                value: "x".to_string(),
                description: None,
                display: Some("Display".to_string()),
                sort: Some("10".to_string()),
                match_score: 80,
                is_exact: false,
            },
        ];

        let deduped = dedupe_ranked_suggestions(input);
        assert_eq!(deduped.len(), 1);
        let only = &deduped[0];
        assert_eq!(only.description.as_deref(), Some("meta"));
        assert_eq!(only.display.as_deref(), Some("Display"));
        assert_eq!(only.sort.as_deref(), Some("10"));
        assert_eq!(only.match_score, 80);
    }

    #[test]
    fn sort_ranked_suggestions_prioritizes_exact_then_match_score() {
        let mut items = vec![
            RankedSuggestion {
                value: "ldap-user".to_string(),
                description: None,
                display: None,
                sort: None,
                match_score: 180,
                is_exact: false,
            },
            RankedSuggestion {
                value: "ldap".to_string(),
                description: None,
                display: None,
                sort: None,
                match_score: 0,
                is_exact: true,
            },
            RankedSuggestion {
                value: "plugins".to_string(),
                description: None,
                display: None,
                sort: None,
                match_score: 420,
                is_exact: false,
            },
        ];

        sort_ranked_suggestions(&mut items);
        let ordered = items
            .iter()
            .map(|item| item.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec!["ldap", "ldap-user", "plugins"]);
    }
}
