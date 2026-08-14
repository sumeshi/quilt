use crate::controllers::csv::{exists_path, CsvController};
use crate::controllers::log::LogController;
use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use glob::glob;
use polars::prelude::*;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Number of NDJSON records inspected per file by default.  Keeping this
/// bounded prevents a load command from scanning an entire large log merely
/// to construct its lazy schema.  Callers that need complete inference can
/// opt into it explicitly with `--infer-schema-length full`.
pub const DEFAULT_NDJSON_INFER_SCHEMA_LENGTH: usize = 1_000;

fn has_glob_pattern(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains('*') || path_str.contains('?') || path_str.contains('[')
}

fn expand_input_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, QuiltError> {
    let mut expanded_paths = Vec::new();

    for path in paths {
        if has_glob_pattern(path) {
            let pattern = path.to_string_lossy();
            let mut matches = Vec::new();

            match glob(&pattern) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(matched_path) => matches.push(matched_path),
                            Err(e) => {
                                return Err(QuiltError::usage(format!(
                                    "Error while expanding glob '{pattern}': {e}"
                                )))
                            }
                        }
                    }
                }
                Err(e) => {
                    return Err(QuiltError::usage(format!(
                        "Invalid glob pattern '{pattern}': {e}"
                    )))
                }
            }

            if matches.is_empty() {
                return Err(QuiltError::usage(format!(
                    "No files found matching pattern: {pattern}"
                )));
            }

            expanded_paths.extend(matches);
        } else {
            expanded_paths.push(path.clone());
        }
    }

    Ok(expanded_paths)
}

pub fn load(
    paths: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    load_with_ndjson_inference_with_resources(
        paths,
        separator,
        low_memory,
        no_headers,
        chunk_size,
        Some(DEFAULT_NDJSON_INFER_SCHEMA_LENGTH),
        resources,
    )
}

/// Load input files while retaining a bounded NDJSON schema inference policy.
/// `infer_schema_length == None` requests Polars' full inference mode.
pub fn load_with_ndjson_inference_with_resources(
    paths: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
    infer_schema_length: Option<usize>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    if infer_schema_length == Some(0) {
        return Err(QuiltError::usage(
            "NDJSON inference length must be positive or omitted for full inference",
        ));
    }
    let expanded_paths = expand_input_paths(paths)?;

    exists_path(&expanded_paths)?;
    LogController::debug(&format!("Loading {} input files", expanded_paths.len()));
    let has_parquet = expanded_paths.iter().any(|path| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("parquet"))
            .unwrap_or(false)
    });
    let has_ndjson = expanded_paths.iter().any(|path| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("jsonl") || ext.eq_ignore_ascii_case("ndjson"))
            .unwrap_or(false)
    });
    let has_csv = expanded_paths.iter().any(|path| {
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        matches!(ext, Some(ref e) if e == "csv" || e == "tsv" || e == "gz" || e == "txt")
            || ext.is_none()
    });
    let family_count = [has_parquet, has_csv, has_ndjson]
        .into_iter()
        .filter(|present| *present)
        .count();
    if family_count > 1 {
        return Err(QuiltError::usage(
            "Error: Cannot mix CSV, parquet, and NDJSON files in the same load command",
        ));
    }
    if has_parquet {
        load_parquet_files(&expanded_paths, resources)
    } else if has_ndjson {
        load_ndjson_files(&expanded_paths, infer_schema_length)
    } else {
        load_csv_files(
            &expanded_paths,
            separator,
            low_memory,
            no_headers,
            chunk_size,
            resources,
        )
    }
}

fn load_ndjson_files(
    paths: &[PathBuf],
    infer_schema_length: Option<usize>,
) -> Result<LazyFrame, QuiltError> {
    // Polars infers each file without materializing its rows. We merge those metadata
    // schemas so sparse columns present only in later files are retained.
    let mut merged_schema = Schema::default();
    for path in paths {
        let file_schema = LazyJsonLineReader::new(path)
            .with_infer_schema_length(infer_schema_length.and_then(NonZeroUsize::new))
            .finish()
            .and_then(|mut frame| frame.collect_schema())
            .map_err(|error| QuiltError::Schema {
                operation: "infer NDJSON schema".into(),
                column: None,
                message: format!("{}: {error}", path.display()),
            })?;

        for (name, dtype) in file_schema.iter() {
            let dtype = match merged_schema.get(name) {
                Some(existing) => {
                    merge_ndjson_dtype(existing, dtype).map_err(|error| QuiltError::Schema {
                        operation: "merge NDJSON schema".into(),
                        column: Some(name.to_string()),
                        message: format!("{}: {error}", path.display()),
                    })?
                }
                None => dtype.clone(),
            };
            merged_schema.insert(name.clone(), dtype);
        }
    }

    let schema = Arc::new(merged_schema);
    let frames = paths
        .iter()
        .map(|path| {
            LazyJsonLineReader::new(path)
                .with_schema(Some(schema.clone()))
                .with_rechunk(true)
                .finish()
                .map_err(|error| QuiltError::Io {
                    operation: "read NDJSON".into(),
                    path: Some(path.display().to_string()),
                    message: error.to_string(),
                })
        })
        .collect::<Result<Vec<_>, QuiltError>>()?;

    concat(
        frames,
        UnionArgs {
            parallel: true,
            rechunk: true,
            ..Default::default()
        },
    )
    .map_err(|error| QuiltError::operation("concatenate NDJSON files", error.to_string()))
}

fn merge_ndjson_dtype(left: &DataType, right: &DataType) -> PolarsResult<DataType> {
    if left == right {
        return Ok(left.clone());
    }
    if matches!(left, DataType::Null) {
        return Ok(right.clone());
    }
    if matches!(right, DataType::Null) {
        return Ok(left.clone());
    }
    if matches!(
        (left, right),
        (DataType::Int64, DataType::UInt64)
            | (DataType::UInt64, DataType::Int64)
            | (DataType::Int64, DataType::Float64)
            | (DataType::Float64, DataType::Int64)
            | (DataType::UInt64, DataType::Float64)
            | (DataType::Float64, DataType::UInt64)
    ) {
        return Ok(DataType::Float64);
    }
    match (left, right) {
        (DataType::List(left_inner), DataType::List(right_inner)) => Ok(DataType::List(Box::new(
            merge_ndjson_dtype(left_inner, right_inner)?,
        ))),
        (DataType::Struct(left_fields), DataType::Struct(right_fields)) => {
            let mut fields = left_fields.clone();
            for right_field in right_fields {
                if let Some(left_field) = fields
                    .iter_mut()
                    .find(|left_field| left_field.name() == right_field.name())
                {
                    let dtype = merge_ndjson_dtype(left_field.dtype(), right_field.dtype())?;
                    *left_field = Field::new(left_field.name().clone(), dtype);
                } else {
                    fields.push(right_field.clone());
                }
            }
            Ok(DataType::Struct(fields))
        }
        _ => Err(PolarsError::ComputeError(
            format!("incompatible NDJSON types: {left} and {right}").into(),
        )),
    }
}

fn load_parquet_files(
    paths: &[PathBuf],
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    if paths.len() == 1 {
        LazyFrame::scan_parquet(&paths[0], ScanArgsParquet::default())
            .map(|frame| crate::controllers::resources::instrument_evaluation(frame, resources))
            .map_err(|e| QuiltError::Io {
                operation: "read parquet".into(),
                path: Some(paths[0].display().to_string()),
                message: e.to_string(),
            })
    } else {
        let mut dataframes = Vec::new();
        for path in paths {
            let df = LazyFrame::scan_parquet(path, ScanArgsParquet::default()).map_err(|e| {
                QuiltError::Io {
                    operation: "read parquet".into(),
                    path: Some(path.display().to_string()),
                    message: e.to_string(),
                }
            })?;
            dataframes.push(crate::controllers::resources::instrument_evaluation(
                df, resources,
            ));
        }
        concat(
            dataframes,
            UnionArgs {
                parallel: true,
                rechunk: true,
                ..Default::default()
            },
        )
        .map_err(|e| QuiltError::operation("concatenate parquet files", e.to_string()))
    }
}

fn load_csv_files(
    paths: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    CsvController::new(paths)
        .get_dataframe_with_resources(separator, low_memory, no_headers, chunk_size, resources)
}
