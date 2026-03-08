use crate::osp_core::{output_model::Group, row::Row};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value};

use crate::osp_dsl::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        matchers::match_row_keys,
        resolve::evaluate_path,
    },
    parse::{
        key_spec::KeySpec,
        path::{PathExpression, Selector, parse_path},
    },
    stages::common::parse_stage_words,
};

#[derive(Debug, Clone)]
struct Pattern {
    key_spec: KeySpec,
    path: Option<PathExpression>,
    dotted: bool,
}

pub(crate) struct ProjectPlan {
    keepers: Vec<Pattern>,
    droppers: Vec<Pattern>,
}

impl ProjectPlan {
    pub(crate) fn project_row(&self, row: &Row) -> Vec<Row> {
        project_single_row(row, &self.keepers, &self.droppers)
    }
}

pub(crate) fn compile(spec: &str) -> Result<ProjectPlan> {
    let (keepers, droppers) = parse_patterns(spec)?;
    if keepers.is_empty() && droppers.is_empty() {
        return Err(anyhow!("P requires one or more keys"));
    }

    Ok(ProjectPlan { keepers, droppers })
}

pub fn apply(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let plan = compile(spec)?;

    let mut out = Vec::new();
    for row in rows {
        out.extend(plan.project_row(&row));
    }
    Ok(out)
}

pub fn apply_groups(groups: Vec<Group>, spec: &str) -> Result<Vec<Group>> {
    let plan = compile(spec)?;

    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let mut projected_rows = Vec::new();
        for row in &group.rows {
            projected_rows.extend(plan.project_row(row));
        }

        if !projected_rows.is_empty() || !group.aggregates.is_empty() {
            out.push(Group {
                groups: group.groups,
                aggregates: group.aggregates,
                rows: projected_rows,
            });
        }
    }
    Ok(out)
}

fn parse_patterns(spec: &str) -> Result<(Vec<Pattern>, Vec<Pattern>)> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut keepers = Vec::new();
    let mut droppers = Vec::new();
    for token in parse_stage_words(trimmed)? {
        for chunk in token.split(',') {
            let text = chunk.trim();
            if text.is_empty() {
                continue;
            }

            let drop = text.starts_with('!');
            let key_spec = KeySpec::parse(text);
            let path = parse_path(&key_spec.token).ok();
            let dotted = key_spec.token.contains('.')
                || key_spec.token.contains('[')
                || key_spec.token.contains(']')
                || path
                    .as_ref()
                    .is_some_and(|expr| expr.absolute || has_selectors(expr));

            let pattern = Pattern {
                key_spec,
                path,
                dotted,
            };

            if drop {
                droppers.push(pattern);
            } else {
                keepers.push(pattern);
            }
        }
    }

    Ok((keepers, droppers))
}

fn project_single_row(row: &Row, keepers: &[Pattern], droppers: &[Pattern]) -> Vec<Row> {
    let flattened = flatten_row(row);
    let nested = Value::Object(row.clone());

    let mut static_flat = if keepers.is_empty() {
        flattened.clone()
    } else {
        Map::new()
    };
    let mut dynamic_columns: Vec<(String, Vec<Value>)> = Vec::new();

    for pattern in keepers {
        if pattern.dotted && collect_dynamic_column(&nested, &mut dynamic_columns, pattern) {
            continue;
        }

        for key in matched_flat_keys(&flattened, pattern) {
            if let Some(value) = flattened.get(&key) {
                static_flat.insert(key, value.clone());
            }
        }
    }

    for pattern in droppers {
        dynamic_columns.retain(|(dynamic_label, _)| !dynamic_label_matches(pattern, dynamic_label));

        for key in matched_flat_keys(&flattened, pattern) {
            static_flat.remove(&key);
        }
    }

    let mut rows = build_rows_from_dynamic(static_flat, dynamic_columns);
    if rows.is_empty() && keepers.is_empty() {
        rows.push(coalesce_flat_row(&Map::new()));
    }
    rows
}

fn build_rows_from_dynamic(
    static_flat: Row,
    dynamic_columns: Vec<(String, Vec<Value>)>,
) -> Vec<Row> {
    if dynamic_columns.is_empty() {
        if static_flat.is_empty() {
            return Vec::new();
        }
        return vec![coalesce_flat_row(&static_flat)];
    }

    let row_count = dynamic_columns
        .iter()
        .map(|(_, values)| values.len())
        .max()
        .unwrap_or(0);
    if row_count == 0 {
        return if static_flat.is_empty() {
            Vec::new()
        } else {
            vec![coalesce_flat_row(&static_flat)]
        };
    }

    let mut rows = Vec::new();
    for index in 0..row_count {
        let mut flat = static_flat.clone();
        for (label, values) in &dynamic_columns {
            if let Some(value) = values.get(index) {
                match value {
                    Value::Object(map) => {
                        for (key, nested_value) in map {
                            flat.insert(key.clone(), nested_value.clone());
                        }
                    }
                    scalar => {
                        flat.insert(label.clone(), scalar.clone());
                    }
                }
            } else {
                flat.insert(label.clone(), Value::Null);
            }
        }

        let projected = coalesce_flat_row(&flat);
        if !projected.is_empty() {
            rows.push(projected);
        }
    }

    rows
}

fn collect_dynamic_column(
    nested_row: &Value,
    dynamic_columns: &mut Vec<(String, Vec<Value>)>,
    pattern: &Pattern,
) -> bool {
    let Some(path) = &pattern.path else {
        return false;
    };

    if !has_selectors(path) {
        return false;
    }

    let values = evaluate_path(nested_row, path);
    if values.is_empty() {
        return false;
    }

    let label = pattern_label(pattern);
    dynamic_columns.push((label, values));
    true
}

fn matched_flat_keys(flat_row: &Row, pattern: &Pattern) -> Vec<String> {
    if let Some(path) = &pattern.path
        && path.absolute
        && !has_selectors(path)
    {
        let exact = flatten_path_without_absolute(path);
        if exact.is_empty() {
            return Vec::new();
        }
        return flat_row
            .keys()
            .filter(|key| *key == &exact)
            .cloned()
            .collect();
    }

    match_row_keys(flat_row, &pattern.key_spec.token, pattern.key_spec.exact)
        .into_iter()
        .map(ToOwned::to_owned)
        .collect()
}

fn flatten_path_without_absolute(path: &PathExpression) -> String {
    let mut out = String::new();
    for (segment_index, segment) in path.segments.iter().enumerate() {
        if segment_index > 0 {
            out.push('.');
        }
        if let Some(name) = &segment.name {
            out.push_str(name);
        } else {
            return String::new();
        }
        for selector in &segment.selectors {
            match selector {
                Selector::Index(index) if *index >= 0 => {
                    out.push('[');
                    out.push_str(&index.to_string());
                    out.push(']');
                }
                _ => return String::new(),
            }
        }
    }
    out
}

fn has_selectors(path: &PathExpression) -> bool {
    path.segments
        .iter()
        .any(|segment| !segment.selectors.is_empty())
}

fn pattern_label(pattern: &Pattern) -> String {
    if let Some(path) = &pattern.path
        && let Some(segment) = path.segments.last()
        && let Some(name) = &segment.name
    {
        return name.clone();
    }

    let token = pattern.key_spec.token.as_str();
    let last = token.rsplit('.').next().unwrap_or(token);
    let head = last.split('[').next().unwrap_or(last);
    if head.is_empty() {
        "value".to_string()
    } else {
        head.to_string()
    }
}

fn dynamic_label_matches(pattern: &Pattern, label: &str) -> bool {
    if pattern_label(pattern) == label {
        return true;
    }

    let mut row = Row::new();
    row.insert(label.to_string(), Value::Null);
    !match_row_keys(&row, &pattern.key_spec.token, pattern.key_spec.exact).is_empty()
}

#[cfg(test)]
mod tests {
    use crate::osp_core::output_model::Group;
    use serde_json::json;

    use super::{apply, apply_groups};

    #[test]
    fn keeps_requested_columns() {
        let rows = vec![
            json!({"uid": "oistes", "cn": "Oistein", "mail": "o@uio.no"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, "uid cn").expect("project should work");
        assert!(projected[0].contains_key("uid"));
        assert!(projected[0].contains_key("cn"));
        assert!(!projected[0].contains_key("mail"));
    }

    #[test]
    fn drops_column_with_prefix() {
        let rows = vec![
            json!({"uid": "oistes", "status": "active"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, "!status").expect("project should work");
        assert!(projected[0].contains_key("uid"));
        assert!(!projected[0].contains_key("status"));
    }

    #[test]
    fn supports_selector_fanout() {
        let rows = vec![
            json!({
                "interfaces": [
                    {"mac": "aa:bb"},
                    {"mac": "cc:dd"}
                ]
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let projected = apply(rows, "interfaces[].mac").expect("project should work");
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].get("mac"), Some(&json!("aa:bb")));
        assert_eq!(projected[1].get("mac"), Some(&json!("cc:dd")));
    }

    #[test]
    fn keeps_all_exact_nested_matches() {
        let rows = vec![
            json!({
                "id": 55753,
                "txts": {"id": 27994},
                "ipaddresses": [{"id": 57171}, {"id": 57172}],
                "metadata": {"asset": {"id": 42}}
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let projected = apply(rows, "id").expect("project should work");
        assert_eq!(
            projected,
            vec![
                json!({
                    "id": 55753,
                    "txts": {"id": 27994},
                    "ipaddresses": [{"id": 57171}, {"id": 57172}],
                    "metadata": {"asset": {"id": 42}}
                })
                .as_object()
                .cloned()
                .expect("object")
            ]
        );
    }

    #[test]
    fn absolute_path_projection_keeps_only_exact_nested_key() {
        let rows = vec![
            json!({"id": 1, "nested": {"id": 2}, "other": {"id": 3}})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, ".nested.id").expect("project should work");
        assert_eq!(
            projected,
            vec![
                json!({"nested": {"id": 2}})
                    .as_object()
                    .cloned()
                    .expect("object")
            ]
        );
    }

    #[test]
    fn apply_groups_keeps_aggregate_only_groups_even_when_rows_drop_out() {
        let groups = vec![Group {
            groups: json!({"dept": "eng"}).as_object().cloned().expect("object"),
            aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
            rows: vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object"),
                json!({"uid": "bob"}).as_object().cloned().expect("object"),
            ],
        }];

        let projected = apply_groups(groups, "missing").expect("group project should work");
        assert_eq!(projected.len(), 1);
        assert!(projected[0].rows.is_empty());
        assert_eq!(projected[0].aggregates.get("count"), Some(&json!(2)));
    }

    #[test]
    fn empty_project_spec_is_rejected() {
        let err = apply(
            vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
            "   ",
        )
        .expect_err("empty spec should fail");

        assert!(err.to_string().contains("requires one or more keys"));
    }

    #[test]
    fn dropping_dynamic_projection_label_removes_fanout_column() {
        let rows = vec![
            json!({
                "uid": "alice",
                "interfaces": [{"mac": "aa:bb"}, {"mac": "cc:dd"}]
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let projected = apply(rows, "uid interfaces[].mac !mac").expect("project should work");
        assert_eq!(
            projected,
            vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object")
            ]
        );
    }
}
