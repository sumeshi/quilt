use crate::error::QuiltError;
use crate::operations::datetime::{
    localize, parse_datetime_detailed_with_diagnostics, DateTimeConfig, ParserDiagnostics,
};
use polars::prelude::*;

pub fn cast(df: &LazyFrame, column: &str, target: &str) -> Result<LazyFrame, QuiltError> {
    cast_with_config(df, column, target, &DateTimeConfig::default())
}

pub fn cast_with_config(
    df: &LazyFrame,
    column: &str,
    target: &str,
    datetime: &DateTimeConfig,
) -> Result<LazyFrame, QuiltError> {
    let target = target.to_ascii_lowercase();
    if !matches!(
        target.as_str(),
        "int" | "uint" | "float" | "string" | "bool" | "datetime"
    ) {
        return Err(QuiltError::usage(format!(
            "Unsupported cast type '{target}'. Expected int, uint, float, string, bool, or datetime"
        )));
    }
    if target != "datetime" && datetime.options_present {
        return Err(QuiltError::usage(
            "datetime parsing options apply only to datetime casts",
        ));
    }
    let diagnostics = ParserDiagnostics::new();

    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("cast", None::<String>, error.to_string()))?;
    if !schema.iter_names().any(|name| name == column) {
        return Err(QuiltError::schema("cast", Some(column), "column not found"));
    }
    let timezone = datetime
        .timezone
        .as_deref()
        .map(|value| {
            value
                .parse::<chrono_tz::Tz>()
                .map_err(|_| QuiltError::usage(format!("invalid timezone '{value}'")))?;
            TimeZone::opt_try_new(Some(value))
                .map_err(|error| QuiltError::usage(format!("invalid timezone '{value}': {error}")))
        })
        .transpose()?
        .flatten();
    let dtype = match target.as_str() {
        "int" => DataType::Int64,
        "uint" => DataType::UInt64,
        "float" => DataType::Float64,
        "string" => DataType::String,
        "bool" => DataType::Boolean,
        "datetime" => DataType::Datetime(TimeUnit::Microseconds, timezone.clone()),
        _ => unreachable!(),
    };
    let expression = if target == "datetime" {
        let output_name = column.to_string();
        let config = datetime.clone();
        let diagnostics = diagnostics.clone();
        let output_dtype = dtype.clone();
        col(column).cast(DataType::String).map(
            move |series| {
                let values = series.str()?.into_iter().enumerate();
                let parsed = values
                    .map(|(row, value)| {
                        value
                            .map(|value| {
                                let parsed = parse_datetime_detailed_with_diagnostics(value, &config, Some(&diagnostics))
                                    .map_err(|error| PolarsError::ComputeError(
                                        format!("Cannot cast column '{}' at row {} to datetime (value redacted): {error}", output_name, row).into(),
                                    ))?
                                    .ok_or_else(|| PolarsError::ComputeError(
                                        format!("Cannot cast column '{}' at row {} to datetime (value redacted)", output_name, row).into(),
                                    ))?;
                                if let Some(timezone) = &config.timezone {
                                    if parsed.authoritative_offset {
                                        return Ok(parsed.value);
                                    }
                                    localize(parsed.value, timezone, config.ambiguous, config.nonexistent)
                                        .map_err(|error| PolarsError::ComputeError(
                                            format!("Cannot cast column '{}' at row {} to datetime (value redacted): {error}", output_name, row).into(),
                                        ))
                                        .map(|value| value.with_timezone(&chrono::Utc).naive_utc())
                                } else {
                                    Ok(parsed.value)
                                }
                            })
                            .transpose()
                    })
                    .collect::<PolarsResult<Vec<_>>>()?;
                let mut datetime = DatetimeChunked::from_naive_datetime_options(
                    output_name.clone().into(),
                    parsed,
                    TimeUnit::Microseconds,
                );
                if let DataType::Datetime(_, Some(timezone)) = &output_dtype {
                    datetime.set_time_zone(timezone.clone()).map_err(|error| PolarsError::ComputeError(error.to_string().into()))?;
                }
                Ok(Some(datetime.into_series().into()))
            },
            GetOutput::from_type(dtype),
        )
    } else {
        let target_name = target.clone();
        let output_name = column.to_string();
        let output_dtype = dtype.clone();
        col(column).map(
            move |series| {
                let casted = series.strict_cast(&output_dtype).map_err(|error| {
                    PolarsError::ComputeError(
                        format!(
                            "Cannot cast column '{output_name}' to {target_name}: {error}"
                        )
                        .into(),
                    )
                })?;
                Ok(Some(casted))
            },
            GetOutput::from_type(dtype),
        )
    }
    .alias(column);
    Ok(df.clone().with_columns([expression]))
}
