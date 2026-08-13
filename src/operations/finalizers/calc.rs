use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use polars::prelude::*;

pub const MODES: [&str; 6] = ["sum", "avg", "min", "max", "median", "std"];

fn is_numeric_dtype(dtype: &DataType) -> bool {
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
            | DataType::Float32
            | DataType::Float64
    )
}

fn scalar_text(value: &AnyValue<'_>) -> String {
    match value {
        AnyValue::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

pub fn calc(df: &LazyFrame, column: &str, mode: &str) -> Result<FinalizerResult, QuiltError> {
    if !MODES.contains(&mode) {
        return Err(QuiltError::usage(format!(
            "Unknown calc aggregation '{mode}'"
        )));
    }
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("calc", Some(column), error.to_string()))?;
    let source_dtype = schema.get(column).ok_or_else(|| {
        QuiltError::schema(
            "calc",
            Some(column),
            format!("Column '{column}' not found for calc"),
        )
    })?;
    if !is_numeric_dtype(source_dtype) {
        return Err(QuiltError::schema(
            "calc",
            Some(column),
            format!(
                "Calc column '{column}' must be numeric, found {}",
                source_dtype
            ),
        ));
    }
    let expression = match mode {
        "sum" => col(column).sum(),
        "avg" => col(column).mean(),
        "min" => col(column).min(),
        "max" => col(column).max(),
        "median" => col(column).quantile(lit(0.5), QuantileMethod::Linear),
        "std" => col(column)
            .cast(DataType::Float64)
            .var(1)
            .sqrt()
            .cast(DataType::Float64),
        _ => unreachable!(),
    };
    let aggregate = df
        .clone()
        .select([
            expression.alias("calc_value"),
            col(column).count().alias("calc_count"),
        ])
        .collect()
        .map_err(|error| {
            QuiltError::operation("calc", format!("failed to evaluate aggregate: {error}"))
        })?;
    let value = aggregate
        .column("calc_value")
        .map_err(|error| QuiltError::operation("calc", error.to_string()))?
        .get(0)
        .map_err(|error| QuiltError::operation("calc", error.to_string()))?;
    let count = aggregate
        .column("calc_count")
        .map_err(|error| QuiltError::operation("calc", error.to_string()))?
        .get(0)
        .map_err(|error| QuiltError::operation("calc", error.to_string()))?
        .try_extract::<u32>()
        .unwrap_or(0);
    if count == 0 || (mode == "std" && count < 2) {
        return Ok(FinalizerResult::Scalar("null".into()));
    }
    let output = if mode == "std" {
        value
            .try_extract::<f64>()
            .map(|number| number.to_string())
            .unwrap_or_else(|_| scalar_text(&value))
    } else {
        scalar_text(&value)
    };
    Ok(FinalizerResult::Scalar(output))
}
