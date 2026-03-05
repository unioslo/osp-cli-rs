use osp_core::{
    output_model::{Group, OutputItems},
    row::Row,
};

pub fn apply(items: OutputItems) -> OutputItems {
    match items {
        OutputItems::Rows(rows) => OutputItems::Rows(rows),
        OutputItems::Groups(groups) => {
            let collapsed = groups.into_iter().map(collapse_group).collect();
            OutputItems::Rows(collapsed)
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
    use osp_core::output_model::{Group, OutputItems};
    use serde_json::json;

    use super::apply;

    #[test]
    fn passes_through_rows_unchanged() {
        let rows = vec![
            json!({"uid": "oistes"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows.clone()));
        assert_eq!(output, OutputItems::Rows(rows));
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

        let output = apply(OutputItems::Groups(groups));
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
