use anyhow::{Result, anyhow};

use crate::dsl::parse::lexer::{Span, StageSegment, tokenize_stage};

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
    Ok(tokens.into_iter().map(|token| token.text).collect())
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

#[cfg(test)]
mod tests {
    use super::{
        parse_alias_after_as, parse_optional_alias_after_key, parse_stage_words, parse_terms,
    };

    #[test]
    fn parse_terms_splits_commas_and_whitespace() {
        assert_eq!(
            parse_terms(" uid, cn  mail,,groups "),
            vec!["uid", "cn", "mail", "groups"]
        );
    }

    #[test]
    fn parse_stage_words_handles_empty_and_quoted_input() {
        assert_eq!(
            parse_stage_words("   ").expect("empty spec should parse"),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_stage_words("uid \"display name\"").expect("quoted words should parse"),
            vec!["uid".to_string(), "display name".to_string()]
        );
    }

    #[test]
    fn alias_parsers_cover_valid_and_invalid_as_forms() {
        let words = vec!["count".to_string(), "AS".to_string(), "total".to_string()];
        assert_eq!(
            parse_optional_alias_after_key(&words, 0, "A").expect("alias parse should work"),
            (Some("total".to_string()), 3)
        );
        assert_eq!(
            parse_alias_after_as(&words, 1, "A").expect("alias parse should work"),
            Some("total".to_string())
        );
        assert_eq!(
            parse_alias_after_as(&words, 0, "A").expect("non-AS token should return none"),
            None
        );

        let err = parse_optional_alias_after_key(&["AS".to_string()], 0, "A")
            .expect_err("leading AS should fail");
        assert!(err.to_string().contains("AS must follow a key"));

        let err = parse_alias_after_as(&["AS".to_string()], 0, "A")
            .expect_err("missing alias should fail");
        assert!(err.to_string().contains("missing alias after AS"));
    }

    #[test]
    fn parse_stage_words_reports_lexer_errors() {
        let err = parse_stage_words("\"unterminated").expect_err("unterminated quote should fail");
        assert!(
            err.to_string().contains("unterminated")
                || err.to_string().contains("expected closing quote")
        );
    }

    #[test]
    fn optional_alias_parser_returns_none_when_alias_is_absent_or_index_missing() {
        let words = vec!["count".to_string(), "group".to_string()];
        assert_eq!(
            parse_optional_alias_after_key(&words, 0, "A").expect("plain key should parse"),
            (None, 1)
        );
        assert_eq!(
            parse_optional_alias_after_key(&words, 5, "A").expect("missing index should parse"),
            (None, 0)
        );
    }
}
