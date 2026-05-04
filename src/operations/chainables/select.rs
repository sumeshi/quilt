use crate::controllers::log::LogController;
use polars::prelude::*;

pub fn select(df: &LazyFrame, colnames: &[String]) -> LazyFrame {
    let schema = match df.clone().collect_schema() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error getting schema for select operation: {e}");
            std::process::exit(1);
        }
    };

    let available_columns: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();

    // Expand column names to handle colon notation, quoted colon notation, and numeric indices
    let mut expanded_colnames = Vec::new();
    for colname in colnames {
        if colname.contains(':') && !colname.starts_with('"') {
            // Check if it's a numeric range (e.g., "1:3")
            if is_numeric_range(colname) {
                let range_cols = parse_numeric_range(colname, &available_columns);
                expanded_colnames.extend(range_cols);
            } else {
                // Handle regular colon-separated range (col1:col3)
                let range_cols = parse_colon_range(colname, &available_columns);
                expanded_colnames.extend(range_cols);
            }
        } else if colname.starts_with('"') && colname.contains(":") && colname.ends_with('"') {
            // Handle quoted colon notation: "col1":"col3"
            let inner = &colname[1..colname.len() - 1]; // Remove outer quotes
            if let Some((start_col, end_col)) = inner.split_once(":") {
                let range_cols = parse_quoted_colon_range(start_col, end_col, &available_columns);
                expanded_colnames.extend(range_cols);
            } else {
                expanded_colnames.push(colname.clone());
            }
        } else {
            // Check if it's a single numeric index
            if is_numeric_index(colname) {
                if let Some(col_name) = parse_single_numeric_index(colname, &available_columns) {
                    expanded_colnames.push(col_name);
                } else {
                    eprintln!("Error: Invalid column index '{colname}'");
                    std::process::exit(1);
                }
            } else {
                expanded_colnames.push(colname.clone());
            }
        }
    }

    // Validate all expanded column names exist
    for colname in &expanded_colnames {
        if !schema.iter_names().any(|s| s == colname) {
            eprintln!("Error: Column '{colname}' not found in DataFrame for select operation");
            std::process::exit(1);
        }
    }

    let mut selected_cols: Vec<Expr> = Vec::new();
    for name in &expanded_colnames {
        if available_columns.contains(name) {
            selected_cols.push(col(name));
        } else {
            LogController::warn(&format!("Column '{name}' not found in DataFrame."));
        }
    }

    if selected_cols.is_empty() {
        LogController::warn("No valid columns selected. Returning original DataFrame.");
        return df.clone();
    }

    df.clone().select(&selected_cols)
}
// Helper function to check if a string is a numeric index
fn is_numeric_index(s: &str) -> bool {
    s.parse::<usize>().is_ok()
}
// Helper function to check if a string is a numeric range (e.g., "1:3")
fn is_numeric_range(s: &str) -> bool {
    if let Some((start, end)) = s.split_once(':') {
        start.trim().parse::<usize>().is_ok() && end.trim().parse::<usize>().is_ok()
    } else {
        false
    }
}
// Helper function to parse a single numeric index to column name
fn parse_single_numeric_index(index_str: &str, available_columns: &[String]) -> Option<String> {
    if let Ok(index) = index_str.parse::<usize>() {
        if index >= 1 && index <= available_columns.len() {
            Some(available_columns[index - 1].clone()) // Convert 1-based to 0-based
        } else {
            None
        }
    } else {
        None
    }
}
// Helper function to parse numeric ranges (1:3)
fn parse_numeric_range(range_str: &str, available_columns: &[String]) -> Vec<String> {
    if let Some((start_str, end_str)) = range_str.split_once(':') {
        let start_str = start_str.trim();
        let end_str = end_str.trim();
        if let (Ok(start_idx), Ok(end_idx)) = (start_str.parse::<usize>(), end_str.parse::<usize>())
        {
            if start_idx < 1 || end_idx < 1 {
                eprintln!("Error: Column indices are 1-based. Got index 0 in range '{range_str}'.");
                std::process::exit(1);
            }
            // Convert 1-based indices to 0-based
            let start_zero_based = start_idx - 1;
            let end_zero_based = end_idx - 1;
            if start_zero_based < available_columns.len()
                && end_zero_based < available_columns.len()
                && start_zero_based <= end_zero_based
            {
                return available_columns[start_zero_based..=end_zero_based].to_vec();
            } else {
                LogController::warn(&format!(
                    "Invalid numeric range: indices out of bounds or invalid order: {range_str}"
                ));
            }
        } else {
            LogController::warn(&format!("Invalid numeric range format: {range_str}"));
        }
    }
    // If parsing fails, return empty vector
    vec![]
}
// Helper function to parse colon-separated ranges (col1:col3)
pub fn parse_colon_range(range_str: &str, available_columns: &[String]) -> Vec<String> {
    if let Some((start_col, end_col)) = range_str.split_once(':') {
        let start_col = start_col.trim();
        let end_col = end_col.trim();
        let start_idx = available_columns.iter().position(|c| c == start_col);
        let end_idx = available_columns.iter().position(|c| c == end_col);

        match (start_idx, end_idx) {
            (Some(start_idx), Some(end_idx)) => {
                if start_idx <= end_idx {
                    return available_columns[start_idx..=end_idx].to_vec();
                }
                LogController::warn(&format!(
                    "Invalid range: '{start_col}' comes after '{end_col}' in column order"
                ));
            }
            (None, _) => {
                eprintln!(
                    "Error: Column '{start_col}' not found in DataFrame for select operation"
                );
                std::process::exit(1);
            }
            (_, None) => {
                eprintln!("Error: Column '{end_col}' not found in DataFrame for select operation");
                std::process::exit(1);
            }
        }
    }
    // If parsing fails, return an empty vector so the caller can surface the
    // missing column that caused the invalid range.
    vec![]
}
// Helper function to parse quoted colon-separated ranges ("col1":"col3")
pub fn parse_quoted_colon_range(
    start_col: &str,
    end_col: &str,
    available_columns: &[String],
) -> Vec<String> {
    // Find indices of start and end columns
    if let (Some(start_idx), Some(end_idx)) = (
        available_columns.iter().position(|c| c == start_col),
        available_columns.iter().position(|c| c == end_col),
    ) {
        if start_idx <= end_idx {
            return available_columns[start_idx..=end_idx].to_vec();
        } else {
            LogController::warn(&format!(
                "Invalid quoted range: '{start_col}' comes after '{end_col}' in column order"
            ));
        }
    } else {
        LogController::warn(&format!(
            "Quoted column range '\"{start_col}\":\"{end_col}\"' contains invalid column names"
        ));
    }
    // If parsing fails, return the original column names
    vec![start_col.to_string(), end_col.to_string()]
}
