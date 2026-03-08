use miette::Result;
use osp_completion::CommandLineParser;
use osp_config::ResolvedConfig;
use osp_repl::LineProjection;
use std::collections::BTreeSet;

use crate::app::{CMD_HELP, REPL_SHELLABLE_COMMANDS};
use crate::invocation::{hidden_invocation_completion_flags, scan_command_tokens_with_trace};
use crate::pipeline::{ParsedCommandLine, parse_command_text_with_aliases};
use crate::state::ReplScopeStack;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplParsedLine {
    pub(crate) command_tokens: Vec<String>,
    pub(crate) dispatch_tokens: Vec<String>,
    pub(crate) stages: Vec<String>,
}

impl ReplParsedLine {
    pub(crate) fn parse(line: &str, config: &ResolvedConfig) -> Result<Self> {
        let parsed = parse_command_text_with_aliases(line, config)?;
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
    invocation: &crate::invocation::InvocationOptions,
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
    use crate::state::ReplScopeStack;
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};

    fn make_config() -> osp_config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("profile.active", "default");

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("config should resolve")
    }

    #[test]
    fn parses_help_alias_into_dispatch_tokens() {
        let parsed = ReplParsedLine::parse("help ldap user", &make_config())
            .expect("help alias should parse");

        assert_eq!(parsed.command_tokens, vec!["help", "ldap", "user"]);
        assert_eq!(parsed.dispatch_tokens, vec!["ldap", "user", "--help"]);
    }

    #[test]
    fn shell_entry_checks_dispatch_tokens_and_scope() {
        let config = make_config();
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
    fn rewrites_help_alias_after_scope_prefix_unit() {
        let rewritten = rewrite_help_alias_tokens_at(
            &["orch".to_string(), "help".to_string(), "status".to_string()],
            1,
        )
        .expect("help alias should rewrite");

        assert_eq!(rewritten, vec!["orch", "status", "--help"]);
    }

    #[test]
    fn invalid_help_alias_targets_do_not_rewrite_unit() {
        assert!(
            rewrite_help_alias_tokens_at(&["help".to_string(), "help".to_string()], 0).is_none()
        );
        assert!(
            rewrite_help_alias_tokens_at(&["help".to_string(), "--help".to_string()], 0).is_none()
        );
        assert!(!has_valid_help_alias_target(
            &["help".to_string(), "help".to_string()],
            0
        ));
        assert!(!has_valid_help_alias_target(
            &["help".to_string(), "--help".to_string()],
            0
        ));
    }

    #[test]
    fn projects_repl_ui_line_hides_invocation_flags_and_help_keyword_unit() {
        let projected = project_repl_ui_line("--json help ldap user", &make_config())
            .expect("projection should succeed");

        assert!(projected.line.contains("ldap user"));
        assert!(!projected.line.contains("--json"));
        assert!(!projected.line.contains("help"));
        assert_eq!(projected.line.len(), "--json help ldap user".len());
    }

    #[test]
    fn projects_empty_repl_ui_line_without_tokenization_unit() {
        let projected =
            project_repl_ui_line("", &make_config()).expect("projection should succeed");
        assert_eq!(projected.line, "");
        assert!(projected.hidden_suggestions.contains("--json"));
    }

    #[test]
    fn projection_hides_invocation_completion_flags_until_verbose_unit() {
        let projected =
            project_repl_ui_line("history -", &make_config()).expect("projection should succeed");
        assert!(projected.hidden_suggestions.contains("--json"));
        assert!(projected.hidden_suggestions.contains("--debug"));

        let projected = project_repl_ui_line("-v history -", &make_config())
            .expect("projection should succeed");
        assert!(!projected.hidden_suggestions.contains("--json"));
        assert!(!projected.hidden_suggestions.contains("--debug"));
    }

    #[test]
    fn projection_suppresses_used_one_shot_invocation_flags_unit() {
        let projected = project_repl_ui_line("-v --json history -", &make_config())
            .expect("projection should succeed");
        assert!(projected.hidden_suggestions.contains("--json"));
        assert!(projected.hidden_suggestions.contains("--format"));
        assert!(projected.hidden_suggestions.contains("--table"));
        assert!(!projected.hidden_suggestions.contains("--debug"));
    }

    #[test]
    fn projection_blanks_help_keyword_but_keeps_target_text_unit() {
        let projected = project_repl_ui_line("help history", &make_config())
            .expect("projection should succeed");
        assert!(!projected.line.contains("help"));
        assert!(projected.line.contains("history"));

        let partial =
            project_repl_ui_line("help his", &make_config()).expect("projection should succeed");
        assert!(!partial.line.contains("help"));
        assert!(partial.line.contains("his"));
    }

    #[test]
    fn help_projection_hides_help_and_non_verbose_flags_unit() {
        let projected =
            project_repl_ui_line("help ", &make_config()).expect("projection should succeed");
        assert!(projected.hidden_suggestions.contains("help"));
        assert!(projected.hidden_suggestions.contains("--json"));
        assert!(projected.hidden_suggestions.contains("--debug"));

        let target = project_repl_ui_line("help history -", &make_config())
            .expect("projection should succeed");
        assert!(!target.hidden_suggestions.contains("--verbose"));
        assert!(target.hidden_suggestions.contains("--json"));
        assert!(target.hidden_suggestions.contains("--debug"));
    }
}
