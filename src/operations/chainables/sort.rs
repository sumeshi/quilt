use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;

pub fn sort(df: &LazyFrame, colnames: &[String], desc: bool) -> Result<LazyFrame, QuiltError> {
    // Sorting is intentionally retained as a logical barrier: an ordered
    // result requires the eventual sink to inspect all input rows, but the
    // application still does not evaluate the frame here.
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("sort", None::<String>, e.to_string()))?;

    for colname in colnames {
        if !schema.iter_names().any(|s| s == colname) {
            return Err(QuiltError::schema(
                "sort",
                Some(colname),
                "column not found",
            ));
        }
    }

    LogController::debug("Sorting columns");

    let sort_exprs: Vec<Expr> = colnames.iter().map(col).collect();
    let sort_options = SortMultipleOptions::default().with_order_descending(desc);

    Ok(df.clone().sort_by_exprs(sort_exprs, sort_options))
}
