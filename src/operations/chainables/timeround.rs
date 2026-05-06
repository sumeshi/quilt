use chrono::{DateTime, NaiveDateTime};
use polars::prelude::*;

pub fn timeround(
    df: &LazyFrame,
    colname: &str,
    unit: &str,
    output_colname: Option<&str>,
) -> LazyFrame {
    // Convert unit shorthand to polars duration format and determine output format
    let (duration, format) = match unit {
        "y" | "year" => ("1y", "%Y"),
        "M" | "month" => ("1mo", "%Y-%m"),
        "d" | "day" => ("1d", "%Y-%m-%d"),
        "h" | "hour" => ("1h", "%Y-%m-%d %H"),
        "m" | "minute" => ("1m", "%Y-%m-%d %H:%M"),
        "s" | "second" => ("1s", "%Y-%m-%d %H:%M:%S"),
        _ => {
            eprintln!("Error: Invalid time unit '{unit}'. Use: y/year, M/month, d/day, h/hour, m/minute, s/second");
            std::process::exit(1);
        }
    };
    let output_col = output_colname
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{colname}_rounded"));
    df.clone().with_columns([col(colname)
        .cast(DataType::String)
        .map(
            |s_col: Column| {
                let ca = s_col.str()?;
                let converted: Vec<Option<String>> = ca
                    .into_iter()
                    .map(|opt_time_str| match opt_time_str {
                        Some(time_str) if time_str.trim().is_empty() => None,
                        Some(time_str) => match parse_datetime_string_canonical(time_str) {
                            Some(parsed) => Some(parsed),
                            None => {
                                eprintln!("Error: Could not parse datetime '{time_str}' for timeround operation");
                                std::process::exit(1);
                            }
                        },
                        None => None,
                    })
                    .collect();
                Ok(Some(
                    StringChunked::from_iter_options(
                        "canonical_datetime".into(),
                        converted.into_iter(),
                    )
                    .into_series()
                    .into(),
                ))
            },
            GetOutput::from_type(DataType::String),
        )
        .str()
        .to_datetime(
            Some(TimeUnit::Microseconds),
            None,
            StrptimeOptions {
                format: None, // Auto-detect format
                ..Default::default()
            },
            lit("raise"),
        )
        .dt()
        .truncate(lit(duration))
        .dt()
        .to_string(format)
        .alias(&output_col)])
}

fn parse_datetime_string_canonical(time_str: &str) -> Option<String> {
    let time_str = time_str.trim();
    if time_str.is_empty() {
        return None;
    }

    let formats = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%d/%b/%Y:%H:%M:%S",
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

    let offset_formats = [
        "%Y-%m-%d %H:%M:%S%.f %:z",
        "%Y-%m-%d %H:%M:%S %:z",
        "%Y-%m-%d %H:%M:%S%.f%:z",
        "%Y-%m-%d %H:%M:%S%:z",
    ];
    for format in &offset_formats {
        if let Ok(dt) = DateTime::parse_from_str(time_str, format) {
            return Some(dt.naive_local().format("%Y-%m-%d %H:%M:%S%.6f").to_string());
        }
    }

    if let Ok(timestamp) = time_str.parse::<i64>() {
        if let Some(dt) = DateTime::from_timestamp(timestamp, 0) {
            return Some(dt.naive_utc().format("%Y-%m-%d %H:%M:%S%.6f").to_string());
        }
    }

    None
}
