use crate::controllers::log::LogController;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use polars::prelude::*;
use regex::Regex;

const TIMELINE_OUTPUT_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub fn timeline(
    df: &LazyFrame,
    time_column: &str,
    interval: &str,
    agg_type: &str,
    agg_column: Option<&str>,
) -> LazyFrame {
    let schema = match df.clone().collect_schema() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error getting schema for timeline operation: {e}");
            std::process::exit(1);
        }
    };

    if !schema.iter_names().any(|s| s == time_column) {
        eprintln!(
            "Error: Time column '{time_column}' not found in DataFrame for timeline operation"
        );
        std::process::exit(1);
    }

    // Parse interval (e.g., "1h", "5m", "30s")
    let interval_duration = parse_interval(interval);
    if interval_duration.is_none() {
        eprintln!("Error: Invalid interval format '{interval}'. Use format like '1h', '5m', '30s'");
        std::process::exit(1);
    }
    let interval_duration = interval_duration.unwrap();
    if interval_duration.num_seconds() <= 0 {
        eprintln!("Error: Interval '{interval}' is sub-second; timeline bucketing requires at least 1 second (e.g. '1s', '5m', '1h')");
        std::process::exit(1);
    }

    LogController::debug(&format!(
        "Creating timeline: column={time_column}, interval={interval}, aggregation={agg_type}"
    ));

    let bucket_column_name = format!("timeline_{interval}");
    let timeline_expr = if can_use_polars_timeline_fast_path(df, time_column) {
        LogController::debug("Using Polars-native timeline fast path");
        fast_timeline_bucket_expr(time_column, interval, &bucket_column_name)
    } else {
        LogController::debug("Using Rust timeline fallback path");
        rust_timeline_bucket_expr(time_column, interval_duration, &bucket_column_name)
    };

    let mut agg_exprs = Vec::with_capacity(if agg_column.is_some() { 2 } else { 1 });
    agg_exprs.push(len().alias("count"));

    // Add aggregation column if specified
    if let Some(agg_col) = agg_column {
        if !schema.iter_names().any(|s| s == agg_col) {
            eprintln!("Error: Aggregation column '{agg_col}' not found in DataFrame");
            std::process::exit(1);
        }
        let agg_expr = match agg_type {
            "sum" => col(agg_col)
                .cast(DataType::Float64)
                .sum()
                .alias(format!("sum_{agg_col}")),
            "avg" => col(agg_col)
                .cast(DataType::Float64)
                .mean()
                .alias(format!("avg_{agg_col}")),
            "min" => col(agg_col)
                .cast(DataType::Float64)
                .min()
                .alias(format!("min_{agg_col}")),
            "max" => col(agg_col)
                .cast(DataType::Float64)
                .max()
                .alias(format!("max_{agg_col}")),
            "std" => col(agg_col)
                .cast(DataType::Float64)
                .std(1)
                .alias(format!("std_{agg_col}")),
            _ => {
                eprintln!(
                    "Error: Unsupported aggregation type '{agg_type}'. Use: sum, avg, min, max, std"
                );
                std::process::exit(1);
            }
        };
        agg_exprs.push(agg_expr);
    }

    df.clone()
        .with_column(timeline_expr)
        .group_by([col(&bucket_column_name)])
        .agg(agg_exprs)
        .sort([&bucket_column_name], SortMultipleOptions::default())
}

fn can_use_polars_timeline_fast_path(df: &LazyFrame, time_column: &str) -> bool {
    let sample = match df.clone().select([col(time_column)]).limit(100).collect() {
        Ok(df) => df,
        Err(_) => return false,
    };

    let series = match sample.column(time_column) {
        Ok(series) => series,
        Err(_) => return false,
    };

    let Ok(string_values) = series.str() else {
        return false;
    };

    let iso_like = Regex::new(
        r"^\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?(?:Z|[+-]\d{2}:\d{2})?)?$",
    )
    .expect("valid ISO-like regex");

    let mut saw_value = false;
    for value in string_values.into_iter().flatten() {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        saw_value = true;
        if !iso_like.is_match(trimmed) {
            return false;
        }
    }

    saw_value
}

fn fast_timeline_bucket_expr(time_column: &str, interval: &str, bucket_column_name: &str) -> Expr {
    col(time_column)
        .cast(DataType::String)
        .str()
        .to_datetime(
            Some(TimeUnit::Microseconds),
            None,
            StrptimeOptions {
                format: None,
                ..Default::default()
            },
            lit("raise"),
        )
        .dt()
        .truncate(lit(interval))
        .dt()
        .to_string(TIMELINE_OUTPUT_FORMAT)
        .alias(bucket_column_name)
}

fn rust_timeline_bucket_expr(
    time_column: &str,
    interval_duration: Duration,
    bucket_column_name: &str,
) -> Expr {
    col(time_column)
        .cast(DataType::String)
        .map(
            move |s_col: Column| {
                let ca = s_col.str()?;
                let mut timeline_buckets: Vec<Option<String>> = Vec::with_capacity(ca.len());
                for opt_time_str in ca.into_iter() {
                    if let Some(time_str) = opt_time_str {
                        if let Some(bucket) = time_to_bucket(time_str, interval_duration) {
                            timeline_buckets.push(Some(bucket));
                        } else {
                            timeline_buckets.push(None);
                        }
                    } else {
                        timeline_buckets.push(None);
                    }
                }
                Ok(Some(
                    Series::new("timeline_bucket".into(), timeline_buckets).into(),
                ))
            },
            GetOutput::from_type(DataType::String),
        )
        .alias(bucket_column_name)
}
fn parse_interval(interval: &str) -> Option<Duration> {
    if interval.is_empty() {
        return None;
    }
    let (num_str, unit) = if let Some(stripped) = interval.strip_suffix("ms") {
        (stripped, "ms")
    } else {
        (
            &interval[..interval.len() - 1],
            &interval[interval.len() - 1..],
        )
    };
    let num: i64 = num_str.parse().ok()?;
    match unit {
        "s" => Some(Duration::seconds(num)),
        "m" => Some(Duration::minutes(num)),
        "h" => Some(Duration::hours(num)),
        "d" => Some(Duration::days(num)),
        "ms" => Some(Duration::milliseconds(num)),
        _ => None,
    }
}
fn time_to_bucket(time_str: &str, interval: Duration) -> Option<String> {
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
    let mut parsed_time: Option<NaiveDateTime> = None;
    for format in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(time_str, format) {
            parsed_time = Some(dt);
            break;
        }
    }
    if parsed_time.is_none() {
        if let Ok(dt) = DateTime::parse_from_rfc3339(time_str) {
            parsed_time = Some(dt.naive_local());
        }
    }
    if parsed_time.is_none() {
        // Try parsing as timestamp
        if let Ok(timestamp) = time_str.parse::<i64>() {
            parsed_time = DateTime::from_timestamp(timestamp, 0).map(|dt| dt.naive_utc());
        }
    }
    let dt = parsed_time?;
    let dt_utc = DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc);
    // Round down to interval boundary
    let interval_seconds = interval.num_seconds();
    if interval_seconds <= 0 {
        return None;
    }
    let timestamp = dt_utc.timestamp();
    let bucket_timestamp = (timestamp / interval_seconds) * interval_seconds;
    let bucket_dt = DateTime::from_timestamp(bucket_timestamp, 0)?;
    Some(bucket_dt.format(TIMELINE_OUTPUT_FORMAT).to_string())
}
