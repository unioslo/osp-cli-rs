use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const GRID_PADDING: usize = 4;
const GRID_COLUMN_WEIGHT: usize = 3;

pub(crate) struct PreparedGridList {
    pub rows: Vec<Vec<String>>,
    pub column_widths: Vec<usize>,
    pub gap: usize,
}

impl PreparedGridList {
    pub(crate) fn from_items(values: &[String], available_width: usize) -> Self {
        let (rows, column_widths) = arrange_in_grid(
            values,
            available_width,
            GRID_PADDING,
            None,
            GRID_COLUMN_WEIGHT,
        );
        Self {
            rows,
            column_widths,
            gap: GRID_PADDING,
        }
    }
}

fn arrange_in_grid(
    values: &[String],
    available_width: usize,
    grid_padding: usize,
    grid_columns: Option<usize>,
    column_weight: usize,
) -> (Vec<Vec<String>>, Vec<usize>) {
    let n = values.len();
    if n <= 1 {
        return (
            vec![values.to_vec()],
            vec![
                values
                    .first()
                    .map_or(0, |value| UnicodeWidthStr::width(value.as_str())),
            ],
        );
    }

    if let Some(forced) = grid_columns {
        return build_grid_matrix(values, forced.max(1).min(n), available_width);
    }

    let mut best_cols = 1usize;
    let mut best_score = usize::MAX;
    let mut best_widths = vec![UnicodeWidthStr::width(values[0].as_str())];
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let col_widths = compute_grid_column_widths(values, cols, rows, available_width);
        let total_width = col_widths.iter().sum::<usize>() + grid_padding * cols.saturating_sub(1);
        if total_width > available_width {
            break;
        }

        let score = rows.abs_diff(cols * column_weight);
        if score <= best_score {
            best_score = score;
            best_cols = cols;
            best_widths = col_widths;
        }
    }

    let rows = n.div_ceil(best_cols);
    let matrix = build_grid_rows(values, best_cols, rows, available_width);
    (matrix, best_widths)
}

fn build_grid_matrix(
    values: &[String],
    columns: usize,
    available_width: usize,
) -> (Vec<Vec<String>>, Vec<usize>) {
    let rows = values.len().div_ceil(columns);
    let widths = compute_grid_column_widths(values, columns, rows, available_width);
    let matrix = build_grid_rows(values, columns, rows, available_width);
    (matrix, widths)
}

fn compute_grid_column_widths(
    values: &[String],
    columns: usize,
    rows: usize,
    available_width: usize,
) -> Vec<usize> {
    let mut column_widths = vec![0usize; columns];
    for (index, value) in values.iter().enumerate() {
        let column_index = index / rows;
        if column_index >= columns {
            continue;
        }
        let truncated = truncate_display_width_crop(value, available_width.max(4));
        column_widths[column_index] =
            column_widths[column_index].max(UnicodeWidthStr::width(truncated.as_str()));
    }
    column_widths
}

fn build_grid_rows(
    values: &[String],
    columns: usize,
    rows: usize,
    available_width: usize,
) -> Vec<Vec<String>> {
    let mut matrix = vec![vec![String::new(); columns]; rows];
    for (index, value) in values.iter().enumerate() {
        let row_index = index % rows;
        let column_index = index / rows;
        if column_index >= columns {
            continue;
        }
        matrix[row_index][column_index] =
            truncate_display_width_crop(value, available_width.max(4));
    }

    matrix
        .into_iter()
        .filter(|row| row.iter().any(|cell| !cell.is_empty()))
        .collect()
}

fn truncate_display_width_crop(value: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }

    let mut out = String::new();
    let mut width = 0usize;
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{PreparedGridList, arrange_in_grid};

    #[test]
    fn prepared_grid_list_crops_cells_to_available_width_unit() {
        let values = vec!["alphabet".to_string(), "beta".to_string()];

        let grid = PreparedGridList::from_items(&values, 4);

        assert_eq!(grid.rows[0][0], "alph");
        assert_eq!(grid.column_widths[0], 4);
        assert_eq!(grid.gap, 4);
    }

    #[test]
    fn grid_layout_handles_single_items_and_forced_columns_unit() {
        let single = PreparedGridList::from_items(&["solo".to_string()], 20);
        assert_eq!(single.rows, vec![vec!["solo".to_string()]]);
        assert_eq!(single.column_widths, vec![4]);

        let values = vec![
            "a".to_string(),
            "bb".to_string(),
            "ccc".to_string(),
            "dddd".to_string(),
        ];
        let (rows, widths) = arrange_in_grid(&values, 40, 2, Some(2), 3);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["a".to_string(), "ccc".to_string()]);
        assert_eq!(rows[1], vec!["bb".to_string(), "dddd".to_string()]);
        assert_eq!(widths, vec![2, 4]);
    }

    #[test]
    fn grid_layout_prefers_balanced_columns_when_width_allows_unit() {
        let values = vec![
            "one".to_string(),
            "two".to_string(),
            "three".to_string(),
            "four".to_string(),
        ];

        let (rows, widths) = arrange_in_grid(&values, 20, 2, None, 1);

        assert_eq!(rows.len(), 2);
        assert_eq!(widths.len(), 2);
        assert_eq!(rows[0], vec!["one".to_string(), "three".to_string()]);
        assert_eq!(rows[1], vec!["two".to_string(), "four".to_string()]);
    }
}
