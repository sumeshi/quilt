//! Full-data export for the interactive editor.
//!
//! Keep exports on the same serialization path as the CLI `dump` command.
//! The surrounding atomic stage lets us check cancellation after the streaming
//! sink finishes and before anything becomes visible at the requested path.

use super::task::TaskControl;
use crate::controllers::arguments::OutputFormat;
use crate::controllers::pipeline::Pipeline;
use crate::error::QuiltError;
use crate::operations::finalizers::{dump, jsonl};
use std::path::Path;

pub(super) fn export(
    pipeline: &Pipeline,
    path: &Path,
    format: OutputFormat,
    task: &TaskControl,
) -> Result<(), QuiltError> {
    task.check()?;
    if path == Path::new("-") {
        return Err(QuiltError::usage(
            "Error: TUI export requires a file path; standard output is not available in the editor.",
        ));
    }
    path.to_str()
        .ok_or_else(|| QuiltError::usage("Error: TUI export path must be valid UTF-8"))?;

    crate::operations::finalizers::atomic_path_checked(
        path,
        "write TUI export",
        |staged_path| {
            let staged_output = staged_path
                .to_str()
                .ok_or_else(|| QuiltError::usage("Error: TUI export path must be valid UTF-8"))?;
            match format {
                OutputFormat::Csv => {
                    dump::dump(pipeline.frame(), Some(staged_output), ',')?;
                }
                OutputFormat::Jsonl => {
                    jsonl::dump_jsonl(pipeline.frame(), staged_output)?;
                }
            }
            Ok(())
        },
        || task.check(),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use polars::prelude::*;
    use std::fs;

    fn pipeline(frame: DataFrame) -> Pipeline {
        Pipeline::new(
            frame.lazy(),
            crate::controllers::resources::ExecutionResources::default(),
        )
    }

    #[test]
    fn full_export_preserves_nested_values_and_exact_integers() {
        let directory = tempfile::tempdir().unwrap();
        let jsonl_path = directory.path().join("result.jsonl");
        let csv_path = directory.path().join("result.csv");
        let nested_value = Series::new("n".into(), &[u64::MAX]);
        let nested = StructChunked::from_series("nested".into(), 1, [&nested_value].into_iter())
            .unwrap()
            .into_series();
        let frame = DataFrame::new_infer_height(vec![
            nested.into(),
            Series::new("large".into(), &[u64::MAX]).into(),
        ])
        .unwrap();
        let pipeline = pipeline(frame);
        let task = TaskControl::new();

        export(&pipeline, &jsonl_path, OutputFormat::Jsonl, &task).unwrap();
        let json: serde_json::Value = serde_json::from_str(
            fs::read_to_string(&jsonl_path)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(json["large"], serde_json::Value::from(u64::MAX));
        assert_eq!(json["nested"]["n"], serde_json::Value::from(u64::MAX));

        export(&pipeline, &csv_path, OutputFormat::Csv, &task).unwrap();
        let mut reader = csv::Reader::from_path(&csv_path).unwrap();
        let headers = reader.headers().unwrap().clone();
        let record = reader.records().next().unwrap().unwrap();
        let nested_index = headers.iter().position(|name| name == "nested").unwrap();
        let large_index = headers.iter().position(|name| name == "large").unwrap();
        let nested_json: serde_json::Value =
            serde_json::from_str(record.get(nested_index).unwrap()).unwrap();
        assert_eq!(nested_json["n"], serde_json::Value::from(u64::MAX));
        assert_eq!(record.get(large_index), Some("18446744073709551615"));
    }

    #[test]
    fn export_refuses_overwrite_and_cleans_failed_stages() {
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join("existing.jsonl");
        fs::write(&existing, "keep me").unwrap();
        let task = TaskControl::new();
        let ordinary = pipeline(df!("value" => &[1i64]).unwrap());
        assert!(export(&ordinary, &existing, OutputFormat::Jsonl, &task).is_err());
        assert_eq!(fs::read_to_string(&existing).unwrap(), "keep me");

        let failed = directory.path().join("failed.jsonl");
        let non_finite = pipeline(df!("value" => &[f64::INFINITY]).unwrap());
        assert!(export(&non_finite, &failed, OutputFormat::Jsonl, &task).is_err());
        assert!(!failed.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn cancelled_export_does_not_publish_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cancelled.csv");
        let task = TaskControl::new();
        task.cancel();
        let pipeline = pipeline(df!("value" => &[1i64]).unwrap());
        assert!(export(&pipeline, &path, OutputFormat::Csv, &task).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
