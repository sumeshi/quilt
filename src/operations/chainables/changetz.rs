use crate::controllers::log::LogController;
use chrono::{Local, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use dtparse::parse as dtparse_parse;
use once_cell::sync::Lazy;
use polars::prelude::*;
use regex::Regex;

static FUZZY_DATETIME_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    let patterns = [
        // Month name with day and year: "January 15th, 2023 at 2:30 PM"
        r"(?i)(?:on\s+)?(?:january|february|march|april|may|june|july|august|september|october|november|december)\s+\d{1,2}(?:st|nd|rd|th)?,?\s+\d{4}(?:\s+at\s+)?\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
        // Short month: "Jan 15, 2023 2:30 PM"
        r"(?i)(?:on\s+)?(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\s+\d{1,2},?\s+\d{4}\s+\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
        // ISO-like in text: "2023-01-15 14:30:00"
        r"\d{4}-\d{1,2}-\d{1,2}\s+\d{1,2}:\d{2}(?::\d{2})?",
        // US date format: "1/15/2023 2:30 PM"
        r"\d{1,2}/\d{1,2}/\d{4}\s+\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
        // Day month year: "Friday Jan 13 2023 9:00 AM"
        r"(?i)(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday)\s+(?:jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)\s+\d{1,2}\s+\d{4}\s+\d{1,2}:\d{2}(?::\d{2})?\s*(?:AM|PM)?",
    ];
    patterns.iter().map(|p| Regex::new(p).unwrap()).collect()
});

/// Parse datetime string with comprehensive format support including fuzzy parsing
fn parse_datetime_auto(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // First try dtparse for maximum flexibility (similar to Python dateutil.parser)
    match dtparse_parse(s) {
        Ok((dt, _)) => {
            LogController::debug(&format!("Successfully parsed '{s}' using dtparse"));
            return Some(dt);
        }
        Err(e) => {
            LogController::debug(&format!("dtparse failed for '{s}': {e}"));
        }
    }
    // Try fuzzy parsing with regex extraction
    if let Some(extracted) = extract_datetime_fuzzy(s) {
        LogController::debug(&format!(
            "Extracted datetime '{extracted}' from fuzzy text '{s}'"
        ));
        if let Some(dt) = parse_extracted_datetime(&extracted) {
            return Some(dt);
        }
    }
    // Fallback to manual format detection for edge cases
    let formats = [
        // ISO 8601 formats
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d",
        // US formats
        "%m/%d/%Y %H:%M:%S%.f",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y",
        // EU formats
        "%d/%m/%Y %H:%M:%S%.f",
        "%d/%m/%Y %H:%M:%S",
        "%d/%m/%Y",
        // Alternative separators
        "%Y/%m/%d %H:%M:%S%.f",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d",
        // Month name formats (common in logs)
        "%d %b %Y %H:%M:%S", // 15 Jan 2023 14:30:25
        "%b %d %Y %H:%M:%S", // Jan 15 2023 14:30:25
        "%d %B %Y %H:%M:%S", // 15 January 2023 14:30:25
        "%B %d %Y %H:%M:%S", // January 15 2023 14:30:25
        "%d-%b-%Y %H:%M:%S", // 15-Jan-2023 14:30:25
        "%d %b %Y",          // 15 Jan 2023
        "%b %d %Y",          // Jan 15 2023
        // Log formats
        "%a %b %d %H:%M:%S %Y",  // Mon Jan 15 14:30:25 2023
        "%a, %d %b %Y %H:%M:%S", // Mon, 15 Jan 2023 14:30:25
        // Unix timestamp formats
        "%s",    // 1674659425
        "%s%.f", // 1674659425.123
        // Windows Event Log formats
        "%m/%d/%Y %I:%M:%S %p", // 1/15/2023 2:30:25 PM
        "%Y-%m-%d %I:%M:%S %p", // 2023-01-15 2:30:25 PM
    ];
    for fmt in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            LogController::debug(&format!("Parsed '{s}' with format '{fmt}'"));
            return Some(dt);
        }
    }
    LogController::warn(&format!("Failed to parse datetime: '{s}'"));
    None
}
/// Extract datetime patterns from fuzzy text using regex
fn extract_datetime_fuzzy(text: &str) -> Option<String> {
    for re in FUZZY_DATETIME_PATTERNS.iter() {
        if let Some(captures) = re.find(text) {
            return Some(captures.as_str().to_string());
        }
    }
    None
}
/// Parse extracted datetime string using multiple formats
fn parse_extracted_datetime(extracted: &str) -> Option<NaiveDateTime> {
    // Clean up the extracted string
    let cleaned = extracted
        .replace(" at ", " ")
        .replace("st,", ",")
        .replace("nd,", ",")
        .replace("rd,", ",")
        .replace("th,", ",")
        .replace("st ", " ")
        .replace("nd ", " ")
        .replace("rd ", " ")
        .replace("th ", " ");
    // Try dtparse again on the cleaned extracted text
    if let Ok((dt, _)) = dtparse_parse(&cleaned) {
        LogController::debug(&format!("Parsed extracted '{cleaned}' using dtparse"));
        return Some(dt);
    }
    // Formats specifically for extracted patterns
    let formats = [
        "%B %d, %Y %I:%M:%S %p",   // January 15, 2023 2:30:00 PM
        "%B %d, %Y %I:%M %p",      // January 15, 2023 2:30 PM
        "%b %d, %Y %I:%M:%S %p",   // Jan 15, 2023 2:30:00 PM
        "%b %d, %Y %I:%M %p",      // Jan 15, 2023 2:30 PM
        "%Y-%m-%d %H:%M:%S",       // 2023-01-15 14:30:00
        "%Y-%m-%d %H:%M",          // 2023-01-15 14:30
        "%m/%d/%Y %I:%M:%S %p",    // 1/15/2023 2:30:00 PM
        "%m/%d/%Y %I:%M %p",       // 1/15/2023 2:30 PM
        "%A %b %d %Y %I:%M:%S %p", // Friday Jan 13 2023 9:00:00 AM
        "%A %b %d %Y %I:%M %p",    // Friday Jan 13 2023 9:00 AM
    ];
    for fmt in &formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&cleaned, fmt) {
            LogController::debug(&format!("Parsed extracted '{cleaned}' with format '{fmt}'"));
            return Some(dt);
        }
    }
    None
}
/// Convert timezone with proper error handling
fn convert_timezone(
    datetime_str: &str,
    from_tz: &str,
    to_tz: &str,
    input_format: &str,
    output_format: &str,
    ambiguous: &str,
) -> Option<String> {
    if datetime_str.trim().is_empty() {
        return Some(String::new());
    }
    // Parse datetime
    let naive_dt = if input_format == "auto" {
        parse_datetime_auto(datetime_str)?
    } else {
        NaiveDateTime::parse_from_str(datetime_str, input_format).ok()?
    };
    // Handle source timezone
    let utc_dt = if from_tz.to_lowercase() == "local" {
        match Local.from_local_datetime(&naive_dt) {
            chrono::LocalResult::Single(dt) => dt.with_timezone(&Utc),
            chrono::LocalResult::Ambiguous(dt1, dt2) => {
                if ambiguous == "earliest" { dt1 } else { dt2 }.with_timezone(&Utc)
            }
            chrono::LocalResult::None => return None,
        }
    } else {
        let from_tz_parsed: Tz = from_tz.parse().ok()?;
        match from_tz_parsed.from_local_datetime(&naive_dt) {
            chrono::LocalResult::Single(dt) => dt.with_timezone(&Utc),
            chrono::LocalResult::Ambiguous(dt1, dt2) => {
                if ambiguous == "earliest" { dt1 } else { dt2 }.with_timezone(&Utc)
            }
            chrono::LocalResult::None => return None,
        }
    };
    // Convert to target timezone
    let to_tz_parsed: Tz = to_tz.parse().ok()?;
    let target_dt = utc_dt.with_timezone(&to_tz_parsed);
    // Format output
    let format = if output_format == "auto" {
        "%Y-%m-%dT%H:%M:%S%.6f%:z" // ISO8601 with microsecond precision
    } else {
        output_format
    };
    Some(target_dt.format(format).to_string())
}
pub fn changetz(
    df: &LazyFrame,
    colname: &str,
    from_tz: &str,
    to_tz: &str,
    input_format: &str,
    output_format: &str,
    ambiguous_time: &str,
) -> LazyFrame {
    // Validate column exists by checking the schema
    let schema = match df.clone().collect_schema() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error getting schema for changetz operation: {e}");
            std::process::exit(1);
        }
    };
    if schema.get(colname).is_none() {
        eprintln!("Error: Column '{colname}' not found for changetz operation");
        std::process::exit(1);
    }

    // Validate timezones
    if from_tz.to_lowercase() != "local" && from_tz.parse::<Tz>().is_err() {
        eprintln!("Error: Invalid source timezone '{from_tz}'");
        std::process::exit(1);
    }
    if to_tz.parse::<Tz>().is_err() {
        eprintln!("Error: Invalid target timezone '{to_tz}'");
        std::process::exit(1);
    }
    LogController::debug(&format!(
        "Converting timezone for column '{colname}': {from_tz} → {to_tz} (format: {input_format} → {output_format}, ambiguous: {ambiguous_time})"
    ));
    // Clone parameters for closure
    let from_tz = from_tz.to_string();
    let to_tz = to_tz.to_string();
    let input_format = input_format.to_string();
    let output_format = output_format.to_string();
    let ambiguous_time = ambiguous_time.to_string();
    // Apply timezone conversion
    df.clone().with_column(
        col(colname)
            .map(
                move |s| {
                    let ca = s.str()?;
                    let converted: StringChunked = ca
                        .into_iter()
                        .map(|opt_str| {
                            opt_str.and_then(|datetime_str| {
                                convert_timezone(
                                    datetime_str,
                                    &from_tz,
                                    &to_tz,
                                    &input_format,
                                    &output_format,
                                    &ambiguous_time,
                                )
                            })
                        })
                        .collect();
                    Ok(Some(converted.into_series().into()))
                },
                GetOutput::from_type(DataType::String),
            )
            .alias(colname),
    )
}
