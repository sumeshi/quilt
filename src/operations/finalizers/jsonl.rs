use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::finalizers::{
    atomic_path, validate_destination, FinalizerResult, OutputArtifact,
};
use polars::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Evaluate a lazy JSON Lines sink into an execution-owned artifact.
pub fn show_jsonl(
    df: &LazyFrame,
    resources: &ExecutionResources,
) -> Result<FinalizerResult, QuiltError> {
    let output_resources = resources.fork(Vec::new());
    let reservation = output_resources
        .reserve_temp_file("qlt-show", "jsonl")
        .map_err(|error| {
            QuiltError::io("create show artifact", None::<String>, error.to_string())
        })?;
    let path = reservation.path().to_path_buf();
    output_resources
        .retain_temp_file(reservation)
        .map_err(|error| {
            QuiltError::io(
                "retain show artifact",
                Some(path.display().to_string()),
                error.to_string(),
            )
        })?;
    sink_jsonl(df, &path, "show JSONL")?;
    Ok(FinalizerResult::Artifact(Arc::new(OutputArtifact::new(
        path,
        output_resources,
    ))))
}

/// Write JSON Lines to a new destination using the shared no-clobber atomic
/// publication path. Durations at every nesting level are exact integer
/// counts in their declared source unit. Datetime values remain typed and use
/// Polars' documented display strings.
pub fn dump_jsonl(df: &LazyFrame, path: &str) -> Result<FinalizerResult, QuiltError> {
    let path = PathBuf::from(path);
    if path == Path::new("-") {
        return Err(QuiltError::usage(
            "Error: The 'dump' command requires a file path. To print to stdout, use the 'show' command instead.",
        ));
    }
    validate_destination(&path, "write dump")?;
    atomic_path(&path, "write dump", |temp| {
        sink_jsonl(df, temp, "write dump")
    })?;
    Ok(FinalizerResult::File(path))
}

fn sink_jsonl(df: &LazyFrame, path: &Path, operation: &str) -> Result<(), QuiltError> {
    let frame = checked_jsonl_frame(df, operation)?;
    frame
        .sink(
            SinkDestination::File {
                target: SinkTarget::Path(PlRefPath::try_from_path(path)?),
            },
            FileWriteFormat::NDJson(NDJsonWriterOptions::default()),
            UnifiedSinkArgs::default(),
        )
        .and_then(|frame| frame.collect())
        .map(|_| ())
        .map_err(|error| QuiltError::finalizer(operation, error.to_string()))
}

/// Add lazy per-column validation to float-containing columns. Polars' JSON
/// serializers map NaN and infinities to null, which hides invalid output, so
/// reject them while the streaming sink evaluates each column batch. Columns
/// without floating-point leaves are left untouched, preserving empty structs.
fn checked_jsonl_frame(df: &LazyFrame, operation: &str) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::finalizer(operation, error.to_string()))?;
    let expressions = schema
        .iter()
        .map(|(name, dtype)| {
            let column = crate::controllers::nested::literal_col(name.as_str());
            let column = if dtype_contains_float(dtype) {
                let column_name = name.to_string();
                column.map(
                    move |column| {
                        if series_has_non_finite(column.as_materialized_series())? {
                            return Err(PolarsError::ComputeError(
                                format!(
                                    "non-finite float in column '{column_name}' cannot be represented in JSON"
                                )
                                .into(),
                            ));
                        }
                        Ok(column)
                    },
                    |_, field| Ok(field.clone()),
                )
            } else {
                column
            };
            let normalized = super::csv::duration_counts_dtype(dtype);
            if normalized != *dtype {
                column.cast(normalized)
            } else {
                column
            }
        })
        .collect::<Vec<_>>();
    Ok(df.clone().select(expressions))
}

fn dtype_contains_float(dtype: &DataType) -> bool {
    match dtype {
        DataType::Float16 | DataType::Float32 | DataType::Float64 => true,
        DataType::List(inner) => dtype_contains_float(inner),
        DataType::Struct(fields) => fields
            .iter()
            .any(|field| dtype_contains_float(field.dtype())),
        _ => false,
    }
}

fn series_has_non_finite(series: &Series) -> PolarsResult<bool> {
    for row in 0..series.len() {
        if value_has_non_finite(series.get(row)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn value_has_non_finite(value: AnyValue<'_>) -> bool {
    match value {
        AnyValue::Float16(value) => !value.is_finite(),
        AnyValue::Float32(value) => !value.is_finite(),
        AnyValue::Float64(value) => !value.is_finite(),
        AnyValue::List(values) => (0..values.len())
            .any(|index| values.get(index).map(value_has_non_finite).unwrap_or(false)),
        AnyValue::Struct(_, _, _) => value._iter_struct_av().any(value_has_non_finite),
        AnyValue::StructOwned(values) => values.0.iter().cloned().any(value_has_non_finite),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn dump_jsonl_preserves_unsigned_integer_and_null_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.jsonl");
        let frame = df!(
            "large" => &[u64::MAX, 1],
            "signed" => &[-2i64, 3],
            "optional" => &[None::<i64>, Some(4)]
        )
        .unwrap()
        .lazy();
        dump_jsonl(&frame, path.to_str().unwrap()).unwrap();
        let lines = fs::read_to_string(&path).unwrap();
        let rows = lines
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["large"], serde_json::Value::from(u64::MAX));
        assert_eq!(rows[0]["signed"], -2);
        assert!(rows[0]["optional"].is_null());
    }

    #[test]
    fn dump_jsonl_rejects_non_finite_float_without_publishing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.jsonl");
        let frame = df!("value" => &[1.0f64, f64::INFINITY]).unwrap().lazy();
        assert!(dump_jsonl(&frame, path.to_str().unwrap()).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn dump_jsonl_preserves_empty_struct_and_nested_duration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.jsonl");
        let empty = Series::from_any_values_and_dtype(
            "empty".into(),
            &[
                AnyValue::StructOwned(Box::new((vec![], vec![]))),
                AnyValue::StructOwned(Box::new((vec![], vec![]))),
            ],
            &DataType::Struct(vec![]),
            false,
        )
        .unwrap();
        let nested_duration = Series::new("elapsed".into(), &[1i64, 2])
            .cast(&DataType::Duration(TimeUnit::Microseconds))
            .unwrap();
        let nested = StructChunked::from_series("nested".into(), 2, [&nested_duration].into_iter())
            .unwrap()
            .into_series();
        let nested_list = Series::new(
            "items".into(),
            &[Some(Series::new(PlSmallStr::EMPTY, &[3i64, 4])), None],
        )
        .cast(&DataType::List(Box::new(DataType::Duration(
            TimeUnit::Microseconds,
        ))))
        .unwrap();
        let frame = DataFrame::new_infer_height(vec![
            empty.into(),
            nested.into(),
            nested_list.into(),
            Series::new("large".into(), &[u64::MAX, 1]).into(),
        ])
        .unwrap()
        .lazy();
        dump_jsonl(&frame, path.to_str().unwrap()).unwrap();
        let contents = fs::read_to_string(path).unwrap();
        let rows = contents
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        let row = &rows[0];
        assert_eq!(row["empty"], serde_json::json!({}));
        assert_eq!(row["nested"]["elapsed"], 1);
        assert_eq!(row["items"], serde_json::json!([3, 4]));
        assert_eq!(row["large"], serde_json::Value::from(u64::MAX));
        assert!(rows[1]["items"].is_null());
    }
}
