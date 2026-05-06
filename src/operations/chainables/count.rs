use crate::controllers::log::LogController;
use polars::prelude::{col, len, Expr, LazyFrame, SortMultipleOptions};

pub fn count(df: &LazyFrame, columns: &[String]) -> LazyFrame {
    LogController::debug("Applying count");

    let schema = match df.clone().collect_schema() {
        Ok(s) => s,
        Err(e) => {
            LogController::error(&format!(
                "Failed to get schema for count: {e}. Returning original LazyFrame."
            ));
            return df.clone();
        }
    };

    let group_colnames: Vec<String> = if columns.is_empty() {
        schema.iter_names().map(|s| s.to_string()).collect()
    } else {
        for column in columns {
            if !schema.iter_names().any(|name| name == column) {
                eprintln!("Error: Column '{column}' not found in DataFrame for count operation");
                std::process::exit(1);
            }
        }
        columns.to_vec()
    };

    df.clone()
        .group_by(group_colnames.iter().map(col).collect::<Vec<Expr>>())
        .agg([len().alias("count")])
        .sort(
            ["count"],
            SortMultipleOptions::default().with_order_descending(true),
        )
}
