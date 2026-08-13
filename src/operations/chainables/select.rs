use crate::controllers::log::LogController;
use crate::error::QuiltError;
use once_cell::sync::Lazy;
use polars::prelude::*;
use regex::Regex;

static RE_COL_RANGE_HYPHEN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?P<p1>[a-zA-Z_][a-zA-Z_0-9]*)(?P<n1>\d+)-(?:(?P<p2>[a-zA-Z_][a-zA-Z_0-9]*)(?P<n2>\d+)|(?P<n3>\d+))$").unwrap()
});

pub fn select(df: &LazyFrame, colnames: &[String]) -> Result<LazyFrame, QuiltError> {
    let schema = df.clone().collect_schema().map_err(|e| {
        QuiltError::schema(
            "select",
            None::<String>,
            format!("Error getting schema: {e}"),
        )
    })?;
    let available_columns: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();
    let mut expanded_colnames = Vec::new();

    for colname in colnames {
        if colname.contains(':') && !colname.starts_with('"') {
            let range_cols = if is_numeric_range(colname) {
                parse_numeric_range(colname, &available_columns)?
            } else {
                parse_colon_range(colname, &available_columns)?
            };
            expanded_colnames.extend(range_cols);
        } else if colname.starts_with('"') && colname.contains(':') && colname.ends_with('"') {
            let inner = &colname[1..colname.len() - 1];
            if let Some((start_col, end_col)) = inner.split_once(':') {
                expanded_colnames.extend(parse_quoted_colon_range(
                    start_col,
                    end_col,
                    &available_columns,
                )?);
            } else {
                expanded_colnames.push(colname.clone());
            }
        } else if available_columns.contains(colname) {
            expanded_colnames.push(colname.clone());
        } else if RE_COL_RANGE_HYPHEN.is_match(colname) {
            expanded_colnames.extend(parse_hyphen_range(colname, &available_columns)?);
        } else if is_numeric_index(colname) {
            if let Some(col_name) = parse_single_numeric_index(colname, &available_columns) {
                expanded_colnames.push(col_name);
            } else {
                return Err(QuiltError::usage(format!(
                    "Error: Invalid column index '{colname}'"
                )));
            }
        } else {
            expanded_colnames.push(colname.clone());
        }
    }

    for colname in &expanded_colnames {
        if !available_columns.iter().any(|name| name == colname) {
            return Err(QuiltError::schema(
                "select",
                Some(colname),
                format!("Error: Column '{colname}' not found in DataFrame for select operation"),
            ));
        }
    }

    let selected_cols: Vec<Expr> = expanded_colnames.iter().map(col).collect();
    if selected_cols.is_empty() {
        LogController::warn("No valid columns selected. Returning original DataFrame.");
        return Ok(df.clone());
    }
    Ok(df.clone().select(&selected_cols))
}

pub fn parse_hyphen_range(
    range_str: &str,
    available_columns: &[String],
) -> Result<Vec<String>, QuiltError> {
    let captures = match RE_COL_RANGE_HYPHEN.captures(range_str) {
        Some(captures) => captures,
        None => return Ok(vec![range_str.to_string()]),
    };
    let prefix1 = captures
        .name("p1")
        .ok_or_else(|| QuiltError::usage(format!("Error: Invalid range '{range_str}'")))?
        .as_str();
    let num1: usize = captures
        .name("n1")
        .ok_or_else(|| QuiltError::usage(format!("Error: Invalid range '{range_str}'")))?
        .as_str()
        .parse()
        .map_err(|_| {
            QuiltError::usage(format!("Error: Range number is too large in '{range_str}'"))
        })?;
    let (prefix2, num2) = if let Some(p2) = captures.name("p2") {
        (
            p2.as_str(),
            captures
                .name("n2")
                .ok_or_else(|| QuiltError::usage(format!("Error: Invalid range '{range_str}'")))?
                .as_str()
                .parse()
                .map_err(|_| {
                    QuiltError::usage(format!("Error: Range number is too large in '{range_str}'"))
                })?,
        )
    } else {
        (
            prefix1,
            captures
                .name("n3")
                .ok_or_else(|| QuiltError::usage(format!("Error: Invalid range '{range_str}'")))?
                .as_str()
                .parse()
                .map_err(|_| {
                    QuiltError::usage(format!("Error: Range number is too large in '{range_str}'"))
                })?,
        )
    };
    if prefix1 != prefix2 {
        return Err(QuiltError::usage(format!(
            "Error: Mismatched prefixes in range '{range_str}'. Both sides must have the same prefix."
        )));
    }
    if num1 > num2 {
        return Err(QuiltError::usage(format!(
            "Error: Invalid range '{range_str}'. Start number must be <= end number."
        )));
    }
    parse_colon_range(
        &format!("{prefix1}{num1}:{prefix1}{num2}"),
        available_columns,
    )
}

fn is_numeric_index(s: &str) -> bool {
    s.parse::<usize>().is_ok()
}

fn is_numeric_range(s: &str) -> bool {
    s.split_once(':')
        .map(|(start, end)| {
            start.trim().parse::<usize>().is_ok() && end.trim().parse::<usize>().is_ok()
        })
        .unwrap_or(false)
}

fn parse_single_numeric_index(index_str: &str, available_columns: &[String]) -> Option<String> {
    let index = index_str.parse::<usize>().ok()?;
    (1..=available_columns.len())
        .contains(&index)
        .then(|| available_columns[index - 1].clone())
}

fn parse_numeric_range(
    range_str: &str,
    available_columns: &[String],
) -> Result<Vec<String>, QuiltError> {
    let (start_str, end_str) = range_str
        .split_once(':')
        .ok_or_else(|| QuiltError::usage(format!("Error: Invalid numeric range '{range_str}'")))?;
    let start_idx = start_str.trim().parse::<usize>().map_err(|_| {
        QuiltError::usage(format!("Error: Invalid numeric range format: {range_str}"))
    })?;
    let end_idx = end_str.trim().parse::<usize>().map_err(|_| {
        QuiltError::usage(format!("Error: Invalid numeric range format: {range_str}"))
    })?;
    if start_idx < 1 || end_idx < 1 {
        return Err(QuiltError::usage(format!(
            "Error: Column indices are 1-based. Got index 0 in range '{range_str}'."
        )));
    }
    if start_idx > end_idx || end_idx > available_columns.len() {
        return Err(QuiltError::usage(format!(
            "Error: Invalid numeric range '{range_str}'. Indices are out of bounds or in invalid order."
        )));
    }
    Ok(available_columns[start_idx - 1..end_idx].to_vec())
}

pub fn parse_colon_range(
    range_str: &str,
    available_columns: &[String],
) -> Result<Vec<String>, QuiltError> {
    let (start_col, end_col) = range_str
        .split_once(':')
        .ok_or_else(|| QuiltError::usage(format!("Error: Invalid range '{range_str}'")))?;
    let start_col = start_col.trim();
    let end_col = end_col.trim();
    let start_idx = available_columns.iter().position(|c| c == start_col);
    let end_idx = available_columns.iter().position(|c| c == end_col);
    match (start_idx, end_idx) {
        (Some(start_idx), Some(end_idx)) if start_idx <= end_idx => {
            Ok(available_columns[start_idx..=end_idx].to_vec())
        }
        (Some(_), Some(_)) => Err(QuiltError::usage(format!(
            "Error: Invalid range: '{start_col}' comes after '{end_col}' in column order"
        ))),
        (None, _) => Err(QuiltError::schema(
            "select",
            Some(start_col),
            format!("Error: Column '{start_col}' not found in DataFrame for select operation"),
        )),
        (_, None) => Err(QuiltError::schema(
            "select",
            Some(end_col),
            format!("Error: Column '{end_col}' not found in DataFrame for select operation"),
        )),
    }
}

pub fn parse_quoted_colon_range(
    start_col: &str,
    end_col: &str,
    available_columns: &[String],
) -> Result<Vec<String>, QuiltError> {
    parse_colon_range(&format!("{start_col}:{end_col}"), available_columns)
}
