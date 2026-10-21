use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;

pub fn grep(
    df: &LazyFrame,
    pattern: &str,
    ignorecase: bool,
    is_inverted: bool,
    columns: Option<&[String]>,
) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("grep", None::<String>, e.to_string()))?;

    let all_column_names: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();
    let target_columns: Vec<String> = if let Some(columns) = columns {
        for column in columns {
            if !schema.iter_names().any(|name| name == column) {
                return Err(QuiltError::schema("grep", Some(column), "column not found"));
            }
        }
        columns.to_vec()
    } else {
        all_column_names
    };

    LogController::debug("Applying grep");

    let final_pattern = if ignorecase {
        format!("(?i){pattern}")
    } else {
        pattern.to_string()
    };

    // Polars cannot combine distinct regex selectors in one predicate. Resolve
    // selector-shaped names in separate lazy expressions before combining them.
    let pattern_lit = lit(final_pattern);
    let special_columns: Vec<&String> = target_columns
        .iter()
        .filter(|name| name.as_str() == "*" || (name.starts_with('^') && name.ends_with('$')))
        .collect();
    let mut reserved: std::collections::HashSet<String> =
        schema.iter_names().map(|name| name.to_string()).collect();
    let mut helper_index = 0usize;
    let helper_names: Vec<String> = (0..special_columns.len())
        .map(|_| loop {
            let name = format!("__qlt_grep_{helper_index}");
            helper_index += 1;
            if reserved.insert(name.clone()) {
                break name;
            }
        })
        .collect();
    let search_columns: Vec<Expr> = special_columns
        .iter()
        .zip(&helper_names)
        .map(|(col_name, helper)| {
            crate::controllers::nested::search_expr(col_name, schema.get(col_name).unwrap())
                .str()
                .contains(pattern_lit.clone(), false) // literal=false for regex
                .fill_null(lit(false))
                .alias(helper)
        })
        .collect();
    let ordinary_exprs = target_columns
        .iter()
        .filter(|name| !(name.as_str() == "*" || (name.starts_with('^') && name.ends_with('$'))))
        .map(|col_name| {
            crate::controllers::nested::search_expr(col_name, schema.get(col_name).unwrap())
                .str()
                .contains(pattern_lit.clone(), false)
                .fill_null(lit(false))
        });
    let filter_expr = ordinary_exprs
        .chain(helper_names.iter().map(col))
        .reduce(|acc, expr| acc.or(expr))
        .unwrap_or_else(|| lit(false));

    let searched = df.clone().with_columns(search_columns);
    if is_inverted {
        Ok(searched.filter(filter_expr.not()).drop(cols(helper_names)))
    } else {
        Ok(searched.filter(filter_expr).drop(cols(helper_names)))
    }
}
