use crate::error::QuiltError;
use crate::operations::datetime::{
    floor_datetime_raw, interval_micros, localize, parse_datetime_detailed_with_diagnostics,
    DateTimeConfig, ParserDiagnostics,
};
use polars::prelude::*;

// Datetime parsing is centralized in `operations::datetime`; bucket receives
// an already typed Datetime and preserves its timezone/unit metadata. Its
// interval arithmetic uses the shared internal microsecond precision before
// converting back to the source unit.

pub fn validate_interval(interval: &str) -> Result<(), QuiltError> {
    interval_micros(interval).map(|_| ()).map_err(|error| {
        QuiltError::conversion(
            "bucket",
            None::<String>,
            format!("Invalid bucket interval '{interval}': {error}"),
        )
    })
}

pub fn bucket(
    df: &LazyFrame,
    column: &str,
    interval: &str,
    output: Option<&str>,
) -> Result<LazyFrame, QuiltError> {
    bucket_with_config(df, column, interval, output, DateTimeConfig::default())
}

pub fn bucket_with_config(
    df: &LazyFrame,
    column: &str,
    interval: &str,
    output: Option<&str>,
    datetime: DateTimeConfig,
) -> Result<LazyFrame, QuiltError> {
    let diagnostics = ParserDiagnostics::new();
    let interval_micros = interval_micros(interval).map_err(|error| {
        QuiltError::conversion(
            "bucket",
            None::<String>,
            format!("Invalid bucket interval '{interval}': {error}"),
        )
    })?;
    let output = output
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{column}_bucket"));

    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("bucket", None::<String>, error.to_string()))?;
    let source = schema
        .get(column)
        .ok_or_else(|| QuiltError::schema("bucket", Some(column), "column not found"))?;
    if !matches!(source, DataType::Datetime(_, _)) && !matches!(source, DataType::String) {
        return Err(QuiltError::schema(
            "bucket",
            Some(column),
            "column must have datetime or string type",
        ));
    }
    if schema.get(&output).is_some() {
        return Err(QuiltError::schema(
            "bucket",
            Some(output.as_str()),
            "output column already exists",
        ));
    }

    let (unit, timezone, parse_strings) = match source {
        DataType::Datetime(unit, timezone) => {
            if datetime.options_present {
                return Err(QuiltError::usage(
                    "bucket datetime parsing options apply only to string input; source is already typed",
                ));
            }
            (*unit, timezone.clone(), false)
        }
        DataType::String => (TimeUnit::Microseconds, None, true),
        _ => unreachable!(),
    };
    let timezone = datetime
        .timezone
        .as_deref()
        .map(|zone| {
            zone.parse::<chrono_tz::Tz>()
                .map_err(|_| QuiltError::usage(format!("invalid timezone '{zone}'")))?;
            polars::prelude::TimeZone::opt_try_new(Some(zone))
                .map_err(|error| QuiltError::usage(format!("invalid timezone '{zone}': {error}")))
        })
        .transpose()?
        .flatten()
        .or(timezone);
    let output_name = output.clone();
    let column_name = column.to_string();
    let output_timezone = timezone.clone();
    let bucketed = col(column)
        .map(
            move |series| {
                let buckets = if parse_strings {
                    let values = series.str()?.into_iter().enumerate();
                    values.map(|(row, value)| value.map(|value| -> PolarsResult<Option<i64>> {
                        let parsed = match parse_datetime_detailed_with_diagnostics(value, &datetime, Some(&diagnostics)).map_err(|error| PolarsError::ComputeError(
                            format!("bucket column '{column_name}' row {row}: {error} (value redacted)").into()))?
                        {
                            Some(parsed) => parsed,
                            None => return Ok(None),
                        };
                        let parsed = if let Some(zone) = datetime.timezone.as_deref() {
                            if parsed.authoritative_offset {
                                parsed.value
                            } else {
                                localize(parsed.value, zone, datetime.ambiguous, datetime.nonexistent)
                                    .map_err(|error| PolarsError::ComputeError(format!("bucket column '{column_name}' row {row}: {error} (value redacted)").into()))?
                                    .with_timezone(&chrono::Utc).naive_utc()
                            }
                        } else { parsed.value };
                        floor_datetime_raw(parsed.and_utc().timestamp_micros(), interval_micros, TimeUnit::Microseconds)
                            .map(Some)
                            .map_err(|_| PolarsError::ComputeError(format!("bucket column '{column_name}' row {row}: bucket floor overflow").into()))
                    }).transpose().map(|value| value.flatten())).collect::<PolarsResult<Vec<_>>>()?
                } else {
                    series.datetime()?.into_iter().enumerate().map(|(row, value)| value.map(|value| {
                        floor_datetime_raw(value, interval_micros, unit).map_err(|_| PolarsError::ComputeError(format!("bucket column '{column_name}' row {row}: Bucket floor overflow").into()))
                    }).transpose()).collect::<PolarsResult<Vec<_>>>()?
                };
                Ok(Some(
                    Int64Chunked::from_iter_options(
                        output_name.clone().into(),
                        buckets.into_iter(),
                    )
                    .into_datetime(unit, output_timezone.clone())
                    .into_series()
                    .into(),
                ))
            },
            GetOutput::from_type(DataType::Datetime(unit, timezone)),
        )
        .alias(output);
    Ok(df.clone().with_columns([bucketed]))
}
