use crate::controllers::nested::literal_col;
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

/// Add a delta column, resetting the previous-row comparison at each group.
/// Rows retain their input order within each group; the result itself remains
/// in its original order.
pub fn delta_with_groups(
    df: &LazyFrame,
    column: &str,
    output: Option<&str>,
    by: &[String],
) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("delta", None::<String>, error.to_string()))?;
    let source = schema
        .get(column)
        .ok_or_else(|| QuiltError::schema("delta", Some(column), "column not found"))?;
    for group in by {
        if schema.get(group).is_none() {
            return Err(QuiltError::schema(
                "delta",
                Some(group),
                "group column not found",
            ));
        }
    }
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
    let groups: Vec<Expr> = by.iter().map(|name| literal_col(name)).collect();
    let over = |expr: Expr| -> Result<Expr, QuiltError> {
        if groups.is_empty() {
            Ok(expr)
        } else {
            expr.over(groups.clone())
                .map_err(|error| QuiltError::operation("delta", error.to_string()))
        }
    };
    if integral_dtype(source) {
        // Decimal128 performs lossless subtraction across the complete
        // integer input range; the final strict cast makes the public result
        // Int64 and reports out-of-range values at sink time.
        let decimal = DataType::Decimal(38, 0);
        let difference = literal_col(column).cast(decimal.clone())
            - literal_col(column).cast(decimal).shift(lit(1i64));
        Ok(df
            .clone()
            .with_columns([over(difference)?.strict_cast(DataType::Int64).alias(output)]))
    } else if matches!(source, DataType::Float32 | DataType::Float64) {
        // Floating inputs retain their source precision; Float32 deltas are
        // Float32 and Float64 deltas are Float64.
        Ok(df
            .clone()
            .with_columns([
                over(literal_col(column) - literal_col(column).shift(lit(1i64)))?.alias(output),
            ]))
    } else if let DataType::Datetime(unit, timezone) = source {
        // Widen before subtraction and scaling, since both can exceed i64 even
        // when each source timestamp is representable.
        let decimal = DataType::Decimal(38, 0);
        let ticks = literal_col(column)
            .cast(DataType::Int64)
            .cast(decimal.clone());
        let raw_delta = ticks.clone() - ticks.shift(lit(1i64));
        let micros = match unit {
            TimeUnit::Nanoseconds => over(raw_delta / lit(1_000i64))?,
            TimeUnit::Microseconds => over(raw_delta)?,
            TimeUnit::Milliseconds => over(raw_delta * lit(1_000i64))?,
        };
        let _ = timezone;
        Ok(df.clone().with_columns([micros
            .strict_cast(DataType::Int64)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_delta_uses_each_groups_previous_input_row() {
        let frame = df!(
            "g" => ["a", "a", "b", "a"],
            "v" => [2i64, 5, 8, 10]
        )
        .unwrap()
        .lazy();
        let out = delta_with_groups(&frame, "v", None, &["g".to_owned()])
            .unwrap()
            .collect()
            .unwrap();
        let result = out.column("v_delta").unwrap().i64().unwrap();
        assert_eq!(result.get(0), None);
        assert_eq!(result.get(1), Some(3));
        assert_eq!(result.get(2), None);
        assert_eq!(result.get(3), Some(5));
    }

    #[test]
    fn datetime_millisecond_delta_is_scaled_to_microseconds() {
        let frame = df!("t" => [0i64, 2]).unwrap().lazy().with_columns([
            literal_col("t").cast(DataType::Datetime(TimeUnit::Milliseconds, None))
        ]);
        let out = delta_with_groups(&frame, "t", None, &[])
            .unwrap()
            .collect()
            .unwrap();
        let value = out
            .column("t_delta")
            .unwrap()
            .duration()
            .unwrap()
            .physical()
            .get(1);
        assert_eq!(value, Some(2_000));
    }
}
