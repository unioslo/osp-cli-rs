use anyhow::{Result, anyhow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathExpression {
    pub absolute: bool,
    pub segments: Vec<PathSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSegment {
    pub name: Option<String>,
    pub selectors: Vec<Selector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Fanout,
    Index(i64),
    Slice {
        start: Option<i64>,
        stop: Option<i64>,
        step: Option<i64>,
    },
}

pub fn parse_path(input: &str) -> Result<PathExpression> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("path expression cannot be empty"));
    }

    let absolute = trimmed.starts_with('.');
    let body = if absolute { &trimmed[1..] } else { trimmed };
    if body.is_empty() {
        return Err(anyhow!("path expression cannot be only '.'"));
    }

    let raw_segments = split_path_segments(body)?;
    let mut segments = Vec::with_capacity(raw_segments.len());
    for raw_segment in raw_segments {
        segments.push(parse_segment(&raw_segment)?);
    }

    Ok(PathExpression { absolute, segments })
}

pub fn requires_materialization(path: &PathExpression) -> bool {
    path.segments.iter().any(|segment| {
        segment.selectors.iter().any(|selector| match selector {
            Selector::Index(index) => *index < 0,
            Selector::Fanout => false,
            Selector::Slice { start, stop, step } => {
                !(start.is_none() && stop.is_none() && step.is_none())
            }
        })
    })
}

pub fn expression_to_flat_key(path: &PathExpression) -> Option<String> {
    let mut output = String::new();

    for (index, segment) in path.segments.iter().enumerate() {
        if index > 0 {
            output.push('.');
        }

        if let Some(name) = &segment.name {
            output.push_str(name);
        } else {
            return None;
        }

        for selector in &segment.selectors {
            match selector {
                Selector::Index(value) if *value >= 0 => {
                    output.push('[');
                    output.push_str(&value.to_string());
                    output.push(']');
                }
                Selector::Fanout | Selector::Slice { .. } | Selector::Index(_) => return None,
            }
        }
    }

    if output.is_empty() {
        None
    } else {
        Some(output)
    }
}

fn split_path_segments(path: &str) -> Result<Vec<String>> {
    let mut depth = 0usize;
    let mut current = String::new();
    let mut segments = Vec::new();

    for ch in path.chars() {
        match ch {
            '[' => {
                depth = depth.saturating_add(1);
                current.push(ch);
            }
            ']' => {
                if depth == 0 {
                    return Err(anyhow!("unmatched ']' in path expression"));
                }
                depth -= 1;
                current.push(ch);
            }
            '.' if depth == 0 => {
                if current.is_empty() {
                    return Err(anyhow!("empty path segment"));
                }
                segments.push(current);
                current = String::new();
            }
            _ => current.push(ch),
        }
    }

    if depth != 0 {
        return Err(anyhow!("unclosed '[' in path expression"));
    }
    if current.is_empty() {
        return Err(anyhow!("empty path segment"));
    }
    segments.push(current);

    Ok(segments)
}

fn parse_segment(raw_segment: &str) -> Result<PathSegment> {
    let mut name = String::new();
    let mut selectors = Vec::new();
    let chars: Vec<char> = raw_segment.chars().collect();
    let mut index = 0usize;

    while index < chars.len() && chars[index] != '[' {
        name.push(chars[index]);
        index += 1;
    }

    let name = if name.is_empty() { None } else { Some(name) };

    while index < chars.len() {
        if chars[index] != '[' {
            return Err(anyhow!("unexpected character in path segment"));
        }
        index += 1;

        let mut content = String::new();
        while index < chars.len() && chars[index] != ']' {
            content.push(chars[index]);
            index += 1;
        }
        if index == chars.len() {
            return Err(anyhow!("unclosed '[' in path segment"));
        }
        index += 1;

        selectors.push(parse_selector(content.trim())?);
    }

    Ok(PathSegment { name, selectors })
}

fn parse_selector(content: &str) -> Result<Selector> {
    if content.is_empty() {
        return Ok(Selector::Fanout);
    }

    if content.contains(':') {
        let parts: Vec<&str> = content.split(':').collect();
        if parts.len() > 3 {
            return Err(anyhow!("slice selector has too many components"));
        }

        let start = parse_optional_i64(parts.first().copied().unwrap_or_default())?;
        let stop = parse_optional_i64(parts.get(1).copied().unwrap_or_default())?;
        let step = parse_optional_i64(parts.get(2).copied().unwrap_or_default())?;

        return Ok(Selector::Slice { start, stop, step });
    }

    let index = content
        .parse::<i64>()
        .map_err(|_| anyhow!("invalid list index: {content}"))?;
    Ok(Selector::Index(index))
}

fn parse_optional_i64(value: &str) -> Result<Option<i64>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    value
        .trim()
        .parse::<i64>()
        .map(Some)
        .map_err(|_| anyhow!("invalid integer in slice selector: {value}"))
}

#[cfg(test)]
mod tests {
    use super::{Selector, expression_to_flat_key, parse_path, requires_materialization};

    #[test]
    fn parses_dotted_path_with_selectors() {
        let path = parse_path("members[0].uid").expect("path should parse");
        assert_eq!(path.segments.len(), 2);
        assert_eq!(path.segments[0].selectors, vec![Selector::Index(0)]);
    }

    #[test]
    fn detects_materialization_for_negative_index() {
        let path = parse_path("members[-1]").expect("path should parse");
        assert!(requires_materialization(&path));
    }

    #[test]
    fn full_slice_does_not_require_materialization() {
        let path = parse_path("members[:]").expect("path should parse");
        assert!(!requires_materialization(&path));
    }

    #[test]
    fn non_full_slice_requires_materialization() {
        let path = parse_path("members[1:]").expect("path should parse");
        assert!(requires_materialization(&path));
    }

    #[test]
    fn expression_to_flat_key_accepts_positive_index_only() {
        let path = parse_path("members[0].uid").expect("path should parse");
        assert_eq!(
            expression_to_flat_key(&path),
            Some("members[0].uid".to_string())
        );

        let path = parse_path("members[-1].uid").expect("path should parse");
        assert_eq!(expression_to_flat_key(&path), None);
    }
}
