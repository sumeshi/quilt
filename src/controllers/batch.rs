use crate::controllers::log::LogController;
use polars::prelude::*;
use std::io::Write;

pub const MIN_BATCH_SIZE_ROWS: usize = 1000; // Minimum 1K rows per batch
pub const MAX_BATCH_SIZE_ROWS: usize = 1_000_000; // Maximum 1M rows per batch

/// Calculate optimal batch size based on memory target and data characteristics
pub fn calculate_batch_size_from_frame(df: &DataFrame, target_bytes: usize) -> usize {
    let sample_size = df.height().min(100);
    if sample_size == 0 {
        return MIN_BATCH_SIZE_ROWS;
    }

    let sample = df.head(Some(sample_size));
    let estimated_bytes_per_row = estimate_row_size(&sample).unwrap_or(0);

    if estimated_bytes_per_row == 0 {
        return MAX_BATCH_SIZE_ROWS;
    }

    let calculated_batch_size = target_bytes / estimated_bytes_per_row;
    let batch_size = calculated_batch_size.clamp(MIN_BATCH_SIZE_ROWS, MAX_BATCH_SIZE_ROWS);

    LogController::debug(&format!(
        "Estimated {estimated_bytes_per_row} bytes per row from collected DataFrame, using batch size: {batch_size} rows"
    ));

    batch_size
}

/// Estimate the size of a row in bytes
pub fn estimate_row_size(sample: &DataFrame) -> Result<usize, Box<dyn std::error::Error>> {
    let mut total_size = 0;
    let height = sample.height();

    if height == 0 {
        return Ok(0);
    }

    for column in sample.get_columns() {
        let column_size = match column.dtype() {
            DataType::String => {
                // For strings, estimate based on actual string lengths
                if let Ok(str_column) = column.str() {
                    str_column
                        .iter()
                        .map(|opt_str| opt_str.map_or(0, |s| s.len()))
                        .sum::<usize>()
                } else {
                    0 // If casting to string fails, assume 0 size for this column
                }
            }
            DataType::Int8 | DataType::UInt8 => height,
            DataType::Int16 | DataType::UInt16 => height * 2,
            DataType::Int32 | DataType::UInt32 | DataType::Float32 => height * 4,
            DataType::Int64 | DataType::UInt64 | DataType::Float64 => height * 8,
            DataType::Date => height * 4,
            DataType::Datetime(_, _) => height * 8,
            DataType::Time => height * 8,
            DataType::Boolean => height,
            _ => height * 8, // Default assumption for other types
        };
        total_size += column_size;
    }

    // Add some overhead for DataFrame structure
    total_size += height * 8; // Row overhead

    Ok(total_size / height) // Average bytes per row
}

pub fn write_dataframe_csv<W: Write>(
    df: &mut DataFrame,
    mut writer: W,
    separator: u8,
    include_header: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    CsvWriter::new(&mut writer)
        .include_header(include_header)
        .with_separator(separator)
        .finish(df)?;
    writer.flush()?;
    Ok(())
}

pub fn write_dataframe_csv_in_batches<W: Write>(
    df: &DataFrame,
    mut writer: W,
    separator: u8,
    batch_size_rows: usize,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut current_offset = 0usize;
    let mut total_rows = 0usize;
    let mut header_written = false;

    loop {
        let mut batch_df = df.slice(current_offset as i64, batch_size_rows);
        if batch_df.height() == 0 {
            break;
        }

        CsvWriter::new(&mut writer)
            .include_header(!header_written)
            .with_separator(separator)
            .finish(&mut batch_df)?;

        header_written = true;
        let processed_rows = batch_df.height();
        total_rows += processed_rows;
        current_offset += processed_rows;

        if processed_rows < batch_size_rows {
            break;
        }
    }

    writer.flush()?;
    Ok(total_rows)
}
