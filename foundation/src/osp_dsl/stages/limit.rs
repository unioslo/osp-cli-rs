use anyhow::{Result, anyhow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LimitSpec {
    pub(crate) count: i64,
    pub(crate) offset: i64,
}

impl LimitSpec {
    pub(crate) fn is_head_only(self) -> bool {
        self.count >= 0 && self.offset >= 0
    }
}

pub(crate) fn parse_limit_spec(spec: &str) -> Result<LimitSpec> {
    let parts: Vec<&str> = spec.split_whitespace().collect();
    if !(1..=2).contains(&parts.len()) {
        return Err(anyhow!("L expects 1 or 2 integers (limit [offset])"));
    }

    let count = parts[0]
        .parse::<i64>()
        .map_err(|_| anyhow!("L arguments must be integers"))?;
    let offset = if parts.len() == 2 {
        parts[1]
            .parse::<i64>()
            .map_err(|_| anyhow!("L arguments must be integers"))?
    } else {
        0
    };

    Ok(LimitSpec { count, offset })
}

pub fn apply<T>(items: Vec<T>, spec: &str) -> Result<Vec<T>> {
    let spec = parse_limit_spec(spec)?;
    let count = spec.count;
    let offset = spec.offset;

    if count == 0 {
        return Ok(Vec::new());
    }

    if count > 0 && offset >= 0 {
        return Ok(items
            .into_iter()
            .skip(offset as usize)
            .take(count as usize)
            .collect());
    }

    let length = items.len() as i64;
    let start = if offset >= 0 {
        offset
    } else {
        (length + offset).max(0)
    };

    let base: Vec<T> = items.into_iter().skip(start.max(0) as usize).collect();

    if count >= 0 {
        Ok(base.into_iter().take(count as usize).collect())
    } else {
        let take = count.unsigned_abs() as usize;
        let skip = base.len().saturating_sub(take);
        Ok(base.into_iter().skip(skip).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::apply;

    #[test]
    fn takes_head_for_positive_limit() {
        let rows = vec![1, 2, 3];
        let output = apply(rows, "2").expect("limit should work");
        assert_eq!(output, vec![1, 2]);
    }

    #[test]
    fn handles_zero_limit() {
        let rows = vec![1, 2, 3];
        let output = apply(rows, "0").expect("limit should work");
        assert!(output.is_empty());
    }

    #[test]
    fn supports_negative_count_for_tail() {
        let rows = vec![1, 2, 3, 4, 5];
        let output = apply(rows, "-2").expect("limit should work");
        assert_eq!(output, vec![4, 5]);
    }

    #[test]
    fn supports_positive_count_with_positive_offset() {
        let rows = vec![1, 2, 3, 4];
        let output = apply(rows, "2 1").expect("limit should work");
        assert_eq!(output, vec![2, 3]);
    }

    #[test]
    fn supports_positive_count_with_negative_offset() {
        let rows = vec![1, 2, 3, 4, 5];
        let output = apply(rows, "2 -2").expect("limit should work");
        assert_eq!(output, vec![4, 5]);
    }

    #[test]
    fn supports_negative_count_with_negative_offset() {
        let rows = vec![1, 2, 3, 4, 5];
        let output = apply(rows, "-1 -2").expect("limit should work");
        assert_eq!(output, vec![5]);
    }

    #[test]
    fn rejects_invalid_argument_count() {
        let rows = vec![1, 2, 3];
        let err = apply(rows, "1 2 3").expect_err("invalid arity should fail");
        assert!(err.to_string().contains("L expects 1 or 2 integers"));
    }
}
