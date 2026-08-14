use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;

pub fn renamecol(
    df: &LazyFrame,
    old_colname: &str,
    new_colname: &str,
) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("renamecol", Some(old_colname), e.to_string()))?;

    if !schema.iter_names().any(|s| s == old_colname) {
        return Err(QuiltError::schema(
            "renamecol",
            Some(old_colname),
            "column not found",
        ));
    }

    LogController::debug("Renaming column");

    // Get all column names and replace the old one with the new one
    let all_columns: Vec<Expr> = schema
        .iter_names()
        .map(|name| {
            if name.as_str() == old_colname {
                col(old_colname).alias(new_colname)
            } else {
                col(name.as_str())
            }
        })
        .collect();

    Ok(df.clone().select(all_columns))
}
