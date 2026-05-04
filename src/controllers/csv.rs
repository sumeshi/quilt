use crate::controllers::log::LogController;
use polars::prelude::*;
use rayon::prelude::*; // Re-enabled for parallel processing
use std::fs::{remove_file, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
// Performance optimization constants
const OPTIMAL_CHUNK_SIZE: usize = 8192; // Optimized chunk size for CSV reading
const PARALLEL_THRESHOLD: usize = 2; // Minimum files to use parallel processing
const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024; // 100MB threshold for large files
const GZIP_BUFFER_SIZE: usize = 16 * 1024 * 1024; // 16MB buffer for gzip (increased from 8MB)

pub fn separator_byte(separator: &str) -> u8 {
    let mut chars = separator.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if ch.is_ascii() => ch as u8,
        (None, _) => {
            eprintln!("Error: Separator must be a single ASCII character, got empty string");
            std::process::exit(1);
        }
        _ => {
            eprintln!("Error: Separator must be a single ASCII character, got '{separator}'");
            std::process::exit(1);
        }
    }
}

// Environment variable helpers for unified configuration
fn get_env_chunk_size() -> Option<usize> {
    std::env::var("QSV_CHUNK_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
}

// Utility function to check if file paths exist
pub fn exists_path(paths: &[impl AsRef<Path>]) -> bool {
    for path in paths {
        if !path.as_ref().exists() {
            eprintln!("Error: File not found: {}", path.as_ref().display());
            return false;
        }
    }
    true
}

// Get optimized CSV reader options for better performance
fn get_optimized_csv_options(
    separator: &str,
    has_header: bool,
    low_memory: bool,
    chunk_size: Option<usize>,
    file_size: Option<u64>,
) -> CsvReadOptions {
    let sep_byte = separator_byte(separator);

    // Prioritize environment variable, then provided chunk_size, then defaults
    let optimized_chunk_size = get_env_chunk_size().or(chunk_size).unwrap_or({
        match file_size {
            Some(size) if size > LARGE_FILE_THRESHOLD => OPTIMAL_CHUNK_SIZE * 2, // Larger chunks for big files
            _ => OPTIMAL_CHUNK_SIZE,
        }
    });

    let mut options = CsvReadOptions::default()
        .with_has_header(has_header)
        .with_low_memory(low_memory)
        .with_chunk_size(optimized_chunk_size)
        // Note: Removing infer_schema_length to maintain backward compatibility
        // .with_infer_schema_length(Some(1000))  // Limit schema inference for speed
        .map_parse_options(|parse_opts| {
            parse_opts.with_separator(sep_byte)
            // Note: Disabling try_parse_dates to maintain backward compatibility
            // .with_try_parse_dates(true)
        });

    // For large files, use additional optimizations
    if let Some(size) = file_size {
        if size > LARGE_FILE_THRESHOLD {
            options = options.with_low_memory(true); // Force low memory for large files
        }
    }

    options
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
    pub fn get_dataframe(
        &self,
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
    ) -> LazyFrame {
        let _ = separator_byte(separator);
        if self.paths.len() == 1 {
            let path = &self.paths[0];
            self.read_csv_file(path, separator, low_memory, no_headers, chunk_size)
        } else {
            self.concat_csv_files(separator, low_memory, no_headers, chunk_size)
        }
    }
    fn read_csv_file(
        &self,
        path: &Path,
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
    ) -> LazyFrame {
        LogController::debug(&format!("Reading CSV file: {}", path.display()));
        let has_header = !no_headers;
        // Check if file is gzipped based on extension
        let is_gzipped = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase() == "gz")
            .unwrap_or(false);
        if is_gzipped {
            LogController::debug(&format!("Reading gzipped file: {}", path.display()));
            self.read_gzipped_csv_file(path, separator, low_memory, has_header, chunk_size)
        } else {
            // Get file size for optimization
            let file_size = std::fs::metadata(path).ok().map(|m| m.len());

            // Use optimized CSV options
            let csv_options =
                get_optimized_csv_options(separator, has_header, low_memory, chunk_size, file_size);

            LogController::debug(&format!(
                "Reading CSV file: {} (size: {}MB)",
                path.display(),
                file_size.map(|s| s / 1024 / 1024).unwrap_or(0)
            ));

            let reader = LazyCsvReader::new(path)
                .with_separator(csv_options.parse_options.separator)
                .with_has_header(csv_options.has_header)
                .with_low_memory(csv_options.low_memory)
                .with_chunk_size(csv_options.chunk_size)
                // Note: Removing infer_schema_length for compatibility
                // .with_infer_schema_length(csv_options.infer_schema_length)
                .finish();

            match reader {
                Ok(df) => df,
                Err(e) => {
                    eprintln!("Error with Polars CSV reader for file {}: {}. Please check the file format and separator.", path.display(), e);
                    std::process::exit(1);
                }
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
    ) -> LazyFrame {
        use flate2::read::GzDecoder;
        use std::io::{BufReader, BufWriter, Read};

        let source = match File::open(path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("Error opening gzipped file {}: {}", path.display(), e);
                std::process::exit(1);
            }
        };

        let temp_path = create_gzip_spool_path(path);
        LogController::debug(&format!(
            "Spooling gzip payload to temporary file: {}",
            temp_path.display()
        ));

        let temp_file = match File::create(&temp_path) {
            Ok(file) => file,
            Err(e) => {
                eprintln!(
                    "Error creating temporary spool file for {}: {}",
                    path.display(),
                    e
                );
                std::process::exit(1);
            }
        };

        let mut gz_decoder = GzDecoder::new(BufReader::new(source));
        let mut spool_writer = BufWriter::new(temp_file);
        let mut total_written = 0usize;
        let mut buffer = vec![0u8; GZIP_BUFFER_SIZE];

        loop {
            match gz_decoder.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = spool_writer.write_all(&buffer[..n]) {
                        let _ = remove_file(&temp_path);
                        eprintln!(
                            "Error writing temporary spool file for {}: {}",
                            path.display(),
                            e
                        );
                        std::process::exit(1);
                    }
                    total_written += n;
                }
                Err(e) => {
                    let _ = remove_file(&temp_path);
                    eprintln!("Error decompressing gzipped file {}: {}", path.display(), e);
                    std::process::exit(1);
                }
            }
        }

        if let Err(e) = spool_writer.flush() {
            let _ = remove_file(&temp_path);
            eprintln!(
                "Error flushing temporary spool file for {}: {}",
                path.display(),
                e
            );
            std::process::exit(1);
        }

        LogController::debug(&format!(
            "Decompressed {}MB from {} to spool file",
            total_written / (1024 * 1024),
            path.display()
        ));

        let temp_reader = match File::open(&temp_path) {
            Ok(file) => file,
            Err(e) => {
                let _ = remove_file(&temp_path);
                eprintln!(
                    "Error reopening temporary spool file for {}: {}",
                    path.display(),
                    e
                );
                std::process::exit(1);
            }
        };

        let csv_options = get_optimized_csv_options(
            separator,
            has_header,
            low_memory,
            chunk_size,
            Some(total_written as u64),
        );

        let reader = csv_options.into_reader_with_file_handle(BufReader::new(temp_reader));
        let result = reader.finish();
        let _ = remove_file(&temp_path);

        match result {
            Ok(df) => df.lazy(),
            Err(e) => {
                eprintln!(
                    "Error parsing gzipped CSV file {}: {}. Please check the file format and separator.",
                    path.display(),
                    e
                );
                std::process::exit(1);
            }
        }
    }
    fn concat_csv_files(
        &self,
        separator: &str,
        low_memory: bool,
        no_headers: bool,
        chunk_size: Option<usize>,
    ) -> LazyFrame {
        LogController::debug(&format!("Reading {} CSV files", self.paths.len()));

        // Use parallel processing for multiple files if threshold is met
        let dataframes = if self.paths.len() >= PARALLEL_THRESHOLD {
            LogController::debug("Using parallel file reading for better performance");
            self.paths
                .par_iter() // Enabled parallel processing
                .map(|path| self.read_csv_file(path, separator, low_memory, no_headers, chunk_size))
                .collect::<Vec<_>>()
        } else {
            // Sequential for small number of files
            self.paths
                .iter()
                .map(|path| self.read_csv_file(path, separator, low_memory, no_headers, chunk_size))
                .collect::<Vec<_>>()
        };

        concat(
            dataframes,
            UnionArgs {
                parallel: true,
                rechunk: true,
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| {
            eprintln!("Error concatenating CSV files: {e}");
            std::process::exit(1);
        })
    }
}

fn create_gzip_spool_path(source_path: &Path) -> PathBuf {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("qsv-gzip");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("qsv-gzip-spool-{stem}-{timestamp}.csv"))
}
