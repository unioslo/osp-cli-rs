use anyhow::{Result, anyhow};

use crate::parse::lexer::{Span, StageSegment, TokenKind, tokenize_stage};

pub fn parse_terms(spec: &str) -> Vec<String> {
    spec.split(|ch: char| ch == ',' || ch.is_whitespace())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn parse_stage_words(spec: &str) -> Result<Vec<String>> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let segment = StageSegment {
        raw: trimmed.to_string(),
        span: Span {
            start: 0,
            end: trimmed.len(),
        },
    };
    let tokens = tokenize_stage(&segment).map_err(|error| anyhow!(error.to_string()))?;
    Ok(tokens
        .into_iter()
        .filter_map(|token| match token.kind {
            TokenKind::Word | TokenKind::Op(_) => Some(token.text),
        })
        .collect())
}

pub fn parse_optional_alias_after_key(
    words: &[String],
    index: usize,
    verb: &str,
) -> Result<(Option<String>, usize)> {
    let Some(token) = words.get(index) else {
        return Ok((None, 0));
    };
    if token.eq_ignore_ascii_case("AS") {
        return Err(anyhow!("{verb}: AS must follow a key"));
    }
    if index + 2 < words.len() && words[index + 1].eq_ignore_ascii_case("AS") {
        return Ok((Some(words[index + 2].clone()), 3));
    }
    Ok((None, 1))
}

pub fn parse_alias_after_as(words: &[String], index: usize, verb: &str) -> Result<Option<String>> {
    let Some(token) = words.get(index) else {
        return Ok(None);
    };
    if !token.eq_ignore_ascii_case("AS") {
        return Ok(None);
    }
    let alias = words
        .get(index + 1)
        .ok_or_else(|| anyhow!("{verb}: missing alias after AS"))?;
    Ok(Some(alias.clone()))
}
