//! Lazy grouped and global aggregations.
//!
//! Parsing a command-line or YAML representation belongs to the caller. This
//! module accepts typed specs so names (including names containing colons)
//! never need to be encoded in an ad-hoc mini-language.

use crate::controllers::nested::literal_col;
use crate::error::QuiltError;
use polars::prelude::*;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateMode {
    /// Count rows when `column` is `None`; count non-null values otherwise.
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Median,
    Std,
    /// Percentile in the inclusive range 0 through 100. Endpoints use exact
    /// min/max and retain the source dtype; interior linear quantiles use the
    /// statistical Float64 result supported by Polars.
    Percentile(u8),
    Nunique,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateSpec {
    pub mode: AggregateMode,
    pub column: Option<String>,
    pub output: String,
}

impl AggregateSpec {
    pub fn new(mode: AggregateMode, column: Option<String>, output: impl Into<String>) -> Self {
        Self {
            mode,
            column,
            output: output.into(),
        }
    }
}

pub(crate) fn integral_sum_dtype(dtype: &DataType) -> Option<DataType> {
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

pub(crate) fn numeric(dtype: &DataType) -> bool {
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

pub(crate) fn expression(spec: &AggregateSpec, schema: &Schema) -> Result<Expr, QuiltError> {
    let Some(column) = spec.column.as_deref() else {
        if spec.mode == AggregateMode::Count {
            return Ok(len().alias(&spec.output));
        }
        return Err(QuiltError::usage(format!(
            "Aggregate '{}' requires an input column",
            spec.output
        )));
    };
    let dtype = schema
        .get(column)
        .ok_or_else(|| QuiltError::schema("aggregate", Some(column), "column not found"))?;
    let col = literal_col(column);
    let expr = match spec.mode {
        AggregateMode::Count => col.count(),
        AggregateMode::Sum => {
            let (sum, output_dtype) = if let Some(output_dtype) = integral_sum_dtype(dtype) {
                // Sum exactly in Decimal128, then fail at sink time if the
                // conventional Int64/UInt64 result cannot represent the sum.
                (
                    col.clone()
                        .cast(DataType::Decimal(38, 0))
                        .sum()
                        .strict_cast(output_dtype.clone()),
                    output_dtype,
                )
            } else if numeric(dtype) {
                (col.clone().sum().cast(dtype.clone()), dtype.clone())
            } else {
                return Err(QuiltError::schema(
                    "aggregate",
                    Some(column),
                    "sum requires a numeric column",
                ));
            };
            // Polars' native sum returns zero for empty/all-null inputs. Quilt
            // keeps aggregate null semantics consistent with calc instead.
            when(col.count().eq(lit(0u32)))
                .then(lit(NULL).cast(output_dtype))
                .otherwise(sum)
        }
        AggregateMode::Avg
        | AggregateMode::Median
        | AggregateMode::Std
        | AggregateMode::Percentile(_) => {
            if !numeric(dtype) {
                return Err(QuiltError::schema(
                    "aggregate",
                    Some(column),
                    "aggregate requires a numeric column",
                ));
            }
            match spec.mode {
                AggregateMode::Avg => col.mean(),
                AggregateMode::Median => col.median(),
                AggregateMode::Std => col.cast(DataType::Float64).std(1),
                // Avoid the Float64 conversion used by linear quantiles at the
                // endpoints, preserving exact 64-bit integer values.
                AggregateMode::Percentile(0) => col.min(),
                AggregateMode::Percentile(100) => col.max(),
                AggregateMode::Percentile(percent) if percent <= 100 => {
                    col.quantile(lit(f64::from(percent) / 100.0), QuantileMethod::Linear)
                }
                AggregateMode::Percentile(percent) => {
                    return Err(QuiltError::usage(format!(
                        "Percentile {percent} is outside the range 0 through 100"
                    )));
                }
                _ => unreachable!(),
            }
        }
        AggregateMode::Min => col.min(),
        AggregateMode::Max => col.max(),
        // Match count's null-excluding behavior for distinct values.
        AggregateMode::Nunique => col.drop_nulls().n_unique(),
    };
    Ok(expr.alias(&spec.output))
}

/// Validate aggregate arguments without inspecting input data or schema.
pub(crate) fn validate_specs(specs: &[AggregateSpec], by: &[String]) -> Result<(), QuiltError> {
    if specs.is_empty() {
        return Err(QuiltError::usage("aggregate requires at least one metric"));
    }
    let mut names = HashSet::new();
    for key in by {
        if !names.insert(key.as_str()) {
            return Err(QuiltError::usage(format!(
                "Duplicate grouping column '{key}'"
            )));
        }
    }
    let mut output_names = HashSet::new();
    for spec in specs {
        if spec.mode != AggregateMode::Count && spec.column.is_none() {
            return Err(QuiltError::usage(format!(
                "Aggregate '{}' requires an input column",
                spec.output
            )));
        }
        if matches!(spec.mode, AggregateMode::Percentile(p) if p > 100) {
            return Err(QuiltError::usage("percentile must be p0 through p100"));
        }
        if spec.output.is_empty() {
            return Err(QuiltError::usage("aggregate output name cannot be empty"));
        }
        if names.contains(spec.output.as_str()) || !output_names.insert(spec.output.as_str()) {
            return Err(QuiltError::schema(
                "aggregate",
                Some(&spec.output),
                "aggregate output conflicts with a group key or another output",
            ));
        }
    }
    Ok(())
}

/// Add aggregate columns, optionally grouping in first-seen input order.
/// The result remains lazy and contains only grouping keys and outputs.
pub fn aggregate(
    df: &LazyFrame,
    specs: &[AggregateSpec],
    by: &[String],
) -> Result<LazyFrame, QuiltError> {
    validate_specs(specs, by)?;
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("aggregate", None::<String>, error.to_string()))?;
    for key in by {
        if schema.get(key).is_none() {
            return Err(QuiltError::schema(
                "aggregate",
                Some(key),
                "group column not found",
            ));
        }
    }

    let expressions = specs
        .iter()
        .map(|spec| expression(spec, &schema))
        .collect::<Result<Vec<_>, _>>()?;
    if by.is_empty() {
        Ok(df.clone().select(expressions))
    } else {
        let keys: Vec<Expr> = by.iter().map(|name| literal_col(name)).collect();
        Ok(df.clone().group_by_stable(keys).agg(expressions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_specs_keep_first_seen_group_order() {
        let frame = df!("g" => ["b", "a", "b"], "v" => [2i64, 10, 3])
            .unwrap()
            .lazy();
        let specs = [AggregateSpec::new(
            AggregateMode::Sum,
            Some("v".into()),
            "total",
        )];
        let out = aggregate(&frame, &specs, &["g".to_owned()])
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(out.column("g").unwrap().str().unwrap().get(0), Some("b"));
        assert_eq!(out.column("total").unwrap().i64().unwrap().get(0), Some(5));
    }

    #[test]
    fn grouped_null_sum_and_row_count_share_one_aggregation() {
        let frame = df!(
            "g" => ["a", "a", "b"],
            "v" => [None::<i64>, None, Some(7)],
            "f" => [None::<f64>, None, Some(2.5)]
        )
        .unwrap()
        .lazy();
        let specs = [
            AggregateSpec::new(AggregateMode::Sum, Some("v".into()), "total"),
            AggregateSpec::new(AggregateMode::Sum, Some("f".into()), "float_total"),
            AggregateSpec::new(AggregateMode::Count, None, "rows"),
        ];
        let out = aggregate(&frame, &specs, &["g".to_owned()])
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(out.column("total").unwrap().get(0).unwrap(), AnyValue::Null);
        assert_eq!(
            out.column("float_total").unwrap().get(0).unwrap(),
            AnyValue::Null
        );
        assert_eq!(
            out.column("float_total").unwrap().f64().unwrap().get(1),
            Some(2.5)
        );
        assert_eq!(out.column("rows").unwrap().u32().unwrap().get(0), Some(2));
        assert_eq!(out.column("rows").unwrap().u32().unwrap().get(1), Some(1));
    }

    #[test]
    fn integral_sum_checks_public_range() {
        let frame = df!("v" => [i64::MAX, 1i64]).unwrap().lazy();
        let specs = [AggregateSpec::new(
            AggregateMode::Sum,
            Some("v".into()),
            "total",
        )];
        assert!(aggregate(&frame, &specs, &[]).unwrap().collect().is_err());

        let frame = df!("v" => [u64::MAX, 1u64]).unwrap().lazy();
        let specs = [AggregateSpec::new(
            AggregateMode::Sum,
            Some("v".into()),
            "total",
        )];
        assert!(aggregate(&frame, &specs, &[]).unwrap().collect().is_err());
    }

    #[test]
    fn integer_percentile_endpoints_and_counts_preserve_exact_values() {
        let frame = df!(
            "v" => [Some(9_007_199_254_740_993u64), Some(u64::MAX), None]
        )
        .unwrap()
        .lazy();
        let specs = [
            AggregateSpec::new(AggregateMode::Percentile(0), Some("v".into()), "p0"),
            AggregateSpec::new(AggregateMode::Percentile(100), Some("v".into()), "p100"),
            AggregateSpec::new(AggregateMode::Nunique, Some("v".into()), "distinct"),
            AggregateSpec::new(AggregateMode::Count, Some("v".into()), "nonnull"),
            AggregateSpec::new(AggregateMode::Count, None, "rows"),
        ];
        let out = aggregate(&frame, &specs, &[]).unwrap().collect().unwrap();
        assert_eq!(
            out.column("p0").unwrap().u64().unwrap().get(0),
            Some(9_007_199_254_740_993)
        );
        assert_eq!(
            out.column("p100").unwrap().u64().unwrap().get(0),
            Some(u64::MAX)
        );
        assert_eq!(
            out.column("distinct").unwrap().u32().unwrap().get(0),
            Some(2)
        );
        assert_eq!(
            out.column("nonnull").unwrap().u32().unwrap().get(0),
            Some(2)
        );
        assert_eq!(out.column("rows").unwrap().u32().unwrap().get(0), Some(3));
    }
}
