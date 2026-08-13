use crate::error::QuiltError;
use polars::prelude::*;

// Datetime deltas are normalized to Duration[μs], matching the shared
// datetime parser's internal precision regardless of source datetime unit.

fn integral_dtype(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}

pub fn delta(df: &LazyFrame, column: &str, output: Option<&str>) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("delta", None::<String>, error.to_string()))?;
    let source = schema
        .get(column)
        .ok_or_else(|| QuiltError::schema("delta", Some(column), "column not found"))?;
    let output = output
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{column}_delta"));
    if schema.get(&output).is_some() {
        return Err(QuiltError::schema(
            "delta",
            Some(output),
            "output column already exists",
        ));
    }
    if integral_dtype(source) {
        // Decimal128 performs lossless subtraction across the complete
        // integer input range; the final strict cast makes the public result
        // Int64 and reports out-of-range values at sink time.
        let decimal = DataType::Decimal(Some(38), Some(0));
        let difference =
            col(column).cast(decimal.clone()) - col(column).cast(decimal).shift(lit(1i64));
        Ok(df
            .clone()
            .with_columns([difference.strict_cast(DataType::Int64).alias(output)]))
    } else if matches!(source, DataType::Float32 | DataType::Float64) {
        // Floating inputs retain their source precision; Float32 deltas are
        // Float32 and Float64 deltas are Float64.
        Ok(df
            .clone()
            .with_columns([(col(column) - col(column).shift(lit(1i64))).alias(output)]))
    } else if let DataType::Datetime(unit, timezone) = source {
        let raw_delta =
            col(column).cast(DataType::Int64) - col(column).cast(DataType::Int64).shift(lit(1i64));
        let micros = match unit {
            TimeUnit::Nanoseconds => raw_delta / lit(1_000i64),
            TimeUnit::Microseconds => raw_delta,
            TimeUnit::Milliseconds => raw_delta * lit(1_000i64),
        };
        let _ = timezone;
        Ok(df.clone().with_columns([micros
            .cast(DataType::Duration(TimeUnit::Microseconds))
            .alias(output)]))
    } else {
        Err(QuiltError::schema(
            "delta",
            Some(column),
            format!("column must be numeric or datetime, found {}", source),
        ))
    }
}
