use crate::controllers::nested::literal_col;
use crate::error::QuiltError;
use crate::operations::chainables::aggregate::{expression, AggregateMode, AggregateSpec};
use crate::operations::finalizers::FinalizerResult;
use polars::prelude::*;

pub const MODES: [&str; 6] = ["sum", "avg", "min", "max", "median", "std"];

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
    if !crate::operations::chainables::aggregate::numeric(source_dtype) {
        return Err(QuiltError::schema(
            "calc",
            Some(column),
            format!(
                "Calc column '{column}' must be numeric, found {}",
                source_dtype
            ),
        ));
    }
    let mode_name = mode;
    let aggregate_mode = match mode_name {
        "sum" => AggregateMode::Sum,
        "avg" => AggregateMode::Avg,
        "min" => AggregateMode::Min,
        "max" => AggregateMode::Max,
        "median" => AggregateMode::Median,
        "std" => AggregateMode::Std,
        _ => unreachable!(),
    };
    let spec = AggregateSpec::new(aggregate_mode, Some(column.to_owned()), "calc_value");
    let expression = expression(&spec, &schema)
        .map_err(|error| QuiltError::schema("calc", Some(column), error.to_string()))?;
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
    if count == 0 || (mode_name == "std" && count < 2) {
        return Ok(FinalizerResult::Scalar("null".into()));
    }
    let output = if mode_name == "std" {
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

    #[test]
    fn calc_uses_the_same_safe_sum_expression_as_aggregate() {
        use crate::operations::chainables::aggregate::{aggregate, AggregateMode, AggregateSpec};

        let frame = df!("n" => [1_000_000_000_000_001i64, 7, 9]).unwrap().lazy();
        let calc_value = calc(&frame, "n", "sum").unwrap();
        let aggregate_frame = aggregate(
            &frame,
            &[AggregateSpec::new(
                AggregateMode::Sum,
                Some("n".into()),
                "total",
            )],
            &[],
        )
        .unwrap()
        .collect()
        .unwrap();
        let aggregate_value = aggregate_frame.column("total").unwrap().get(0).unwrap();
        assert_eq!(
            calc_value,
            FinalizerResult::Scalar(aggregate_value.to_string())
        );
    }
}
