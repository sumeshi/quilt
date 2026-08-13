use crate::controllers::csv::{exists_path, CsvController};
use crate::controllers::log::LogController;
use glob::glob;
use polars::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn has_glob_pattern(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains('*') || path_str.contains('?') || path_str.contains('[')
}

fn expand_input_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
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
                                eprintln!("Error while expanding glob '{pattern}': {e}");
                                std::process::exit(1);
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Invalid glob pattern '{pattern}': {e}");
                    std::process::exit(1);
                }
            }

            if matches.is_empty() {
                eprintln!("No files found matching pattern: {pattern}");
                std::process::exit(1);
            }

            expanded_paths.extend(matches);
        } else {
            expanded_paths.push(path.clone());
        }
    }

    expanded_paths
}

pub fn load(
    paths: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
) -> LazyFrame {
    let expanded_paths = expand_input_paths(paths);

    if !exists_path(&expanded_paths) {
        eprintln!("One or more files do not exist");
        std::process::exit(1);
    }
    LogController::debug(&format!(
        "{} files are loaded. [{}]",
        expanded_paths.len(),
        expanded_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
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
        eprintln!("Error: Cannot mix CSV, parquet, and NDJSON files in the same load command");
        std::process::exit(1);
    }
    if has_parquet {
        load_parquet_files(&expanded_paths)
    } else if has_ndjson {
        load_ndjson_files(&expanded_paths)
    } else {
        load_csv_files(
            &expanded_paths,
            separator,
            low_memory,
            no_headers,
            chunk_size,
        )
    }
}

fn load_ndjson_files(paths: &[PathBuf]) -> LazyFrame {
    // Polars infers each file without materializing its rows. We merge those metadata
    // schemas so sparse columns present only in later files are retained.
    let mut merged_schema = Schema::default();
    for path in paths {
        let file_schema = LazyJsonLineReader::new(path)
            .with_infer_schema_length(None)
            .finish()
            .and_then(|mut frame| frame.collect_schema())
            .unwrap_or_else(|error| {
                eprintln!(
                    "Error inferring NDJSON schema from {}: {error}",
                    path.display()
                );
                std::process::exit(1);
            });

        for (name, dtype) in file_schema.iter() {
            let dtype = match merged_schema.get(name) {
                Some(existing) => merge_ndjson_dtype(existing, dtype).unwrap_or_else(|error| {
                    eprintln!(
                        "Error merging NDJSON schema for column '{name}' in {}: {error}",
                        path.display()
                    );
                    std::process::exit(1);
                }),
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
                .unwrap_or_else(|error| {
                    eprintln!("Error reading NDJSON file {}: {error}", path.display());
                    std::process::exit(1);
                })
        })
        .collect::<Vec<_>>();

    concat(
        frames,
        UnionArgs {
            parallel: true,
            rechunk: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|error| {
        eprintln!("Error concatenating NDJSON files: {error}");
        std::process::exit(1);
    })
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

fn load_parquet_files(paths: &[PathBuf]) -> LazyFrame {
    if paths.len() == 1 {
        LazyFrame::scan_parquet(&paths[0], ScanArgsParquet::default()).unwrap_or_else(|e| {
            eprintln!("Error reading parquet file {}: {}", paths[0].display(), e);
            std::process::exit(1);
        })
    } else {
        let mut dataframes = Vec::new();
        for path in paths {
            let df =
                LazyFrame::scan_parquet(path, ScanArgsParquet::default()).unwrap_or_else(|e| {
                    eprintln!("Error reading parquet file {}: {}", path.display(), e);
                    std::process::exit(1);
                });
            dataframes.push(df);
        }
        concat(
            dataframes,
            UnionArgs {
                parallel: true,
                rechunk: true,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| {
            eprintln!("Error concatenating parquet files: {e}");
            std::process::exit(1);
        })
    }
}

fn load_csv_files(
    paths: &[PathBuf],
    separator: &str,
    low_memory: bool,
    no_headers: bool,
    chunk_size: Option<usize>,
) -> LazyFrame {
    CsvController::new(paths).get_dataframe(separator, low_memory, no_headers, chunk_size)
}
