use crate::controllers::batch::{
    calculate_batch_size_from_frame, write_dataframe_csv, write_dataframe_csv_in_batches,
};
use crate::controllers::log::LogController;
use chrono::Local;
use polars::prelude::*;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

pub fn dump(df: &LazyFrame, output_path_opt: Option<&str>, separator: char) {
    let output_path_str = output_path_opt.map(|p| p.to_string()).unwrap_or_else(|| {
        let now = Local::now();
        format!("dump_{}.csv", now.format("%Y%m%d_%H%M%S"))
    });

    if output_path_str == "-" {
        eprintln!("Error: The 'dump' command requires a file path. To print to stdout, use the 'show' command instead.");
        std::process::exit(1);
    }
    LogController::debug(&format!("Dumping DataFrame to CSV: {output_path_str}"));
    dump_traditional(df, &output_path_str, separator);
}

pub fn dump_with_batch_size(
    df: &LazyFrame,
    output_path_opt: Option<&str>,
    separator: char,
    batch_size_bytes: usize,
) {
    let output_path_str = output_path_opt.map(|p| p.to_string()).unwrap_or_else(|| {
        let now = Local::now();
        format!("dump_{}.csv", now.format("%Y%m%d_%H%M%S"))
    });

    if output_path_str == "-" {
        eprintln!("Error: The 'dump' command requires a file path. To print to stdout, use the 'show' command instead.");
        std::process::exit(1);
    }

    LogController::debug(&format!(
        "Dumping DataFrame with batch size: {}MB",
        batch_size_bytes / 1_048_576
    ));

    LogController::debug(&format!("Dumping DataFrame to CSV: {output_path_str}"));
    let output_path = PathBuf::from(&output_path_str);

    match File::create(&output_path) {
        Ok(file) => {
            let writer = BufWriter::new(file);
            if let Err(e) = dump_streaming_internal(df, writer, separator, batch_size_bytes) {
                LogController::debug(&format!("Streaming dump failed: {e}"));
                LogController::info("Falling back to traditional dump method");
                // Fallback needs to be handled carefully as the file might be partially written
                // For simplicity, we let dump_traditional overwrite the file.
                dump_traditional(df, &output_path_str, separator);
            } else {
                LogController::info(&format!(
                    "Successfully dumped large dataset to: {}",
                    output_path.display()
                ));
            }
        }
        Err(e) => {
            eprintln!(
                "Error: Failed to create file '{}': {}",
                output_path.display(),
                e
            );
            std::process::exit(1);
        }
    }
}

/// Stream dump for large datasets to any writer (file or stdout)
fn dump_streaming_internal<W: Write>(
    df: &LazyFrame,
    mut writer: W,
    separator: char,
    batch_size_bytes: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let collected = df.clone().collect()?;
    let batch_size_rows = calculate_batch_size_from_frame(&collected, batch_size_bytes);
    LogController::debug(&format!(
        "Using batch size: {} rows (~{}MB)",
        batch_size_rows,
        batch_size_bytes / 1_048_576
    ));
    let total_rows =
        write_dataframe_csv_in_batches(&collected, &mut writer, separator as u8, batch_size_rows)?;
    LogController::info(&format!("Successfully streamed {total_rows} rows"));
    Ok(())
}

/// Traditional dump method (fallback for simple cases or stdout)
fn dump_traditional(df: &LazyFrame, output_path_str: &str, separator: char) {
    LogController::debug("Using traditional dump method");

    let mut df_collected = match df.clone().collect() {
        Ok(df) => df,
        Err(e) => {
            eprintln!("Error: Failed to collect DataFrame for dumping: {e}");
            eprintln!("Tip: For very large files, the streaming approach should have worked.");
            eprintln!("      Try reducing data size with 'head', 'select', or other filters.");
            std::process::exit(1);
        }
    };

    let output_path = PathBuf::from(output_path_str);
    let result = match File::create(&output_path) {
        Ok(file) => {
            let writer = BufWriter::new(file);
            write_dataframe_csv(&mut df_collected, writer, separator as u8, true)
        }
        Err(e) => {
            eprintln!(
                "Error: Failed to create file '{}': {}",
                output_path.display(),
                e
            );
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("Error writing CSV to '{output_path_str}': {e}");
        std::process::exit(1);
    } else {
        LogController::info(&format!("Successfully dumped to: {output_path_str}"));
    }
}
