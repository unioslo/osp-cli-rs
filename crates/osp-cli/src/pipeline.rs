use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::ResolvedConfig;
use osp_dsl::parse_pipeline;

const ALIAS_PREFIX: &str = "alias.";
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
        return Ok(ParsedCommandLine {
            tokens: alias_tokens,
            stages,
        });
    }

    Ok(ParsedCommandLine {
        tokens,
        stages: parsed.stages,
    })
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

    if let Ok(index) = key_part.parse::<usize>() {
        if index > 0 && index <= positional_args.len() {
            return Ok(positional_args[index - 1].clone());
        }
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
