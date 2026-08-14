use crate::controllers::log::LogController;
use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use polars::prelude::*;
use rayon::prelude::*; // Re-enabled for parallel processing
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
// Performance optimization constants
const OPTIMAL_CHUNK_SIZE: usize = 8192; // Optimized chunk size for CSV reading
const PARALLEL_THRESHOLD: usize = 2; // Minimum files to use parallel processing
const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024; // 100MB threshold for large files
const GZIP_BUFFER_SIZE: usize = 16 * 1024 * 1024; // 16MB buffer for gzip (increased from 8MB)

pub fn separator_byte(separator: &str) -> Result<u8, QuiltError> {
    let mut chars = separator.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if ch.is_ascii() => Ok(ch as u8),
        (None, _) => Err(QuiltError::usage(
            "Separator must be a single ASCII character, got empty string",
        )),
        _ => Err(QuiltError::usage(format!(
            "Separator must be a single ASCII character, got '{separator}'"
        ))),
    }
}

// Environment variable helpers for unified configuration
fn get_env_chunk_size() -> Result<Option<usize>, QuiltError> {
    let Ok(raw) = std::env::var("QLT_CHUNK_SIZE") else {
        return Ok(None);
    };
    parse_chunk_size(&raw).map(Some)
}

fn parse_chunk_size(raw: &str) -> Result<usize, QuiltError> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| QuiltError::usage("QLT_CHUNK_SIZE must be a positive integer"))?;
    if value == 0 {
        return Err(QuiltError::usage(
            "QLT_CHUNK_SIZE must be greater than zero",
        ));
    }
    Ok(value)
}

// Utility function to check if file paths exist
pub fn exists_path(paths: &[impl AsRef<Path>]) -> Result<(), QuiltError> {
    for path in paths {
        if !path.as_ref().exists() {
            return Err(QuiltError::Io {
                operation: "load".into(),
                path: Some(path.as_ref().display().to_string()),
                message: "Error: File not found".into(),
            });
        }
    }
    Ok(())
}

// Get optimized CSV reader options for better performance
fn get_optimized_csv_options(
    separator: &str,
    has_header: bool,
    low_memory: bool,
    chunk_size: Option<usize>,
    file_size: Option<u64>,
) -> Result<CsvReadOptions, QuiltError> {
    let sep_byte = separator_byte(separator)?;

    // An explicit CLI value wins and therefore does not consult the environment.
    let optimized_chunk_size = match chunk_size {
        Some(value) if value > 0 => value,
        Some(_) => return Err(QuiltError::usage("chunk size must be greater than zero")),
        None => get_env_chunk_size()?.unwrap_or(match file_size {
            Some(size) if size > LARGE_FILE_THRESHOLD => OPTIMAL_CHUNK_SIZE * 2,
            _ => OPTIMAL_CHUNK_SIZE,
        }),
    };

    let mut options = CsvReadOptions::default()
        .with_has_header(has_header)
        .with_low_memory(low_memory)
        .with_chunk_size(optimized_chunk_size)
        // CSV schema inference follows Polars' reader defaults; NDJSON has a
        // separate bounded inference option in the load command.
        .map_parse_options(|parse_opts| {
            parse_opts.with_separator(sep_byte)
            // Date conversion is explicit through the shared datetime commands.
        });

    // For large files, use additional optimizations
    if let Some(size) = file_size {
        if size > LARGE_FILE_THRESHOLD {
            options = options.with_low_memory(true); // Force low memory for large files
        }
    }

    Ok(options)
}
pub struct CsvController {
    paths: Vec<PathBuf>,
}
impl CsvController {
    pub fn new(paths: &[PathBuf]) -> Self {
        Self {
            paths: paths.to_vec(),
        }
    }
    pub fn get_dataframe_with_resources(
        &self,
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
        resources: &ExecutionResources,
    ) -> Result<LazyFrame, QuiltError> {
        separator_byte(separator)?;
        if self.paths.len() == 1 {
            let path = &self.paths[0];
            self.read_csv_file(
                path, separator, low_memory, no_headers, chunk_size, resources,
            )
        } else {
            self.concat_csv_files(separator, low_memory, no_headers, chunk_size, resources)
        }
    }
    fn read_csv_file(
        &self,
        path: &Path,
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
        resources: &ExecutionResources,
    ) -> Result<LazyFrame, QuiltError> {
        LogController::debug("Reading CSV file");
        let has_header = !no_headers;
        // Check if file is gzipped based on extension
        let is_gzipped = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase() == "gz")
            .unwrap_or(false);
        if is_gzipped {
            LogController::debug("Reading gzipped CSV file");
            self.read_gzipped_csv_file(
                path, separator, low_memory, has_header, chunk_size, resources,
            )
        } else {
            // Get file size for optimization
            let file_size = std::fs::metadata(path).ok().map(|m| m.len());

            // Use optimized CSV options
            let csv_options = get_optimized_csv_options(
                separator, has_header, low_memory, chunk_size, file_size,
            )?;

            LogController::debug(&format!(
                "Reading CSV file (size_mb={})",
                file_size.map(|s| s / 1024 / 1024).unwrap_or(0)
            ));

            let reader = LazyCsvReader::new(path)
                .with_separator(csv_options.parse_options.separator)
                .with_has_header(csv_options.has_header)
                .with_low_memory(csv_options.low_memory)
                .with_chunk_size(csv_options.chunk_size)
                // CSV schema inference follows the reader defaults.
                .finish();

            match reader {
                Ok(df) => Ok(crate::controllers::resources::instrument_evaluation(
                    df, resources,
                )),
                Err(e) => Err(QuiltError::Io {
                    operation: "read CSV".into(),
                    path: Some(path.display().to_string()),
                    message: e.to_string(),
                }),
            }
        }
    }

    fn read_gzipped_csv_file(
        &self,
        path: &Path,
        separator: &str,
        low_memory: bool,
        has_header: bool,
        chunk_size: Option<usize>,
        resources: &ExecutionResources,
    ) -> Result<LazyFrame, QuiltError> {
        use flate2::read::GzDecoder;
        use std::io::{BufReader, BufWriter, Read};

        if !resources.temp_files_enabled() {
            return Err(QuiltError::usage(
                "run --show-plan cannot inspect gzipped CSV without execution",
            ));
        }

        let source = File::open(path).map_err(|e| QuiltError::Io {
            operation: "open gzip".into(),
            path: Some(path.display().to_string()),
            message: e.to_string(),
        })?;

        LogController::debug("Spooling gzip payload to temporary file");

        let mut reservation = resources
            .reserve_temp_file("qlt-gzip-spool", "csv")
            .map_err(|e| QuiltError::Io {
                operation: "create gzip spool".into(),
                path: Some(path.display().to_string()),
                message: e.to_string(),
            })?;

        let mut gz_decoder = GzDecoder::new(BufReader::new(source));
        let temp_path = reservation.path().to_path_buf();
        let Some(spool_file) = reservation.file_mut() else {
            return Err(QuiltError::Io {
                operation: "create gzip spool".into(),
                path: Some(temp_path.display().to_string()),
                message: "temporary spool reservation was already consumed".into(),
            });
        };
        let mut spool_writer = BufWriter::new(spool_file);
        let mut total_written = 0usize;
        let mut buffer = vec![0u8; GZIP_BUFFER_SIZE];

        loop {
            match gz_decoder.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = spool_writer.write_all(&buffer[..n]) {
                        return Err(QuiltError::Io {
                            operation: "write gzip spool".into(),
                            path: Some(temp_path.display().to_string()),
                            message: e.to_string(),
                        });
                    }
                    total_written += n;
                }
                Err(e) => {
                    return Err(QuiltError::Io {
                        operation: "decompress gzip".into(),
                        path: Some(path.display().to_string()),
                        message: e.to_string(),
                    });
                }
            }
        }

        if let Err(e) = spool_writer.flush() {
            return Err(QuiltError::Io {
                operation: "flush gzip spool".into(),
                path: Some(temp_path.display().to_string()),
                message: e.to_string(),
            });
        }
        drop(spool_writer);

        LogController::debug(&format!(
            "Decompressed {}MB to spool file",
            total_written / (1024 * 1024)
        ));

        let csv_options = get_optimized_csv_options(
            separator,
            has_header,
            low_memory,
            chunk_size,
            Some(total_written as u64),
        )?;

        let reader = LazyCsvReader::new(&temp_path)
            .with_separator(csv_options.parse_options.separator)
            .with_has_header(csv_options.has_header)
            .with_low_memory(csv_options.low_memory)
            .with_chunk_size(csv_options.chunk_size)
            .finish()
            .map_err(|e| QuiltError::Io {
                operation: "scan gzip CSV".into(),
                path: Some(path.display().to_string()),
                message: e.to_string(),
            })?;
        resources
            .retain_temp_file(reservation)
            .map_err(|e| QuiltError::Io {
                operation: "retain gzip spool".into(),
                path: Some(temp_path.display().to_string()),
                message: e.to_string(),
            })?;
        Ok(reader)
    }
    fn concat_csv_files(
        &self,
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
        resources: &ExecutionResources,
    ) -> Result<LazyFrame, QuiltError> {
        LogController::debug(&format!("Reading {} CSV files", self.paths.len()));

        // Use parallel processing for multiple files if threshold is met
        let dataframes: Vec<LazyFrame> = if self.paths.len() >= PARALLEL_THRESHOLD {
            LogController::debug("Using parallel file reading for better performance");
            self.paths
                .par_iter() // Enabled parallel processing
                .map(|path| {
                    self.read_csv_file(
                        path, separator, low_memory, no_headers, chunk_size, resources,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            // Sequential for small number of files
            self.paths
                .iter()
                .map(|path| {
                    self.read_csv_file(
                        path, separator, low_memory, no_headers, chunk_size, resources,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        concat(
            dataframes,
            UnionArgs {
                parallel: true,
                rechunk: true,
                ..Default::default()
            },
        )
        .map_err(|e| QuiltError::Operation {
            operation: "concatenate CSV files".into(),
            message: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_size_parser_rejects_zero_malformed_and_overflow() {
        assert_eq!(parse_chunk_size("42").unwrap(), 42);
        assert!(parse_chunk_size("0").is_err());
        assert!(parse_chunk_size("not-a-number").is_err());
        assert!(parse_chunk_size("999999999999999999999999999999").is_err());
    }

    #[test]
    fn gzip_decompression_buffer_is_fixed_and_bounded() {
        assert_eq!(GZIP_BUFFER_SIZE, 16 * 1024 * 1024);
    }

    #[test]
    fn gzip_load_returns_pending_scan_with_execution_owned_spool() {
        let path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample-min.csv.gz"
        ));
        let resources = ExecutionResources::new();
        let frame = CsvController::new(&[path])
            .get_dataframe_with_resources(",", false, false, None, &resources)
            .unwrap();
        let plan = frame.describe_plan().unwrap();
        assert!(plan.contains("CSV SCAN") || plan.contains("Csv SCAN"));
        assert_eq!(resources.tracked_count(), 1);
        let spool_paths = resources.tracked_paths();
        assert_eq!(spool_paths.len(), 1);
        assert!(spool_paths[0].exists());
        drop(frame);
        assert_eq!(resources.tracked_count(), 1);
        let cloned_resources = resources.clone();
        drop(resources);
        assert!(spool_paths[0].exists());
        drop(cloned_resources);
        assert!(!spool_paths[0].exists());
    }
}
