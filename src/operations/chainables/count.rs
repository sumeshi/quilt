use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::{col, len, Expr, LazyFrame, SortMultipleOptions};

pub fn count(df: &LazyFrame, columns: &[String]) -> Result<LazyFrame, QuiltError> {
    // Grouped counting (and its count sort) is intentionally a global
    // operation. It remains lazy in the plan, while evaluation must observe
    // the complete input to produce groups and counts.
    LogController::debug("Applying count");

    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("count", None::<String>, e.to_string()))?;

    let group_colnames: Vec<String> = if columns.is_empty() {
        schema.iter_names().map(|s| s.to_string()).collect()
    } else {
        for column in columns {
            if !schema.iter_names().any(|name| name == column) {
                return Err(QuiltError::schema(
                    "count",
                    Some(column),
                    "column not found",
                ));
            }
        }
        columns.to_vec()
    };
    if group_colnames.iter().any(|column| column == "count") {
        return Err(QuiltError::schema(
            "count",
            Some("count"),
            "grouping column conflicts with output column 'count'",
        ));
    }

    Ok(df
        .clone()
        .group_by(group_colnames.iter().map(col).collect::<Vec<Expr>>())
        .agg([len().alias("count")])
        .sort(
            ["count"],
            SortMultipleOptions::default().with_order_descending(true),
        ))
}
