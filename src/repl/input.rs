use crate::completion::CommandLineParser;
use crate::config::ResolvedConfig;
use crate::dsl::parse::lexer::split_pipeline;
use crate::repl::LineProjection;
use miette::{IntoDiagnostic, Result, WrapErr};
use std::collections::BTreeSet;

use crate::app::ReplScopeStack;
use crate::app::{CMD_HELP, REPL_SHELLABLE_COMMANDS};
use crate::cli::invocation::{hidden_invocation_completion_flags, scan_command_tokens_with_trace};
use crate::cli::pipeline::{ParsedCommandLine, parse_command_text_with_aliases};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplParsedLine {
    pub(crate) command_tokens: Vec<String>,
    pub(crate) dispatch_tokens: Vec<String>,
    pub(crate) stages: Vec<String>,
}

impl ReplParsedLine {
    pub(crate) fn parse(line: &str, config: &ResolvedConfig) -> Result<Self> {
        let parsed = parse_command_text_with_aliases(line, config).wrap_err_with(|| {
            format!(
                "failed to parse REPL line: {}",
                crate::cli::pipeline::truncate_display(line, 60)
            )
        })?;
        Ok(Self::from_parsed(parsed))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.command_tokens.is_empty()
    }

    pub(crate) fn requests_repl_help(&self) -> bool {
        self.command_tokens.len() == 1 && matches!(self.command_tokens[0].as_str(), "--help" | "-h")
    }

    pub(crate) fn shell_entry_command<'a>(&'a self, scope: &ReplScopeStack) -> Option<&'a str> {
        if !self.stages.is_empty() || self.dispatch_tokens.len() != 1 {
            return None;
        }

        let command = self.dispatch_tokens[0].trim();
        if command.is_empty()
            || !is_repl_shellable_command(command)
            || scope.contains_command(command)
        {
            return None;
        }

        Some(command)
    }

    pub(crate) fn prefixed_tokens(&self, scope: &ReplScopeStack) -> Vec<String> {
        scope.prefixed_tokens(&self.dispatch_tokens)
    }

    fn from_parsed(parsed: ParsedCommandLine) -> Self {
        let command_tokens = parsed.tokens;
        let dispatch_tokens =
            rewrite_help_alias_tokens(&command_tokens).unwrap_or_else(|| command_tokens.clone());

        Self {
            command_tokens,
            dispatch_tokens,
            stages: parsed.stages,
        }
    }
}

pub(crate) fn rewrite_help_alias_tokens(tokens: &[String]) -> Option<Vec<String>> {
    rewrite_help_alias_tokens_at(tokens, 0)
}

pub(crate) fn rewrite_help_alias_tokens_at(
    tokens: &[String],
    command_index: usize,
) -> Option<Vec<String>> {
    if tokens.get(command_index).map(String::as_str) != Some(CMD_HELP)
        || !has_valid_help_alias_target(tokens, command_index)
    {
        return None;
    }

    let mut rewritten = tokens[..command_index].to_vec();
    rewritten.extend_from_slice(&tokens[command_index + 1..]);
    if !rewritten.iter().any(|arg| arg == "--help" || arg == "-h") {
        rewritten.push("--help".to_string());
    }
    Some(rewritten)
}

pub(crate) fn project_repl_ui_line(line: &str, config: &ResolvedConfig) -> Result<LineProjection> {
    let _ = config;
    split_pipeline(line).into_diagnostic().wrap_err_with(|| {
        format!(
            "failed to parse REPL line: {}",
            crate::cli::pipeline::truncate_display(line, 60)
        )
    })?;
    let parser = CommandLineParser;
    let spans = parser.tokenize_with_spans(line);
    if spans.is_empty() {
        return Ok(LineProjection::passthrough(line)
            .with_hidden_suggestions(hidden_invocation_completion_flags(&Default::default())));
    }

    let tokens = spans
        .iter()
        .map(|span| span.value.clone())
        .collect::<Vec<_>>();
    let scanned = scan_command_tokens_with_trace(&tokens)?;
    let mut projected = line.as_bytes().to_vec();

    for (index, span) in spans.iter().enumerate() {
        if scanned.kept_indices.contains(&index) {
            continue;
        }
        blank_bytes(&mut projected, span.start, span.end);
    }

    if scanned.tokens.first().map(String::as_str) == Some(CMD_HELP)
        && scanned.tokens.len() > 1
        && let Some(help_index) = scanned.kept_indices.first().copied()
        && let Some(span) = spans.get(help_index)
    {
        blank_bytes(&mut projected, span.start, span.end);
    }

    let hidden_suggestions = projection_hidden_suggestions(&scanned.tokens, &scanned.invocation);

    Ok(LineProjection::passthrough(
        String::from_utf8(projected).unwrap_or_else(|_| line.to_string()),
    )
    .with_hidden_suggestions(hidden_suggestions))
}

pub(crate) fn help_alias_target_at(tokens: &[String], command_index: usize) -> Option<&str> {
    tokens.get(command_index + 1).map(String::as_str)
}

pub(crate) fn has_valid_help_alias_target(tokens: &[String], command_index: usize) -> bool {
    matches!(
        help_alias_target_at(tokens, command_index),
        Some(target) if !target.is_empty() && !target.starts_with('-') && target != CMD_HELP
    )
}

fn projection_hidden_suggestions(
    tokens: &[String],
    invocation: &crate::cli::invocation::InvocationOptions,
) -> BTreeSet<String> {
    if tokens.first().map(String::as_str) != Some(CMD_HELP) {
        return hidden_invocation_completion_flags(invocation);
    }

    let mut hidden = hidden_invocation_completion_flags(&Default::default());
    hidden.insert(CMD_HELP.to_string());
    if has_valid_help_alias_target(tokens, 0) {
        hidden.remove("--verbose");
        if invocation.verbose > 0 {
            hidden.insert("--verbose".to_string());
        }
    }

    hidden
}

fn blank_bytes(buffer: &mut [u8], start: usize, end: usize) {
    for byte in buffer.get_mut(start..end).into_iter().flatten() {
        *byte = b' ';
    }
}

pub(crate) fn is_repl_shellable_command(command: &str) -> bool {
    REPL_SHELLABLE_COMMANDS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(command.trim()))
}

#[cfg(test)]
mod tests {
    use super::{
        ReplParsedLine, has_valid_help_alias_target, project_repl_ui_line,
        rewrite_help_alias_tokens_at,
    };
    use crate::app::ReplScopeStack;
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};

    fn make_config() -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("config should resolve")
    }

    #[test]
    fn help_alias_parsing_rewrite_and_shell_entry_rules_cover_valid_and_invalid_cases_unit() {
        let config = make_config();
        let parsed =
            ReplParsedLine::parse("help ldap user", &config).expect("help alias should parse");
        assert_eq!(parsed.command_tokens, vec!["help", "ldap", "user"]);
        assert_eq!(parsed.dispatch_tokens, vec!["ldap", "user", "--help"]);

        let rewritten = rewrite_help_alias_tokens_at(
            &["orch".to_string(), "help".to_string(), "status".to_string()],
            1,
        )
        .expect("help alias should rewrite after a scope prefix");
        assert_eq!(rewritten, vec!["orch", "status", "--help"]);

        for invalid in [
            vec!["help".to_string(), "help".to_string()],
            vec!["help".to_string(), "--help".to_string()],
        ] {
            assert!(rewrite_help_alias_tokens_at(&invalid, 0).is_none());
            assert!(!has_valid_help_alias_target(&invalid, 0));
        }

        let mut scope = ReplScopeStack::default();
        let ldap = ReplParsedLine::parse("ldap", &config).expect("shell should parse");
        assert_eq!(ldap.shell_entry_command(&scope), Some("ldap"));

        scope.enter("ldap");
        assert_eq!(ldap.shell_entry_command(&scope), None);

        let help_alias =
            ReplParsedLine::parse("help ldap", &config).expect("help alias should parse");
        assert_eq!(help_alias.shell_entry_command(&scope), None);
    }

    #[test]
    fn project_repl_ui_line_masks_invocation_tokens_while_preserving_visible_targets_unit() {
        let config = make_config();

        let projected = project_repl_ui_line("--json help ldap user", &config)
            .expect("projection should succeed");
        assert!(projected.line.contains("ldap user"));
        assert!(!projected.line.contains("--json"));
        assert!(!projected.line.contains("help"));
        assert_eq!(projected.line.len(), "--json help ldap user".len());

        let empty = project_repl_ui_line("", &config).expect("projection should succeed");
        assert_eq!(empty.line, "");
        assert!(empty.hidden_suggestions.contains("--json"));

        for (line, visible_fragment) in [("help history", "history"), ("help his", "his")] {
            let projected = project_repl_ui_line(line, &config).expect("projection should succeed");
            assert!(!projected.line.contains("help"));
            assert!(projected.line.contains(visible_fragment), "line: {line}");
        }
    }

    #[test]
    fn project_repl_ui_line_hidden_suggestions_follow_help_verbosity_and_used_flags_unit() {
        let config = make_config();

        let cases = [
            ("history -", vec!["--json", "--debug"], Vec::<&str>::new()),
            (
                "-v history -",
                Vec::<&str>::new(),
                vec!["--json", "--debug"],
            ),
            (
                "-v --json history -",
                vec!["--json", "--format", "--table"],
                vec!["--debug"],
            ),
            (
                "help ",
                vec!["help", "--json", "--debug"],
                Vec::<&str>::new(),
            ),
            (
                "help history -",
                vec!["--json", "--debug"],
                vec!["--verbose"],
            ),
        ];

        for (line, hidden, visible) in cases {
            let projected = project_repl_ui_line(line, &config).expect("projection should succeed");

            for suggestion in hidden {
                assert!(
                    projected.hidden_suggestions.contains(suggestion),
                    "line: {line}, expected hidden: {suggestion}"
                );
            }
            for suggestion in visible {
                assert!(
                    !projected.hidden_suggestions.contains(suggestion),
                    "line: {line}, expected visible: {suggestion}"
                );
            }
        }
    }
}
