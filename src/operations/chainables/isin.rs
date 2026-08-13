use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;

pub fn isin(df: &LazyFrame, colname: &str, values: &[String]) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("isin", Some(colname), e.to_string()))?;

    if !schema.iter_names().any(|s| s == colname) {
        return Err(QuiltError::schema(
            "isin",
            Some(colname),
            "column not found",
        ));
    }

    LogController::debug(&format!(
        "Applying isin: column={colname} values={values:?}"
    ));

    if values.is_empty() {
        LogController::debug("Empty values list for isin, returning empty result");
        return Ok(df.clone().filter(lit(false)));
    }

    // Get the column data type
    let col_dtype = schema.get(colname).ok_or_else(|| {
        QuiltError::schema(
            "isin",
            Some(colname),
            "column disappeared during schema inspection",
        )
    })?;

    // Build filter expression efficiently using fold instead of manual iteration
    let filter_expr = if matches!(
        col_dtype,
        DataType::Int64 | DataType::Int32 | DataType::Float64 | DataType::Float32
    ) {
        // For numeric columns, convert to string and compare
        values
            .iter()
            .map(|val_str| {
                col(colname)
                    .cast(DataType::String)
                    .eq(lit(val_str.as_str()))
            })
            .reduce(|acc, expr| acc.or(expr))
            .unwrap_or_else(|| lit(false))
    } else {
        // For string and other types, use direct comparison
        values
            .iter()
            .map(|val_str| col(colname).eq(lit(val_str.as_str())))
            .reduce(|acc, expr| acc.or(expr))
            .unwrap_or_else(|| lit(false))
    };

    Ok(df.clone().filter(filter_expr))
}
