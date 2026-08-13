use crate::error::QuiltError;
use crate::operations::datetime::{
    localize, localize_local, parse_ambiguous_policy, parse_datetime_detailed_with_diagnostics,
    AmbiguousPolicy, DateTimeConfig, NonexistentPolicy, ParserDiagnostics,
};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use polars::prelude::*;

fn convert_timezone(
    value: &str,
    from_tz: &str,
    to_tz: &str,
    output_format: Option<&str>,
    config: &DateTimeConfig,
    diagnostics: &ParserDiagnostics,
) -> Result<String, String> {
    let parsed = parse_datetime_detailed_with_diagnostics(value, config, Some(diagnostics))?
        .ok_or_else(|| "empty datetime".to_string())?;
    let utc = if parsed.authoritative_offset {
        DateTime::<Utc>::from_naive_utc_and_offset(parsed.value, Utc)
    } else if from_tz.eq_ignore_ascii_case("local") {
        localize_local(parsed.value, config.ambiguous, config.nonexistent)?.with_timezone(&Utc)
    } else {
        localize(parsed.value, from_tz, config.ambiguous, config.nonexistent)?.with_timezone(&Utc)
    };
    let target: Tz = to_tz
        .parse()
        .map_err(|_| format!("invalid timezone '{to_tz}'"))?;
    let target = utc.with_timezone(&target);
    let format = output_format.unwrap_or("%Y-%m-%dT%H:%M:%S%.6f%:z");
    Ok(target.format(format).to_string())
}

#[allow(clippy::too_many_arguments)]
pub fn changetz(
    df: &LazyFrame,
    colname: &str,
    from_tz: &str,
    to_tz: &str,
    input_format: Option<&str>,
    output_format: Option<&str>,
    ambiguous_time: Option<&str>,
    nonexistent_time: Option<&str>,
) -> Result<LazyFrame, QuiltError> {
    let config = DateTimeConfig {
        input_format: input_format.map(ToOwned::to_owned),
        ambiguous: ambiguous_time
            .map(parse_ambiguous_policy)
            .transpose()
            .map_err(QuiltError::usage)?
            .unwrap_or(AmbiguousPolicy::Error),
        nonexistent: nonexistent_time
            .map(crate::operations::datetime::parse_nonexistent_policy)
            .transpose()
            .map_err(QuiltError::usage)?
            .unwrap_or(NonexistentPolicy::Error),
        options_present: input_format.is_some()
            || ambiguous_time.is_some()
            || nonexistent_time.is_some(),
        ..DateTimeConfig::default()
    };
    changetz_with_config(df, colname, from_tz, to_tz, output_format, config)
}

pub fn changetz_with_config(
    df: &LazyFrame,
    colname: &str,
    from_tz: &str,
    to_tz: &str,
    output_format: Option<&str>,
    config: DateTimeConfig,
) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("changetz", None::<String>, error.to_string()))?;
    if schema.get(colname).is_none() {
        return Err(QuiltError::schema(
            "changetz",
            Some(colname),
            "column not found",
        ));
    }
    if from_tz.parse::<Tz>().is_err() && !from_tz.eq_ignore_ascii_case("local") {
        return Err(QuiltError::usage(format!(
            "invalid source timezone '{from_tz}'"
        )));
    }
    if to_tz.parse::<Tz>().is_err() {
        return Err(QuiltError::usage(format!(
            "Invalid target timezone '{to_tz}'"
        )));
    }
    let from_tz = from_tz.to_string();
    let to_tz = to_tz.to_string();
    let output_name = colname.to_string();
    let output_format = output_format.map(ToOwned::to_owned);
    let diagnostics = ParserDiagnostics::new();
    Ok(df.clone().with_column(
        col(colname)
            .map(
                move |series| {
                    let values = series.str()?.into_iter().enumerate();
                    let converted = values
                        .map(|(row, value)| {
                            value
                                .map(|value| {
                                    if value.trim().is_empty() {
                                        return Ok(None);
                                    }
                                    convert_timezone(
                                        value,
                                        &from_tz,
                                        &to_tz,
                                        output_format.as_deref(),
                                        &config,
                                        &diagnostics,
                                    )
                                    .map(Some)
                                    .map_err(|error| {
                                        PolarsError::ComputeError(
                                            format!("changetz column '{output_name}' row {row}: {error} (value redacted)").into(),
                                        )
                                    })
                                })
                                .transpose()
                                .map(|value| value.flatten())
                        })
                        .collect::<PolarsResult<Vec<_>>>()?;
                    Ok(Some(Column::from(Series::new(
                        output_name.clone().into(),
                        converted,
                    ))))
                },
                GetOutput::from_type(DataType::String),
            )
            .alias(colname),
    ))
}
