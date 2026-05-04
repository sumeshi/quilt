use crate::controllers::csv::{exists_path, CsvController};
use crate::controllers::log::LogController;
use glob::glob;
use polars::prelude::*;
use std::path::{Path, PathBuf};

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
    // Check if any files are parquet
    let has_parquet = expanded_paths.iter().any(|path| {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase() == "parquet")
            .unwrap_or(false)
    });
    let has_csv = expanded_paths.iter().any(|path| {
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase());
        matches!(ext, Some(ref e) if e == "csv" || e == "tsv" || e == "gz" || e == "txt")
            || ext.is_none() // Files without extension are assumed to be CSV
    });
    // Cannot mix parquet and CSV files
    if has_parquet && has_csv {
        eprintln!("Error: Cannot mix parquet and CSV files in the same load command");
        std::process::exit(1);
    }
    if has_parquet {
        load_parquet_files(&expanded_paths)
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
fn load_parquet_files(paths: &[PathBuf]) -> LazyFrame {
    if paths.len() == 1 {
        LazyFrame::scan_parquet(&paths[0], ScanArgsParquet::default()).unwrap_or_else(|e| {
            eprintln!("Error reading parquet file {}: {}", paths[0].display(), e);
            std::process::exit(1);
        })
    } else {
        // Concatenate multiple parquet files
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
