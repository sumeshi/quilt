use crate::controllers::batch::{
    calculate_batch_size_from_frame, write_dataframe_csv, write_dataframe_csv_in_batches,
};
use crate::controllers::log::LogController;
use polars::prelude::*;
use std::io::{BufWriter, Write};

pub fn show(df: &LazyFrame) {
    LogController::debug("Showing DataFrame with direct writer output");
    show_direct(df);
}

pub fn render_csv(df: &LazyFrame) -> Result<String, String> {
    let mut df_collected = df
        .clone()
        .collect()
        .map_err(|e| format!("Failed to collect DataFrame: {e}"))?;
    let estimated_size = df_collected.height() * 100;
    let mut buf = Vec::with_capacity(estimated_size);
    CsvWriter::new(&mut buf)
        .include_header(true)
        .with_separator(b',')
        .finish(&mut df_collected)
        .map_err(|e| format!("Error writing CSV to buffer: {e}"))?;
    String::from_utf8(buf).map_err(|e| format!("Could not convert CSV buffer to UTF-8 string: {e}"))
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
        LogController::debug("Falling back to direct show method");
        show_direct(df);
    }
}

/// Memory-efficient streaming show for large datasets
fn show_streaming_internal<W: Write>(
    df: &LazyFrame,
    mut writer: W,
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
        write_dataframe_csv_in_batches(&collected, &mut writer, b',', batch_size_rows)?;
    LogController::info(&format!("Successfully showed {total_rows} rows"));
    Ok(())
}

fn show_direct(df: &LazyFrame) {
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout.lock());

    let mut df_collected = match df.clone().collect() {
        Ok(df) => df,
        Err(e) => {
            eprintln!("Error: Failed to collect DataFrame for show: {e}");
            eprintln!("Tip: Try reducing data size with 'head', 'select', or other filters.");
            return;
        }
    };

    if let Err(e) = write_dataframe_csv(&mut df_collected, &mut writer, b',', true) {
        eprintln!("Error: Failed to write CSV to stdout: {e}");
    } else {
        LogController::debug("Successfully showed DataFrame as CSV to stdout");
    }
}
