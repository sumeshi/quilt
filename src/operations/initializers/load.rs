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

/// Optional input behaviors used by the command layer. Existing callers can keep
/// using the compatibility wrappers below.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadOptions<'a> {
    pub stdin_format: Option<&'a str>,
    pub rejects_path: Option<&'a Path>,
    pub csv_options: Option<&'a crate::controllers::csv_input::CsvInputOptions>,
}

fn has_glob_pattern(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains('*') || path_str.contains('?') || path_str.contains('[')
}

pub(crate) fn expand_input_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, QuiltError> {
    let mut expanded_paths = Vec::new();

    for path in paths {
        if has_glob_pattern(path) && !path.exists() {
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
                                )));
                            }
                        }
                    }
                }
                Err(e) => {
                    return Err(QuiltError::usage(format!(
                        "Invalid glob pattern '{pattern}': {e}"
                    )));
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
    load_with_options(
        paths,
        separator,
        low_memory,
        no_headers,
        chunk_size,
        DEFAULT_NDJSON_INFER_SCHEMA_LENGTH,
        None,
        LoadOptions::default(),
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
    load_with_options(
        paths,
        separator,
        low_memory,
        no_headers,
        chunk_size,
        infer_schema_length,
        source_column,
        LoadOptions::default(),
        resources,
    )
}

/// Load with stdin selection and optional NDJSON quarantine.
#[allow(clippy::too_many_arguments)]
pub fn load_with_options(
    paths: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
    infer_schema_length: Option<usize>,
    source_column: Option<&str>,
    options: LoadOptions<'_>,
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
    if let Some(format) = options.stdin_format {
        super::stdin::validate_format(format)?;
        if !paths.is_empty() {
            return Err(QuiltError::usage(
                "--stdin-format can only be used when load has no file inputs",
            ));
        }
    }
    let rejects_path = options
        .rejects_path
        .or_else(|| options.csv_options.and_then(|csv| csv.rejects.as_deref()));
    if rejects_path.is_some() && !resources.temp_files_enabled() {
        return Err(QuiltError::usage(
            "run --show-plan cannot inspect rejects without execution",
        ));
    }
    let has_stdin = paths.is_empty();
    let input_paths = if let Some(format) = options.stdin_format {
        if !has_stdin {
            return Err(QuiltError::usage(
                "--stdin-format can only be used when load has no file inputs",
            ));
        }
        vec![super::stdin::spool(format, resources)?]
    } else if paths.is_empty() {
        return Err(QuiltError::usage("load requires an input file"));
    } else {
        paths.to_vec()
    };
    let stdin_input = options.stdin_format.is_some();
    let input_separator = if options
        .stdin_format
        .is_some_and(|format| format.eq_ignore_ascii_case("tsv"))
    {
        "\t"
    } else {
        separator
    };
    let expanded_paths = expand_input_paths(&input_paths)?;
    let source_labels = if stdin_input {
        vec![PathBuf::from("<stdin>")]
    } else {
        expanded_paths.clone()
    };
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
    let csv_options_supplied = options
        .csv_options
        .is_some_and(|csv| csv.all_strings || !csv.schema.is_empty());
    if csv_options_supplied && (has_ndjson || has_parquet) {
        return Err(QuiltError::usage(
            "CSV schema options cannot be used with NDJSON or parquet inputs",
        ));
    }
    if has_parquet {
        if rejects_path.is_some() {
            return Err(QuiltError::usage(
                "--rejects is supported only for CSV and NDJSON inputs",
            ));
        }
        load_parquet_files(&expanded_paths, source_column, resources)
    } else if has_ndjson {
        load_ndjson_files(
            &expanded_paths,
            &source_labels,
            infer_schema_length,
            source_column,
            rejects_path,
            resources,
        )
    } else {
        load_csv_files(
            &expanded_paths,
            &source_labels,
            input_separator,
            low_memory,
            no_headers,
            chunk_size,
            source_column,
            options.csv_options,
            rejects_path,
            resources,
        )
    }
}

fn load_ndjson_files(
    original_paths: &[PathBuf],
    source_labels: &[PathBuf],
    infer_schema_length: Option<usize>,
    source_column: Option<&str>,
    rejects_path: Option<&Path>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    if let Some(report_path) = rejects_path {
        let report = Arc::new(std::sync::Mutex::new(super::rejected::RejectedReport::new(
            report_path,
        )?));
        let inputs = original_paths
            .iter()
            .map(|path| super::ndjson::prepare_path(path, resources))
            .collect::<Result<Vec<_>, _>>()?;
        let (accepted, schema, empty_files, _rejected) = super::ndjson::quarantine(
            &inputs,
            source_labels,
            &report,
            infer_schema_length,
            resources,
        )?;
        if schema.is_empty() {
            Arc::try_unwrap(report)
                .map_err(|_| {
                    QuiltError::operation("publish rejects report", "report is still in use")
                })?
                .into_inner()
                .map_err(|_| {
                    QuiltError::operation("publish rejects report", "report lock is poisoned")
                })?
                .publish()?;
            return Err(QuiltError::schema(
                "load",
                None::<String>,
                "cannot infer NDJSON schema: no accepted records",
            ));
        }
        let schema = Arc::new(schema);
        let frames = accepted
            .iter()
            .zip(&empty_files)
            .zip(source_labels)
            .map(|((path, empty), source)| {
                let frame = if *empty {
                    DataFrame::empty_with_schema(schema.as_ref()).lazy()
                } else {
                    let mut frame = LazyJsonLineReader::new(PlRefPath::try_from_path(path)?)
                        .with_schema(Some(schema.clone()))
                        .with_rechunk(true)
                        .finish()
                        .map_err(|error| QuiltError::Io {
                            operation: "read quarantined NDJSON".into(),
                            path: Some(source.display().to_string()),
                            message: error.to_string(),
                        })?;
                    match &mut frame.logical_plan {
                        DslPlan::Scan {
                            unified_scan_args, ..
                        } => unified_scan_args.glob = false,
                        _ => {
                            return Err(QuiltError::operation(
                                "read quarantined NDJSON",
                                "expected a file scan plan",
                            ));
                        }
                    }
                    frame
                };
                add_source_column(frame, source, source_column)
            })
            .collect::<Result<Vec<_>, QuiltError>>()?;
        let frame = concat(
            frames,
            UnionArgs {
                parallel: true,
                rechunk: true,
                ..Default::default()
            },
        )
        .map_err(|error| {
            QuiltError::operation("concatenate quarantined NDJSON", error.to_string())
        })?;
        Arc::try_unwrap(report)
            .map_err(|_| QuiltError::operation("publish rejects report", "report still in use"))?
            .into_inner()
            .map_err(|_| QuiltError::operation("publish rejects report", "report lock poisoned"))?
            .publish()?;
        return Ok(frame);
    }
    let paths = original_paths
        .iter()
        .map(|path| super::ndjson::prepare_path(path, resources))
        .collect::<Result<Vec<_>, _>>()?;
    let (merged_schema, empty_files) =
        super::ndjson::schema(&paths, source_labels, infer_schema_length)?;

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
        .zip(source_labels)
        .map(|((path, empty), original)| {
            if *empty {
                return add_source_column(
                    DataFrame::empty_with_schema(schema.as_ref()).lazy(),
                    original,
                    source_column,
                );
            }
            let mut frame = LazyJsonLineReader::new(PlRefPath::try_from_path(path)?)
                .with_schema(Some(schema.clone()))
                .with_rechunk(true)
                .finish()
                .map_err(|error| QuiltError::Io {
                    operation: "read NDJSON".into(),
                    path: Some(path.display().to_string()),
                    message: error.to_string(),
                })?;
            // Polars' NDJSON reader hardcodes glob=true and has no setter.
            // Paths have already been expanded and validated by Quilt.
            match &mut frame.logical_plan {
                DslPlan::Scan {
                    unified_scan_args, ..
                } => unified_scan_args.glob = false,
                _ => {
                    return Err(QuiltError::operation(
                        "read NDJSON",
                        "expected a file scan plan",
                    ));
                }
            }
            add_source_column(frame, original, source_column)
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
        LazyFrame::scan_parquet(
            PlRefPath::try_from_path(&paths[0])?,
            ScanArgsParquet {
                glob: false,
                ..Default::default()
            },
        )
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
            let df = LazyFrame::scan_parquet(
                PlRefPath::try_from_path(path)?,
                ScanArgsParquet {
                    glob: false,
                    ..Default::default()
                },
            )
            .map_err(|e| QuiltError::Io {
                operation: "read parquet".into(),
                path: Some(path.display().to_string()),
                message: e.to_string(),
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

// Keep the common load options identical across the format-specific loaders.
#[allow(clippy::too_many_arguments)]
fn load_csv_files(
    paths: &[PathBuf],
    source_labels: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
    source_column: Option<&str>,
    csv_options: Option<&crate::controllers::csv_input::CsvInputOptions>,
    rejects_path: Option<&Path>,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    let mut input_options = csv_options.cloned().unwrap_or_default();
    if let Some(report) = rejects_path {
        input_options.rejects = Some(report.to_path_buf());
    }
    let report = input_options
        .rejects
        .as_ref()
        .map(|path| {
            super::rejected::RejectedReport::new(path)
                .map(|report| Arc::new(std::sync::Mutex::new(report)))
        })
        .transpose()?;
    let frame = if source_column.is_some() {
        let frames = paths
            .iter()
            .zip(source_labels)
            .map(|(path, source)| {
                let frame = CsvController::new(std::slice::from_ref(path))
                    .get_dataframe_with_shared_report(
                        separator,
                        low_memory,
                        no_headers,
                        chunk_size,
                        resources,
                        &input_options,
                        report.clone(),
                        Some(std::slice::from_ref(source)),
                    )?;
                add_source_column(frame, source, source_column)
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
        .map_err(|error| QuiltError::operation("concatenate CSV files", error.to_string()))?
    } else {
        CsvController::new(paths).get_dataframe_with_shared_report(
            separator,
            low_memory,
            no_headers,
            chunk_size,
            resources,
            &input_options,
            report.clone(),
            Some(source_labels),
        )?
    };
    if let Some(report) = report {
        Arc::try_unwrap(report)
            .map_err(|_| QuiltError::operation("publish rejects report", "report is still in use"))?
            .into_inner()
            .map_err(|_| {
                QuiltError::operation("publish rejects report", "report lock is poisoned")
            })?
            .publish()?;
    }
    Ok(frame)
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
