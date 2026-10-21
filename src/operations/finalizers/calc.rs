use crate::controllers::nested::literal_col;
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

fn integral_sum_dtype(dtype: &DataType) -> Option<DataType> {
    match dtype {
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64 => {
            Some(DataType::Int64)
        }
        DataType::UInt8 | DataType::UInt16 | DataType::UInt32 | DataType::UInt64 => {
            Some(DataType::UInt64)
        }
        _ => None,
    }
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
        "sum" => {
            if let Some(public_dtype) = integral_sum_dtype(source_dtype) {
                // Decimal128 keeps the complete input sum exact. The strict cast
                // reports totals outside Polars' conventional Int64/UInt64 sum
                // type instead of silently wrapping them.
                literal_col(column)
                    .cast(DataType::Decimal(38, 0))
                    .sum()
                    .strict_cast(public_dtype)
            } else {
                literal_col(column).sum()
            }
        }
        "avg" => literal_col(column).mean(),
        "min" => literal_col(column).min(),
        "max" => literal_col(column).max(),
        "median" => literal_col(column).quantile(lit(0.5), QuantileMethod::Linear),
        "std" => literal_col(column)
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
            literal_col(column).count().alias("calc_count"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_sum_does_not_wrap() {
        let signed = df!("n" => [i64::MAX, 1]).unwrap().lazy();
        assert!(calc(&signed, "n", "sum").is_err());
        let unsigned = df!("n" => [u64::MAX, 1]).unwrap().lazy();
        assert!(calc(&unsigned, "n", "sum").is_err());
    }

    #[test]
    fn empty_and_all_null_sum_remain_null() {
        let all_null = df!("n" => [None::<i64>]).unwrap().lazy();
        assert_eq!(
            calc(&all_null, "n", "sum").unwrap(),
            FinalizerResult::Scalar("null".into())
        );
    }
}
