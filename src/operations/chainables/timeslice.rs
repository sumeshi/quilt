use crate::controllers::log::LogController;
use crate::controllers::nested::literal_col;
use crate::error::QuiltError;
use crate::operations::datetime::{
    localize, parse_datetime_detailed_with_diagnostics, parse_datetime_detailed_with_precision,
    validate_config_format, DateTimeConfig, ParserDiagnostics,
};
use polars::prelude::*;

pub fn timeslice(
    df: &LazyFrame,
    time_column: &str,
    start_time: Option<&str>,
    end_time: Option<&str>,
    datetime: &DateTimeConfig,
) -> Result<LazyFrame, QuiltError> {
    if let Some(format) = datetime.input_format.as_deref() {
        validate_config_format(format).map_err(QuiltError::usage)?;
    }
    let diagnostics = ParserDiagnostics::new();
    let schema = df.clone().collect_schema().map_err(|e| {
        QuiltError::schema(
            "timeslice",
            None::<String>,
            format!("Error getting schema: {e}"),
        )
    })?;

    let source = schema.get(time_column).ok_or_else(|| {
        QuiltError::schema(
            "timeslice",
            Some(time_column),
            format!(
                "Error: Time column '{time_column}' not found in DataFrame for timeslice operation"
            ),
        )
    })?;
    if matches!(source, DataType::Datetime(_, _)) && datetime.options_present {
        return Err(QuiltError::usage(
            "datetime parsing options apply only to string timeslice input",
        ));
    }
    LogController::debug("Creating timeslice");

    if let DataType::Datetime(unit, _) = source {
        let unit = *unit;
        let lower = start_time
            .map(|value| datetime_boundary_raw(value, datetime, &diagnostics, unit))
            .transpose()?;
        let upper = end_time
            .map(|value| datetime_boundary_raw(value, datetime, &diagnostics, unit))
            .transpose()?;
        let physical = literal_col(time_column).cast(DataType::Int64);
        let mut result = df.clone();
        if let Some(lower) = lower {
            result = result.filter(physical.clone().gt_eq(lit(lower)));
        }
        if let Some(upper) = upper {
            result = result.filter(physical.lt_eq(lit(upper)));
        }
        return Ok(result);
    }

    // Start with the original dataframe
    let mut result_df = df.clone();
    let mut temporary_column = "_qlt_timeslice".to_string();
    while schema.get(&temporary_column).is_some() {
        temporary_column.push('_');
    }
    let temporary_output_name = temporary_column.clone();
    let time_column_name = time_column.to_string();
    let time_col_expr = literal_col(time_column)
        .cast(DataType::String)
        .map(
            {
                let datetime = datetime.clone();
                let time_column_name = time_column_name.clone();
                let diagnostics = diagnostics.clone();
                move |s_col: Column| {
                    let ca = s_col.str()?;
                    // Chunk-local UDF output collection; the input LazyFrame is
                    // still pending until a finalizer evaluates it.
                    let converted: Vec<Option<String>> = ca
                        .iter()
                        .enumerate()
                        .map(|(row, opt_time_str)| {
                            opt_time_str
                                .map(|value| {
                                    parse_datetime_string_canonical(value, &datetime, &diagnostics)
                                        .map_err(|error| PolarsError::ComputeError(
                                            format!("timeslice column '{}' row {}: {} (value redacted)", time_column_name, row, error).into()
                                        ))
                                })
                                .transpose()
                                .map(|value| value.flatten())
                        })
                        .collect::<PolarsResult<Vec<_>>>()?;
                    Ok(
                        StringChunked::from_iter_options(
                            temporary_output_name.clone().into(),
                            converted.into_iter(),
                        )
                        .into_series()
                        .into(),
                    )
                }
            },
            |_, field| Ok(Field::new(field.name().clone(), DataType::String)),
        )
        .alias(&temporary_column);

    // Add the converted datetime column temporarily
    result_df = result_df.with_columns([time_col_expr]);

    // Apply start time filter if provided
    if let Some(start) = start_time {
        LogController::debug("Applying timeslice start filter");

        // Parse start time to a canonical string for lexicographic comparison
        let start_datetime = parse_datetime_string_canonical(start, datetime, &diagnostics)
            .map_err(|error| {
                QuiltError::conversion(
                    "timeslice",
                    None::<String>,
                    format!("start boundary could not be parsed: {error}"),
                )
            })?
            .ok_or_else(|| {
                QuiltError::conversion(
                    "timeslice",
                    None::<String>,
                    "Error: Could not parse start boundary",
                )
            })?;

        let start_filter = literal_col(&temporary_column).gt_eq(lit(start_datetime));
        result_df = result_df.filter(start_filter);
    }

    // Apply end time filter if provided
    if let Some(end) = end_time {
        LogController::debug("Applying timeslice end filter");

        // Parse end time to a canonical string for lexicographic comparison
        let end_datetime = parse_datetime_string_canonical(end, datetime, &diagnostics)
            .map_err(|error| {
                QuiltError::conversion(
                    "timeslice",
                    None::<String>,
                    format!("end boundary could not be parsed: {error}"),
                )
            })?
            .ok_or_else(|| {
                QuiltError::conversion(
                    "timeslice",
                    None::<String>,
                    "Error: Could not parse end boundary",
                )
            })?;

        let end_filter = literal_col(&temporary_column).lt_eq(lit(end_datetime));
        result_df = result_df.filter(end_filter);
    }

    // Remove the temporary datetime column
    let original_columns: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();
    Ok(result_df.select(
        original_columns
            .iter()
            .map(|name| literal_col(name))
            .collect::<Vec<_>>(),
    ))
}

fn datetime_boundary_raw(
    value: &str,
    config: &DateTimeConfig,
    diagnostics: &ParserDiagnostics,
    unit: TimeUnit,
) -> Result<i64, QuiltError> {
    let parsed = parse_datetime_detailed_with_precision(
        value,
        config,
        Some(diagnostics),
        unit == TimeUnit::Nanoseconds,
    )
    .map_err(|error| QuiltError::conversion("timeslice", None::<String>, error))?
    .ok_or_else(|| QuiltError::conversion("timeslice", None::<String>, "empty boundary"))?;
    let instant = if parsed.authoritative_offset {
        parsed.value.and_utc()
    } else if let Some(zone) = config.timezone.as_deref() {
        localize(parsed.value, zone, config.ambiguous, config.nonexistent)
            .map_err(|error| QuiltError::conversion("timeslice", None::<String>, error))?
            .with_timezone(&chrono::Utc)
    } else {
        parsed.value.and_utc()
    };
    match unit {
        TimeUnit::Nanoseconds => instant.timestamp_nanos_opt().ok_or_else(|| {
            QuiltError::conversion(
                "timeslice",
                None::<String>,
                "boundary is outside datetime range",
            )
        }),
        TimeUnit::Microseconds => {
            if instant.timestamp_subsec_nanos() % 1_000 != 0 {
                return Err(QuiltError::conversion(
                    "timeslice",
                    None::<String>,
                    "boundary precision is finer than the typed datetime unit",
                ));
            }
            Ok(instant.timestamp_micros())
        }
        TimeUnit::Milliseconds => {
            if instant.timestamp_subsec_nanos() % 1_000_000 != 0 {
                return Err(QuiltError::conversion(
                    "timeslice",
                    None::<String>,
                    "boundary precision is finer than the typed datetime unit",
                ));
            }
            Ok(instant.timestamp_millis())
        }
    }
}

fn parse_datetime_string_canonical(
    time_str: &str,
    config: &DateTimeConfig,
    diagnostics: &ParserDiagnostics,
) -> Result<Option<String>, String> {
    let datetime =
        match parse_datetime_detailed_with_diagnostics(time_str, config, Some(diagnostics))? {
            None => None,
            Some(parsed) => {
                if parsed.authoritative_offset {
                    Some(parsed.value)
                } else if let Some(timezone) = config.timezone.as_deref() {
                    Some(
                        localize(parsed.value, timezone, config.ambiguous, config.nonexistent)?
                            .with_timezone(&chrono::Utc)
                            .naive_utc(),
                    )
                } else {
                    Some(parsed.value)
                }
            }
        };
    Ok(datetime.map(|datetime| datetime.format("%Y-%m-%d %H:%M:%S%.6f").to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_nanosecond_timeslice_keeps_boundary_precision_and_negative_epochs() {
        let values =
            Int64Chunked::from_vec("ts".into(), vec![-1, 123_456_788, 123_456_789, 123_456_790])
                .into_datetime(TimeUnit::Nanoseconds, None)
                .into_series();
        let frame = DataFrame::new_infer_height(vec![values.into()])
            .unwrap()
            .lazy();
        let filtered = timeslice(
            &frame,
            "ts",
            Some("1970-01-01T00:00:00.123456789Z"),
            Some("1970-01-01T00:00:00.123456789Z"),
            &DateTimeConfig::default(),
        )
        .unwrap()
        .collect()
        .unwrap();
        assert_eq!(filtered.height(), 1);
        assert_eq!(
            filtered
                .column("ts")
                .unwrap()
                .datetime()
                .unwrap()
                .physical()
                .get(0),
            Some(123_456_789)
        );
    }
}
