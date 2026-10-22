use crate::error::QuiltError;
use polars::prelude::*;

/// Preserve nested values as JSON cells and durations as exact integer counts
/// at the CSV boundary. Duration values remain in their declared source unit
/// (ns, us, or ms), including values nested in lists and structs; converting
/// to a more familiar unit could lose precision.
/// This projection stays lazy so filtering and column selection still precede
/// serialization.
pub(super) fn csv_frame(df: &LazyFrame, operation: &str) -> Result<LazyFrame, QuiltError> {
    output_frame(df, operation, true)
}

/// Prepare a bounded table preview, which keeps temporal values typed so the
/// table formatter can show their human-readable representation.
pub(super) fn table_frame(df: &LazyFrame, operation: &str) -> Result<LazyFrame, QuiltError> {
    output_frame(df, operation, false)
}

fn output_frame(
    df: &LazyFrame,
    operation: &str,
    durations_as_counts: bool,
) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::finalizer(operation, error.to_string()))?;
    let expressions = schema.iter().map(|(name, dtype)| {
        let column = crate::controllers::nested::literal_col(name.as_str());
        if matches!(dtype, DataType::Struct(_) | DataType::List(_)) {
            let column = if durations_as_counts {
                let normalized = duration_counts_dtype(dtype);
                if normalized != *dtype {
                    column.cast(normalized)
                } else {
                    column
                }
            } else {
                column
            };
            crate::controllers::nested::json_expr(column, dtype).alias(name.clone())
        } else if durations_as_counts && matches!(dtype, DataType::Duration(_)) {
            column.cast(DataType::Int64).alias(name.clone())
        } else {
            column
        }
    });
    Ok(df.clone().select(expressions.collect::<Vec<_>>()))
}

/// Replace Duration leaves recursively while preserving nullability, list
/// shape, field names, and all non-duration dtypes (including Datetime).
pub(super) fn duration_counts_dtype(dtype: &DataType) -> DataType {
    match dtype {
        DataType::Duration(_) => DataType::Int64,
        DataType::List(inner) => DataType::List(Box::new(duration_counts_dtype(inner))),
        DataType::Struct(fields) => DataType::Struct(
            fields
                .iter()
                .map(|field| Field::new(field.name().clone(), duration_counts_dtype(field.dtype())))
                .collect(),
        ),
        _ => dtype.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_duration_projection_preserves_exact_source_unit_counts() {
        let duration = Series::new("elapsed".into(), &[1i64, 1_000_000_001])
            .cast(&DataType::Duration(TimeUnit::Nanoseconds))
            .unwrap();
        let frame = DataFrame::new_infer_height(vec![duration.into()])
            .unwrap()
            .lazy();
        let output = csv_frame(&frame, "test CSV").unwrap().collect().unwrap();
        assert_eq!(output.column("elapsed").unwrap().dtype(), &DataType::Int64);
        assert_eq!(
            output.column("elapsed").unwrap().i64().unwrap().get(1),
            Some(1_000_000_001)
        );
    }

    #[test]
    fn csv_nested_duration_projection_uses_integer_source_counts() {
        let nested_duration = Series::new("elapsed".into(), &[1i64, 2])
            .cast(&DataType::Duration(TimeUnit::Nanoseconds))
            .unwrap();
        let nested = StructChunked::from_series("nested".into(), 2, [&nested_duration].into_iter())
            .unwrap()
            .into_series();
        let list = Series::new(
            "items".into(),
            &[Some(Series::new(PlSmallStr::EMPTY, &[3i64, 4])), None],
        )
        .cast(&DataType::List(Box::new(DataType::Duration(
            TimeUnit::Nanoseconds,
        ))))
        .unwrap();
        let frame = DataFrame::new_infer_height(vec![nested.into(), list.into()])
            .unwrap()
            .lazy();
        let output = csv_frame(&frame, "test CSV").unwrap().collect().unwrap();
        assert_eq!(
            output.column("nested").unwrap().str().unwrap().get(0),
            Some(r#"{"elapsed":1}"#)
        );
        assert_eq!(
            output.column("items").unwrap().str().unwrap().get(0),
            Some("[3,4]")
        );
        assert_eq!(output.column("items").unwrap().str().unwrap().get(1), None);
    }

    #[test]
    fn csv_nested_json_rejects_non_finite_float_leaves() {
        let directory = tempfile::tempdir().unwrap();
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let nested = StructChunked::from_series(
                "nested".into(),
                1,
                [&Series::new("value".into(), &[value])].into_iter(),
            )
            .unwrap()
            .into_series();
            let items = Series::new(
                "items".into(),
                &[Some(Series::new(PlSmallStr::EMPTY, &[value]))],
            );
            for nested_column in [nested, items] {
                let frame = DataFrame::new_infer_height(vec![nested_column.into()])
                    .unwrap()
                    .lazy();
                let path = directory.path().join("output.csv");
                let error = super::super::dump::dump(&frame, path.to_str(), ',').unwrap_err();
                assert!(error.to_string().contains("non-finite float"));
                assert!(!path.exists());
                assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
            }
        }
    }
}
