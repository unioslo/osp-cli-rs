use crate::core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use anyhow::{Result, anyhow};

/// Collapses grouped output into summary rows of group headers and aggregates.
///
/// Returns an error when called on flat rows.
pub fn apply(items: OutputItems) -> Result<OutputItems> {
    match items {
        OutputItems::Rows(_) => Err(anyhow!("Z requires grouped output; use G before Z")),
        OutputItems::Groups(groups) => {
            let collapsed = groups.into_iter().map(collapse_group).collect();
            Ok(OutputItems::Rows(collapsed))
        }
    }
}

fn collapse_group(group: Group) -> Row {
    let mut summary = Row::new();
    summary.extend(group.groups);
    summary.extend(group.aggregates);
    summary
}

#[cfg(test)]
mod tests {
    use crate::core::output_model::{Group, OutputItems};
    use serde_json::json;

    use super::apply;

    #[test]
    fn rejects_flat_rows() {
        let rows = vec![
            json!({"uid": "oistes"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let err = apply(OutputItems::Rows(rows)).expect_err("collapse should reject flat rows");
        assert!(err.to_string().contains("requires grouped output"));
    }

    #[test]
    fn collapses_groups_into_summary_rows() {
        let groups = vec![Group {
            groups: json!({"dept": "Sales"})
                .as_object()
                .cloned()
                .expect("object"),
            aggregates: json!({"member_count": 2})
                .as_object()
                .cloned()
                .expect("object"),
            rows: Vec::new(),
        }];

        let output = apply(OutputItems::Groups(groups)).expect("group collapse should work");
        match output {
            OutputItems::Rows(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].get("dept").and_then(|v| v.as_str()), Some("Sales"));
                assert_eq!(
                    rows[0].get("member_count").and_then(|v| v.as_i64()),
                    Some(2)
                );
            }
            OutputItems::Groups(_) => panic!("collapse must return rows"),
        }
    }
}
