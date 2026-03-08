use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::ResolvedConfig;
use osp_dsl::{
    model::{ParsedStage, ParsedStageKind},
    parse::pipeline::parse_stage,
    parse_pipeline,
};

use crate::app::is_sensitive_key;

const MAX_ALIAS_EXPANSION_DEPTH: usize = 100;

pub(crate) fn truncate_display(s: &str, max_len: usize) -> String {
    let trimmed = s.trim();
    let char_count = trimmed.chars().count();
    if char_count <= max_len {
        trimmed.to_string()
    } else {
        let end = trimmed
            .char_indices()
            .nth(max_len)
            .map(|(index, _)| index)
            .unwrap_or(trimmed.len());
        format!("{}... ({} chars)", &trimmed[..end], char_count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommandLine {
    pub tokens: Vec<String>,
    pub stages: Vec<String>,
}

pub fn parse_command_text_with_aliases(
    text: &str,
    config: &ResolvedConfig,
) -> Result<ParsedCommandLine> {
    let parsed = parse_pipeline(text)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to parse pipeline: {}", truncate_display(text, 60)))?;
    let command_tokens = shell_words::split(&parsed.command)
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "failed to parse command tokens: {}",
                truncate_display(&parsed.command, 60)
            )
        })?;
    finalize_command_with_aliases(command_tokens, parsed.stages, config)
}

pub fn parse_command_tokens_with_aliases(
    tokens: &[String],
    config: &ResolvedConfig,
) -> Result<ParsedCommandLine> {
    if tokens.is_empty() {
        return Ok(ParsedCommandLine {
            tokens: Vec::new(),
            stages: Vec::new(),
        });
    }

    let split = split_command_tokens(tokens);
    finalize_command_with_aliases(split.command_tokens, split.stages, config)
}

fn maybe_expand_alias(
    candidate: &str,
    positional_args: &[String],
    config: &ResolvedConfig,
) -> Result<Option<String>> {
    let Some(value) = config.get_alias_entry(candidate) else {
        return Ok(None);
    };

    let template = value.raw_value.to_string();
    let expanded = expand_alias_template(candidate, &template, positional_args, config)
        .wrap_err_with(|| format!("failed to expand alias `{candidate}`"))?;
    Ok(Some(expanded))
}

fn finalize_command_with_aliases(
    command_tokens: Vec<String>,
    stages: Vec<String>,
    config: &ResolvedConfig,
) -> Result<ParsedCommandLine> {
    if command_tokens.is_empty() {
        return Ok(ParsedCommandLine {
            tokens: Vec::new(),
            stages: Vec::new(),
        });
    }

    let alias_name = &command_tokens[0];
    if let Some(expanded) = maybe_expand_alias(alias_name, &command_tokens[1..], config)? {
        tracing::trace!(
            alias = %alias_name,
            "alias expanded"
        );
        let alias_parsed = parse_pipeline(&expanded)
            .into_diagnostic()
            .wrap_err_with(|| {
                format!(
                    "failed to parse alias `{alias_name}` expansion: {}",
                    truncate_display(&expanded, 60)
                )
            })?;
        let alias_tokens = shell_words::split(&alias_parsed.command)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to parse alias `{alias_name}` command tokens"))?;
        if alias_tokens.is_empty() {
            return Ok(ParsedCommandLine {
                tokens: Vec::new(),
                stages: Vec::new(),
            });
        }

        let mut merged_stages = alias_parsed.stages;
        merged_stages.extend(stages);
        return finalize_parsed_command(alias_tokens, merged_stages);
    }

    finalize_parsed_command(command_tokens, stages)
}

fn finalize_parsed_command(tokens: Vec<String>, stages: Vec<String>) -> Result<ParsedCommandLine> {
    validate_cli_dsl_stages(&stages)?;
    Ok(ParsedCommandLine {
        tokens: merge_orch_os_tokens(tokens),
        stages,
    })
}

fn merge_orch_os_tokens(tokens: Vec<String>) -> Vec<String> {
    if tokens.len() < 4 || tokens.first().map(String::as_str) != Some("orch") {
        return tokens;
    }
    if tokens.get(1).map(String::as_str) != Some("provision") {
        return tokens;
    }

    let mut merged = Vec::with_capacity(tokens.len());
    let mut index = 0usize;
    while index < tokens.len() {
        if tokens[index] == "--os" && index + 2 < tokens.len() {
            let family = &tokens[index + 1];
            let version = &tokens[index + 2];
            if !version.is_empty() && !version.starts_with('-') {
                merged.push("--os".to_string());
                merged.push(format!("{family}{version}"));
                index += 3;
                continue;
            }
        }

        merged.push(tokens[index].clone());
        index += 1;
    }

    merged
}

pub fn validate_cli_dsl_stages(stages: &[String]) -> Result<()> {
    for raw in stages {
        let parsed = parse_stage(raw).into_diagnostic().wrap_err_with(|| {
            format!("failed to parse DSL stage: {}", truncate_display(raw, 80))
        })?;
        if parsed.verb.is_empty() {
            continue;
        }
        if matches!(
            parsed.kind,
            ParsedStageKind::Explicit | ParsedStageKind::Quick
        ) || is_cli_help_stage(&parsed)
        {
            continue;
        }

        return Err(miette!(
            "Unknown DSL verb '{}' in pipe '{}'. Use `| H <verb>` for help.",
            parsed.verb,
            raw.trim()
        ));
    }

    Ok(())
}

pub fn is_cli_help_stage(parsed: &ParsedStage) -> bool {
    matches!(parsed.kind, ParsedStageKind::UnknownExplicit) && parsed.verb.eq_ignore_ascii_case("H")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SplitCommandTokens {
    command_tokens: Vec<String>,
    stages: Vec<String>,
}

fn split_command_tokens(tokens: &[String]) -> SplitCommandTokens {
    let mut segments = Vec::new();
    let mut current = Vec::new();

    for token in tokens {
        if token == "|" {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(token.clone());
    }

    if !current.is_empty() {
        segments.push(current);
    }

    let mut iter = segments.into_iter();
    let command_tokens = iter.next().unwrap_or_default();
    let stages = iter
        .map(|segment| {
            segment
                .into_iter()
                .map(|token| quote_token(&token))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();

    SplitCommandTokens {
        command_tokens,
        stages,
    }
}

fn expand_alias_template(
    alias_name: &str,
    template: &str,
    positional_args: &[String],
    config: &ResolvedConfig,
) -> Result<String> {
    let mut current = template.to_string();

    for _ in 0..MAX_ALIAS_EXPANSION_DEPTH {
        if !current.contains("${") {
            return Ok(current);
        }

        let mut out = String::new();
        let mut cursor = 0usize;

        while let Some(rel_start) = current[cursor..].find("${") {
            let start = cursor + rel_start;
            out.push_str(&current[cursor..start]);

            let after_open = start + 2;
            let Some(rel_end) = current[after_open..].find('}') else {
                return Err(miette!(
                    "invalid alias placeholder syntax in alias '{alias_name}': '{template}'"
                ));
            };
            let end = after_open + rel_end;
            let placeholder = current[after_open..end].trim();
            if placeholder.is_empty() {
                return Err(miette!(
                    "invalid alias placeholder syntax in alias '{alias_name}': '{template}'"
                ));
            }

            let (key_part, default) = split_placeholder(placeholder);
            let replacement =
                resolve_alias_placeholder(alias_name, key_part, default, positional_args, config)?;
            out.push_str(&replacement);
            cursor = end + 1;
        }

        out.push_str(&current[cursor..]);
        if out == current {
            return Ok(out);
        }
        current = out;
    }

    Err(miette!(
        "Expansion depth exceeded 100 on alias '{alias_name}'."
    ))
}

fn split_placeholder(placeholder: &str) -> (&str, Option<&str>) {
    if let Some((key, default)) = placeholder.split_once(':') {
        (key.trim(), Some(default))
    } else {
        (placeholder.trim(), None)
    }
}

fn resolve_alias_placeholder(
    alias_name: &str,
    key_part: &str,
    default: Option<&str>,
    positional_args: &[String],
    config: &ResolvedConfig,
) -> Result<String> {
    if key_part.is_empty() {
        return Err(miette!(
            "invalid alias placeholder syntax in alias '{alias_name}'"
        ));
    }

    if let Ok(index) = key_part.parse::<usize>()
        && index > 0
        && index <= positional_args.len()
    {
        return Ok(positional_args[index - 1].clone());
    }

    if key_part == "*" || key_part == "@" {
        let joined = positional_args
            .iter()
            .map(|arg| quote_token(arg))
            .collect::<Vec<String>>()
            .join(" ");
        return Ok(joined);
    }

    if is_sensitive_key(key_part) {
        return Err(miette!(
            "Alias '{alias_name}' cannot expand sensitive config placeholder '{key_part}'"
        ));
    }

    if let Some(value) = config.get(key_part) {
        return Ok(value.to_string());
    }

    if let Some(default_value) = default {
        return Ok(default_value.to_string());
    }

    Err(miette!(
        "Alias '{alias_name}' requires value for placeholder '{key_part}'"
    ))
}

fn quote_token(token: &str) -> String {
    if token.is_empty() {
        return "''".to_string();
    }
    let needs_quotes = token.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '"'
                    | '\\'
                    | '$'
                    | '`'
                    | '|'
                    | '&'
                    | ';'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '!'
            )
    });
    if !needs_quotes {
        return token.to_string();
    }

    if !token.contains('\'') {
        return format!("'{token}'");
    }

    let mut out = String::new();
    out.push('\'');
    for ch in token.chars() {
        if ch == '\'' {
            out.push_str("'\"'\"'");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::{
        expand_alias_template, parse_command_text_with_aliases, parse_command_tokens_with_aliases,
        truncate_display, validate_cli_dsl_stages,
    };
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};

    fn test_config(entries: &[(&str, &str)]) -> osp_config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver
            .resolve(ResolveOptions::default())
            .expect("test config should resolve")
    }

    #[test]
    fn alias_can_expand_non_sensitive_config_values() {
        let config = test_config(&[("alias.demo", "echo ${ui.format}"), ("ui.format", "json")]);

        let parsed = parse_command_tokens_with_aliases(&["demo".to_string()], &config)
            .expect("alias should expand");
        assert_eq!(parsed.tokens, vec!["echo".to_string(), "json".to_string()]);
    }

    #[test]
    fn alias_rejects_sensitive_config_placeholders() {
        let config = test_config(&[]);

        let err = expand_alias_template("danger", "echo ${auth.api_key}", &[], &config)
            .expect_err("sensitive placeholder should be rejected");
        assert!(
            err.to_string()
                .contains("cannot expand sensitive config placeholder")
        );
    }

    #[test]
    fn alias_expands_and_merges_following_stages() {
        let config = test_config(&[("alias.demo", "orch provision --os alma 9 | P uid")]);

        let parsed = parse_command_tokens_with_aliases(
            &["demo".to_string(), "|".to_string(), "alice".to_string()],
            &config,
        )
        .expect("alias should expand");

        assert_eq!(
            parsed.tokens,
            vec![
                "orch".to_string(),
                "provision".to_string(),
                "--os".to_string(),
                "alma9".to_string()
            ]
        );
        assert_eq!(
            parsed.stages,
            vec!["P uid".to_string(), "alice".to_string()]
        );
    }

    #[test]
    fn parse_command_text_with_aliases_splits_shell_words_and_dsl() {
        let config = test_config(&[]);
        let parsed = parse_command_text_with_aliases("ldap user \"alice smith\" | P uid", &config)
            .expect("command text should parse");

        assert_eq!(
            parsed.tokens,
            vec![
                "ldap".to_string(),
                "user".to_string(),
                "alice smith".to_string()
            ]
        );
        assert_eq!(parsed.stages, vec!["P uid".to_string()]);
    }

    #[test]
    fn validate_cli_dsl_stages_rejects_unknown_verbs() {
        let err =
            validate_cli_dsl_stages(&["R uid".to_string()]).expect_err("unknown verb should fail");
        assert!(err.to_string().contains("Unknown DSL verb"));
    }

    #[test]
    fn alias_placeholders_support_positional_defaults_and_star_quoting() {
        let config = test_config(&[]);

        let expanded = expand_alias_template(
            "demo",
            "echo ${1} ${2:guest} ${*}",
            &[
                "alice".to_string(),
                "two words".to_string(),
                "O'Neil".to_string(),
            ],
            &config,
        )
        .expect("alias should expand");

        assert_eq!(
            expanded,
            "echo alice two words alice 'two words' 'O'\"'\"'Neil'"
        );
    }

    #[test]
    fn alias_placeholder_syntax_errors_are_reported_cleanly() {
        let config = test_config(&[]);

        let err = expand_alias_template("demo", "echo ${}", &[], &config)
            .expect_err("empty placeholder should fail");
        assert!(err.to_string().contains("invalid alias placeholder syntax"));

        let err = expand_alias_template("demo", "echo ${user", &[], &config)
            .expect_err("unterminated placeholder should fail");
        assert!(err.to_string().contains("invalid alias placeholder syntax"));
    }

    #[test]
    fn parse_command_tokens_with_aliases_handles_empty_input() {
        let config = test_config(&[]);
        let parsed =
            parse_command_tokens_with_aliases(&[], &config).expect("empty command should parse");

        assert!(parsed.tokens.is_empty());
        assert!(parsed.stages.is_empty());
    }

    #[test]
    fn validate_cli_dsl_stages_allows_help_stage() {
        validate_cli_dsl_stages(&["H sort".to_string()]).expect("help stage should be allowed");
    }

    #[test]
    fn truncate_display_respects_utf8_boundaries() {
        assert_eq!(truncate_display("  å🙂bcdef  ", 3), "å🙂b... (7 chars)");
    }
}
