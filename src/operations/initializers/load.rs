use crate::controllers::csv::{exists_path, CsvController};
use crate::controllers::log::LogController;
use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use glob::glob;
use polars::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Full inference is the safe default; bounded samples are validated against
/// the remaining records and reject late fields instead of dropping them.
pub const DEFAULT_NDJSON_INFER_SCHEMA_LENGTH: Option<usize> = None;

fn has_glob_pattern(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains('*') || path_str.contains('?') || path_str.contains('[')
}

pub(crate) fn expand_input_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, QuiltError> {
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
        DEFAULT_NDJSON_INFER_SCHEMA_LENGTH,
        None,
        resources,
    )
}

/// Load input with full NDJSON validation and lossless integer inference.
/// `infer_schema_length == None` discovers all fields; bounded discovery rejects late fields.
#[allow(clippy::too_many_arguments)]
pub fn load_with_ndjson_inference_with_resources(
    paths: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
    infer_schema_length: Option<usize>,
    source_column: Option<&str>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    if infer_schema_length == Some(0) {
        return Err(QuiltError::usage(
            "NDJSON inference length must be positive or omitted for full inference",
        ));
    }
    if source_column.is_some_and(|name| name.trim().is_empty()) {
        return Err(QuiltError::usage("source column name must not be empty"));
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
            .map(|ext| {
                ext.eq_ignore_ascii_case("jsonl")
                    || ext.eq_ignore_ascii_case("ndjson")
                    || super::ndjson::is_gzip(path)
            })
            .unwrap_or(false)
    });
    let has_csv = expanded_paths.iter().any(|path| {
        if super::ndjson::is_gzip(path) {
            return false;
        }
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
        load_parquet_files(&expanded_paths, source_column, resources)
    } else if has_ndjson {
        load_ndjson_files(
            &expanded_paths,
            infer_schema_length,
            source_column,
            resources,
        )
    } else {
        load_csv_files(
            &expanded_paths,
            separator,
            low_memory,
            no_headers,
            chunk_size,
            source_column,
            resources,
        )
    }
}

fn load_ndjson_files(
    original_paths: &[PathBuf],
    infer_schema_length: Option<usize>,
    source_column: Option<&str>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    let paths = original_paths
        .iter()
        .map(|path| super::ndjson::prepare_path(path, resources))
        .collect::<Result<Vec<_>, _>>()?;
    let (merged_schema, empty_files) =
        super::ndjson::schema(&paths, original_paths, infer_schema_length)?;

    if merged_schema.is_empty() {
        return Err(QuiltError::schema(
            "load",
            None::<String>,
            "cannot infer NDJSON schema: empty inputs or only empty objects have no columns",
        ));
    }

    let schema = Arc::new(merged_schema);
    let frames = paths
        .iter()
        .zip(&empty_files)
        .zip(original_paths)
        .map(|((path, empty), original)| {
            if *empty {
                return add_source_column(
                    DataFrame::empty_with_schema(schema.as_ref()).lazy(),
                    original,
                    source_column,
                );
            }
            LazyJsonLineReader::new(path)
                .with_schema(Some(schema.clone()))
                .with_rechunk(true)
                .finish()
                .map_err(|error| QuiltError::Io {
                    operation: "read NDJSON".into(),
                    path: Some(path.display().to_string()),
                    message: error.to_string(),
                })
                .and_then(|frame| add_source_column(frame, original, source_column))
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

fn load_parquet_files(
    paths: &[PathBuf],
    source_column: Option<&str>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    if paths.len() == 1 {
        LazyFrame::scan_parquet(&paths[0], ScanArgsParquet::default())
            .map_err(|e| QuiltError::Io {
                operation: "read parquet".into(),
                path: Some(paths[0].display().to_string()),
                message: e.to_string(),
            })
            .and_then(|frame| add_source_column(frame, &paths[0], source_column))
            .map(|frame| crate::controllers::resources::instrument_evaluation(frame, resources))
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
                add_source_column(df, path, source_column)?,
                resources,
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
    source_column: Option<&str>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    if source_column.is_some() {
        let frames = paths
            .iter()
            .map(|path| {
                let frame = CsvController::new(std::slice::from_ref(path))
                    .get_dataframe_with_resources(
                        separator, low_memory, no_headers, chunk_size, resources,
                    )?;
                add_source_column(frame, path, source_column)
            })
            .collect::<Result<Vec<_>, QuiltError>>()?;
        return concat(
            frames,
            UnionArgs {
                parallel: true,
                rechunk: true,
                ..Default::default()
            },
        )
        .map_err(|error| QuiltError::operation("concatenate CSV files", error.to_string()));
    }
    CsvController::new(paths)
        .get_dataframe_with_resources(separator, low_memory, no_headers, chunk_size, resources)
}

fn add_source_column(
    mut frame: LazyFrame,
    path: &Path,
    source_column: Option<&str>,
) -> Result<LazyFrame, QuiltError> {
    if let Some(name) = source_column {
        let schema = frame
            .collect_schema()
            .map_err(|error| QuiltError::schema("load", None::<String>, error.to_string()))?;
        if schema.get(name).is_some() {
            return Err(QuiltError::schema(
                "load",
                Some(name),
                "source column already exists in input",
            ));
        }
        frame = frame.with_column(lit(path.to_string_lossy().into_owned()).alias(name));
    }
    Ok(frame)
}
