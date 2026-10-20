use crate::error::QuiltError;
use polars::prelude::*;

/// Preserve nested values as JSON cells at the CSV boundary. This projection
/// stays lazy so filtering and column selection still precede serialization.
pub(super) fn csv_frame(df: &LazyFrame, operation: &str) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::finalizer(operation, error.to_string()))?;
    let expressions = schema.iter().map(|(name, dtype)| {
        let column = col(name.clone());
        if matches!(dtype, DataType::Struct(_) | DataType::List(_)) {
            crate::controllers::nested::json_expr(column).alias(name.clone())
        } else {
            column
        }
    });
    Ok(df.clone().select(expressions.collect::<Vec<_>>()))
}
