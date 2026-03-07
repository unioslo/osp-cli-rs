use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::ResolvedConfig;
use osp_dsl::{
    model::{ParsedStage, ParsedStageKind},
    parse::pipeline::parse_stage,
    parse_pipeline,
};

const MAX_ALIAS_EXPANSION_DEPTH: usize = 100;

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
        .wrap_err("failed to parse pipeline")?;
    let command_tokens = shell_words::split(&parsed.command)
        .into_diagnostic()
        .wrap_err("failed to parse command")?;
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
    let expanded = expand_alias_template(candidate, &template, positional_args, config)?;
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

    if let Some(expanded) = maybe_expand_alias(&command_tokens[0], &command_tokens[1..], config)? {
        let alias_parsed = parse_pipeline(&expanded)
            .into_diagnostic()
            .wrap_err("failed to parse alias pipeline")?;
        let alias_tokens = shell_words::split(&alias_parsed.command)
            .into_diagnostic()
            .wrap_err("failed to parse alias command")?;
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
        let parsed = parse_stage(raw)
            .into_diagnostic()
            .wrap_err("failed to parse DSL stage")?;
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
