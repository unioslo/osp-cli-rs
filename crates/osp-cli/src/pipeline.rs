use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::ResolvedConfig;
use osp_dsl::parse_pipeline;

const ALIAS_PREFIX: &str = "alias.";
const MAX_ALIAS_EXPANSION_DEPTH: usize = 100;
const VALID_SINGLE_LETTER_PIPE_VERBS: [&str; 10] =
    ["F", "P", "S", "G", "A", "L", "Z", "C", "Y", "H"];
const QUICK_PIPE_PREFIXES: [&str; 2] = ["K", "V"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommandLine {
    pub tokens: Vec<String>,
    pub stages: Vec<String>,
}

pub fn parse_command_text_with_aliases(
    text: &str,
    config: &ResolvedConfig,
) -> Result<ParsedCommandLine> {
    let parsed = parse_pipeline(text);
    let tokens = shell_words::split(&parsed.command)
        .into_diagnostic()
        .wrap_err("failed to parse command")?;
    if tokens.is_empty() {
        return Ok(ParsedCommandLine {
            tokens: Vec::new(),
            stages: Vec::new(),
        });
    }

    if let Some(expanded) = maybe_expand_alias(&tokens[0], &tokens[1..], config)? {
        let alias_parsed = parse_pipeline(&expanded);
        let alias_tokens = shell_words::split(&alias_parsed.command)
            .into_diagnostic()
            .wrap_err("failed to parse alias command")?;
        if alias_tokens.is_empty() {
            return Ok(ParsedCommandLine {
                tokens: Vec::new(),
                stages: Vec::new(),
            });
        }
        let mut stages = alias_parsed.stages;
        stages.extend(parsed.stages);
        return finalize_parsed_command(alias_tokens, stages);
    }

    finalize_parsed_command(tokens, parsed.stages)
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

    let mut rendered = Vec::with_capacity(tokens.len());
    for token in tokens {
        if token == "|" {
            rendered.push("|".to_string());
        } else {
            rendered.push(quote_token(token));
        }
    }
    let line = rendered.join(" ");
    parse_command_text_with_aliases(&line, config)
}

fn maybe_expand_alias(
    candidate: &str,
    positional_args: &[String],
    config: &ResolvedConfig,
) -> Result<Option<String>> {
    let key = alias_key(candidate);
    let Some(value) = config.get(&key) else {
        return Ok(None);
    };

    let template = value.to_string();
    let expanded = expand_alias_template(candidate, &template, positional_args, config)?;
    Ok(Some(expanded))
}

fn finalize_parsed_command(tokens: Vec<String>, stages: Vec<String>) -> Result<ParsedCommandLine> {
    validate_explicit_dsl_stages(&stages)?;
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

fn validate_explicit_dsl_stages(stages: &[String]) -> Result<()> {
    for raw in stages {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        let verb = trimmed.split_whitespace().next().unwrap_or_default();
        if verb.len() != 1 || !verb.chars().all(|ch| ch.is_ascii_alphabetic()) {
            continue;
        }

        let normalized = verb.to_ascii_uppercase();
        if QUICK_PIPE_PREFIXES.contains(&normalized.as_str()) {
            continue;
        }
        if VALID_SINGLE_LETTER_PIPE_VERBS.contains(&normalized.as_str()) {
            continue;
        }

        return Err(miette!(
            "Unknown DSL verb '{}' in pipe '{}'. Use `| H <verb>` for help.",
            normalized,
            trimmed
        ));
    }

    Ok(())
}

fn alias_key(candidate: &str) -> String {
    let normalized = candidate.trim().to_ascii_lowercase();
    if normalized.starts_with(ALIAS_PREFIX) {
        normalized
    } else {
        format!("{ALIAS_PREFIX}{normalized}")
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
