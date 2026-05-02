use crate::controllers::log::LogController;
use chrono::{DateTime, NaiveDateTime};
use polars::prelude::*;

pub fn timeslice(
    df: &LazyFrame,
    time_column: &str,
    start_time: Option<&str>,
    end_time: Option<&str>,
) -> LazyFrame {
    let schema = match df.clone().collect_schema() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error getting schema for timeslice operation: {e}");
            std::process::exit(1);
        }
    };

    if !schema.iter_names().any(|s| s == time_column) {
        eprintln!(
            "Error: Time column '{time_column}' not found in DataFrame for timeslice operation"
        );
        std::process::exit(1);
    }

    LogController::debug(&format!(
        "Creating timeslice: column={time_column}, start={start_time:?}, end={end_time:?}"
    ));

    // Start with the original dataframe
    let mut result_df = df.clone();

    let time_col_expr = col(time_column)
        .cast(DataType::String)
        .map(
            |s_col: Column| {
                let ca = s_col.str()?;
                let converted: Vec<Option<String>> = ca
                    .into_iter()
                    .map(|opt_time_str| opt_time_str.and_then(parse_datetime_string_canonical))
                    .collect();
                Ok(Some(
                    StringChunked::from_iter_options(
                        "_temp_datetime".into(),
                        converted.into_iter(),
                    )
                    .into_series()
                    .into(),
                ))
            },
            GetOutput::from_type(DataType::String),
        )
        .alias("_temp_datetime");

    // Add the converted datetime column temporarily
    result_df = result_df.with_columns([time_col_expr]);

    // Apply start time filter if provided
    if let Some(start) = start_time {
        LogController::debug(&format!("Applying start time filter: {start}"));

        // Parse start time to a canonical string for lexicographic comparison
        let start_datetime = match parse_datetime_string_canonical(start) {
            Some(dt) => dt,
            None => {
                eprintln!("Error: Could not parse start time '{start}'");
                std::process::exit(1);
            }
        };

        let start_filter = col("_temp_datetime").gt_eq(lit(start_datetime));
        result_df = result_df.filter(start_filter);
    }

    // Apply end time filter if provided
    if let Some(end) = end_time {
        LogController::debug(&format!("Applying end time filter: {end}"));

        // Parse end time to a canonical string for lexicographic comparison
        let end_datetime = match parse_datetime_string_canonical(end) {
            Some(dt) => dt,
            None => {
                eprintln!("Error: Could not parse end time '{end}'");
                std::process::exit(1);
            }
        };

        let end_filter = col("_temp_datetime").lt_eq(lit(end_datetime));
        result_df = result_df.filter(end_filter);
    }

    // Remove the temporary datetime column
    let original_columns: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();
    result_df.select([cols(original_columns)])
}

fn parse_datetime_string_canonical(time_str: &str) -> Option<String> {
    let time_str = time_str.trim();
    if time_str.is_empty() {
        return None;
    }

    // Try multiple datetime formats
    let formats = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%d/%b/%Y:%H:%M:%S", // Apache log format
        "%Y-%m-%d",
        "%H:%M:%S",
    ];

    for format in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(time_str, format) {
            return Some(dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string());
        }
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(time_str) {
        return Some(dt.naive_local().format("%Y-%m-%d %H:%M:%S%.6f").to_string());
    }

    // Try parsing as timestamp
    if let Ok(timestamp) = time_str.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp(timestamp, 0) {
            return Some(dt.naive_utc().format("%Y-%m-%d %H:%M:%S%.6f").to_string());
        }
    }

    None
}
