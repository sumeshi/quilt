use crate::controllers::batch::calculate_batch_size;
use crate::controllers::log::LogController;
use polars::prelude::*;
use std::io::{BufWriter, Write};

pub fn show(df: &LazyFrame) {
    LogController::debug("Showing DataFrame with traditional method");
    show_traditional(df);
}

pub fn show_with_batch_size(df: &LazyFrame, batch_size_bytes: usize) {
    LogController::debug(&format!(
        "Showing DataFrame with streaming support (batch size: {}MB)",
        batch_size_bytes / 1_048_576
    ));

    let stdout = std::io::stdout();
    let writer = BufWriter::new(stdout);

    if let Err(e) = show_streaming_internal(df, writer, batch_size_bytes) {
        LogController::debug(&format!("Streaming show failed: {e}"));
        LogController::debug("Falling back to traditional show method");
        show_traditional(df);
    }
}

/// Memory-efficient streaming show for large datasets
fn show_streaming_internal<W: Write>(
    df: &LazyFrame,
    mut writer: W,
    batch_size_bytes: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let batch_size_rows = calculate_batch_size(df, batch_size_bytes)?;
    LogController::debug(&format!(
        "Using batch size: {} rows (~{}MB)",
        batch_size_rows,
        batch_size_bytes / 1_048_576
    ));

    let mut current_offset = 0;
    let mut total_rows = 0;
    let mut header_written = false;

    loop {
        let mut batch_df = df
            .clone()
            .slice(current_offset as i64, batch_size_rows as u32)
            .collect()?;

        if batch_df.height() == 0 {
            break; // No more data
        }

        LogController::debug(&format!(
            "Showing batch: rows {}-{}",
            current_offset,
            current_offset + batch_df.height()
        ));

        // Estimate buffer size: ~100 bytes per row on average for CSV output
        let estimated_buffer_size = batch_df.height() * 100;
        let mut buf = Vec::with_capacity(estimated_buffer_size);
        CsvWriter::new(&mut buf)
            .include_header(!header_written)
            .with_separator(b',')
            .finish(&mut batch_df)?;

        writer.write_all(&buf)?;

        if !header_written {
            header_written = true;
        }

        let processed_rows = batch_df.height();
        total_rows += processed_rows;
        current_offset += processed_rows;

        if processed_rows < batch_size_rows {
            break; // Last batch
        }
    }

    writer.flush()?;
    LogController::info(&format!("Successfully showed {total_rows} rows"));
    Ok(())
}

/// Traditional show method (fallback)
fn show_traditional(df: &LazyFrame) {
    LogController::debug("Using traditional show method");

    match df.clone().collect() {
        Ok(mut df_collected) => {
            // By default, Polars prints a table to stdout
            // To emulate the previous CSV output, we use CsvWriter
            // Estimate buffer size based on data size
            let estimated_size = df_collected.height() * 100; // ~100 bytes per row estimate
            let mut buf = Vec::with_capacity(estimated_size);
            if CsvWriter::new(&mut buf)
                .include_header(true)
                .with_separator(b',')
                .finish(&mut df_collected)
                .is_ok()
            {
                // The `show` command in many tools prints to stdout.
                // We will write the buffer to stdout.
                if let Ok(s) = String::from_utf8(buf) {
                    print!("{s}");
                    LogController::debug("Successfully showed DataFrame as CSV to stdout");
                } else {
                    eprintln!("Error: Could not convert buffer to UTF-8 string");
                }
            } else {
                eprintln!("Error writing to buffer");
            }
        }
        Err(e) => {
            eprintln!("Error: Failed to collect DataFrame: {e}");
            eprintln!("Tip: For very large files, the streaming approach should have worked.");
            eprintln!("      Try using 'head <n>' to limit the number of rows.");
        }
    }
}
