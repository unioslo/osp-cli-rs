use miette::Result;
use osp_config::ResolvedConfig;

use crate::app::{CMD_HELP, REPL_SHELLABLE_COMMANDS};
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
    if tokens.first().map(String::as_str) != Some(CMD_HELP) || tokens.len() == 1 {
        return None;
    }

    let mut rewritten = tokens[1..].to_vec();
    if !rewritten.iter().any(|arg| arg == "--help" || arg == "-h") {
        rewritten.push("--help".to_string());
    }
    Some(rewritten)
}

pub(crate) fn is_repl_shellable_command(command: &str) -> bool {
    REPL_SHELLABLE_COMMANDS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(command.trim()))
}

#[cfg(test)]
mod tests {
    use super::ReplParsedLine;
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
}
