use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    ConfigError, ConfigValue, ExplainInterpolation, ExplainInterpolationStep, ResolvedValue,
};

#[derive(Debug, Clone)]
struct ParsedTemplate {
    raw: String,
    placeholders: Vec<PlaceholderSpan>,
}

#[derive(Debug, Clone)]
struct PlaceholderSpan {
    start: usize,
    end: usize,
    name: String,
}

/// Placeholder expansion is intentionally isolated from scope/source selection.
///
/// By the time interpolation runs, the resolver has already chosen one raw
/// value per key. That keeps interpolation deterministic and lets explain
/// output report the same placeholder chain the normal resolution path used.
struct Interpolator {
    raw: HashMap<String, ConfigValue>,
    cache: HashMap<String, ConfigValue>,
}

impl Interpolator {
    fn from_resolved_values(values: &BTreeMap<String, ResolvedValue>) -> Self {
        Self {
            raw: values
                .iter()
                .map(|(key, value)| (key.clone(), value.raw_value.clone()))
                .collect(),
            cache: HashMap::new(),
        }
    }

    fn apply_all(
        &mut self,
        values: &mut BTreeMap<String, ResolvedValue>,
    ) -> Result<(), ConfigError> {
        let keys = values.keys().cloned().collect::<Vec<String>>();
        for key in keys {
            let value = self.resolve_value(&key, &mut Vec::new())?;
            if let Some(entry) = values.get_mut(&key) {
                entry.value = value;
            }
        }

        Ok(())
    }

    fn explain(
        &self,
        key: &str,
        pre_interpolated: &BTreeMap<String, ResolvedValue>,
        final_values: &BTreeMap<String, ResolvedValue>,
    ) -> Result<Option<ExplainInterpolation>, ConfigError> {
        let Some(template) = self.parsed_template(key)? else {
            return Ok(None);
        };

        let mut steps = Vec::new();
        let mut seen = BTreeSet::new();
        self.collect_steps_recursive(
            key,
            pre_interpolated,
            final_values,
            &mut steps,
            &mut seen,
            &mut Vec::new(),
        )?;

        Ok(Some(ExplainInterpolation {
            template: template.raw,
            steps,
        }))
    }

    fn resolve_value(
        &mut self,
        key: &str,
        stack: &mut Vec<String>,
    ) -> Result<ConfigValue, ConfigError> {
        if let Some(value) = self.cache.get(key) {
            return Ok(value.clone());
        }

        if let Some(index) = stack.iter().position(|item| item == key) {
            let mut cycle = stack[index..].to_vec();
            cycle.push(key.to_string());
            return Err(ConfigError::PlaceholderCycle { cycle });
        }

        let value =
            self.raw
                .get(key)
                .cloned()
                .ok_or_else(|| ConfigError::UnresolvedPlaceholder {
                    key: key.to_string(),
                    placeholder: key.to_string(),
                })?;

        if key.starts_with("alias.") {
            self.cache.insert(key.to_string(), value.clone());
            return Ok(value);
        }

        stack.push(key.to_string());

        let resolved = match value {
            ConfigValue::Secret(secret) => match secret.into_inner() {
                ConfigValue::String(template) => {
                    let (interpolated, _contains_secret) =
                        self.interpolate_template(key, parse_template(key, &template)?, stack)?;
                    ConfigValue::String(interpolated).into_secret()
                }
                other => other.into_secret(),
            },
            ConfigValue::String(template) => {
                let (interpolated, contains_secret) =
                    self.interpolate_template(key, parse_template(key, &template)?, stack)?;
                let value = ConfigValue::String(interpolated);
                if contains_secret {
                    value.into_secret()
                } else {
                    value
                }
            }
            other => other,
        };

        stack.pop();
        self.cache.insert(key.to_string(), resolved.clone());

        Ok(resolved)
    }

    fn interpolate_template(
        &mut self,
        key: &str,
        template: ParsedTemplate,
        stack: &mut Vec<String>,
    ) -> Result<(String, bool), ConfigError> {
        if template.placeholders.is_empty() {
            return Ok((template.raw, false));
        }

        let mut out = String::new();
        let mut cursor = 0usize;
        let mut contains_secret = false;

        for placeholder in &template.placeholders {
            out.push_str(&template.raw[cursor..placeholder.start]);
            let resolved = self.resolve_placeholder(key, &placeholder.name, stack)?;
            if resolved.is_secret() {
                contains_secret = true;
            }
            out.push_str(&resolved.as_interpolation_string(key, &placeholder.name)?);
            cursor = placeholder.end;
        }

        out.push_str(&template.raw[cursor..]);
        Ok((out, contains_secret))
    }

    fn parsed_template(&self, key: &str) -> Result<Option<ParsedTemplate>, ConfigError> {
        if key.starts_with("alias.") {
            return Ok(None);
        }

        let Some(ConfigValue::String(template)) = self.raw.get(key).map(ConfigValue::reveal) else {
            return Ok(None);
        };
        let parsed = parse_template(key, template)?;
        Ok((!parsed.placeholders.is_empty()).then_some(parsed))
    }

    fn resolve_placeholder(
        &mut self,
        key: &str,
        placeholder: &str,
        stack: &mut Vec<String>,
    ) -> Result<ConfigValue, ConfigError> {
        if !self.raw.contains_key(placeholder) {
            return Err(ConfigError::UnresolvedPlaceholder {
                key: key.to_string(),
                placeholder: placeholder.to_string(),
            });
        }

        self.resolve_value(placeholder, stack)
    }

    fn collect_steps_recursive(
        &self,
        key: &str,
        pre_interpolated: &BTreeMap<String, ResolvedValue>,
        final_values: &BTreeMap<String, ResolvedValue>,
        steps: &mut Vec<ExplainInterpolationStep>,
        seen: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<(), ConfigError> {
        let Some(template) = self.parsed_template(key)? else {
            return Ok(());
        };

        if let Some(index) = stack.iter().position(|item| item == key) {
            let mut cycle = stack[index..].to_vec();
            cycle.push(key.to_string());
            return Err(ConfigError::PlaceholderCycle { cycle });
        }

        stack.push(key.to_string());
        for placeholder in &template.placeholders {
            if !self.raw.contains_key(&placeholder.name) {
                return Err(ConfigError::UnresolvedPlaceholder {
                    key: key.to_string(),
                    placeholder: placeholder.name.clone(),
                });
            }

            if seen.insert(placeholder.name.clone())
                && let (Some(raw_entry), Some(final_entry)) = (
                    pre_interpolated.get(&placeholder.name),
                    final_values.get(&placeholder.name),
                )
            {
                steps.push(ExplainInterpolationStep {
                    placeholder: placeholder.name.clone(),
                    raw_value: raw_entry.raw_value.clone(),
                    value: final_entry.value.clone(),
                    source: raw_entry.source,
                    scope: raw_entry.scope.clone(),
                    origin: raw_entry.origin.clone(),
                });
            }

            self.collect_steps_recursive(
                &placeholder.name,
                pre_interpolated,
                final_values,
                steps,
                seen,
                stack,
            )?;
        }
        stack.pop();

        Ok(())
    }
}

pub(crate) fn interpolate_all(
    values: &mut BTreeMap<String, ResolvedValue>,
) -> Result<(), ConfigError> {
    Interpolator::from_resolved_values(values).apply_all(values)
}

pub(crate) fn explain_interpolation(
    key: &str,
    pre_interpolated: &BTreeMap<String, ResolvedValue>,
    final_values: &BTreeMap<String, ResolvedValue>,
) -> Result<Option<ExplainInterpolation>, ConfigError> {
    // Explain traces follow the raw selected template graph, but each
    // placeholder step also records the final adapted value so callers can see
    // where type/schema changes happened after interpolation.
    Interpolator::from_resolved_values(pre_interpolated).explain(
        key,
        pre_interpolated,
        final_values,
    )
}

/// Parse `${key}` segments once so interpolation and explain tracing can share
/// the same validated template shape.
fn parse_template(key: &str, template: &str) -> Result<ParsedTemplate, ConfigError> {
    let mut placeholders = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = template[cursor..].find("${") {
        let start = cursor + rel_start;
        let after_open = start + 2;
        let Some(rel_end) = template[after_open..].find('}') else {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        };
        let end = after_open + rel_end;

        let placeholder = template[after_open..end].trim();
        if placeholder.is_empty() {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        }

        placeholders.push(PlaceholderSpan {
            start,
            end: end + 1,
            name: placeholder.to_string(),
        });
        cursor = end + 1;
    }

    Ok(ParsedTemplate {
        raw: template.to_string(),
        placeholders,
    })
}
