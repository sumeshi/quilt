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

    LogController::debug("Applying isin");

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

    let comparisons = values
        .iter()
        .map(|value| {
            match col_dtype {
                DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => value
                    .parse::<i64>()
                    .map_err(|_| ())
                    .map(|parsed| col(colname).cast(DataType::Int64).eq(lit(parsed))),
                DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => value
                    .parse::<u64>()
                    .map_err(|_| ())
                    .map(|parsed| col(colname).cast(DataType::UInt64).eq(lit(parsed))),
                DataType::Float32 | DataType::Float64 => value
                    .parse::<f64>()
                    .map_err(|_| ())
                    .map(|parsed| col(colname).cast(DataType::Float64).eq(lit(parsed))),
                DataType::Boolean => value
                    .parse::<bool>()
                    .map_err(|_| ())
                    .map(|parsed| col(colname).eq(lit(parsed))),
                DataType::String => Ok(col(colname).eq(lit(value.as_str()))),
                _ => Ok(col(colname).cast(DataType::String).eq(lit(value.as_str()))),
            }
            .map_err(|_| {
                QuiltError::usage(format!(
                "Error: Value '{value}' is invalid for column '{colname}' with type {col_dtype}"
            ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let filter_expr = comparisons
        .into_iter()
        .reduce(|acc, expr| acc.or(expr))
        .unwrap_or_else(|| lit(false));

    Ok(df.clone().filter(filter_expr))
}
