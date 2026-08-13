//! Shared datetime parsing and timezone policy.
//!
//! Parsing precedence is explicit format, strict RFC3339/ISO and known
//! unambiguous formats, explicitly unit-tagged epoch, then bounded fuzzy
//! formats. Internal values are normalized to microsecond `NaiveDateTime`.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use polars::prelude::TimeUnit;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochUnit {
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbiguousPolicy {
    Error,
    Earliest,
    Latest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonexistentPolicy {
    Error,
    ShiftForward,
    ShiftBackward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParserFamily {
    ExplicitFormat,
    Rfc3339,
    IsoDateTime,
    VendorApache,
    VendorWeekday,
    VendorMonthName,
    Epoch,
    FuzzyNumeric,
    FuzzyMonthName,
}

#[derive(Clone, Default)]
pub struct ParserDiagnostics(Arc<Mutex<HashSet<ParserFamily>>>);

impl ParserDiagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&self, family: ParserFamily) {
        if let Ok(mut seen) = self.0.lock() {
            if seen.insert(family) {
                log::debug!("datetime parser family accepted: {family:?}");
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDateTime {
    pub value: NaiveDateTime,
    pub family: ParserFamily,
    pub authoritative_offset: bool,
}

/// Convert a bucket interval to internal microseconds.
pub fn interval_micros(interval: &str) -> Result<i64, String> {
    let (digits, suffix) = interval
        .strip_suffix('s')
        .map(|value| (value, 's'))
        .or_else(|| interval.strip_suffix('m').map(|value| (value, 'm')))
        .or_else(|| interval.strip_suffix('h').map(|value| (value, 'h')))
        .or_else(|| interval.strip_suffix('d').map(|value| (value, 'd')))
        .ok_or_else(|| "interval must match ^[1-9][0-9]*(s|m|h|d)$".to_string())?;
    if digits.is_empty()
        || !digits.chars().all(|character| character.is_ascii_digit())
        || digits.starts_with('0')
    {
        return Err("interval must match ^[1-9][0-9]*(s|m|h|d)$".to_string());
    }
    let count = digits
        .parse::<i64>()
        .map_err(|_| "interval is too large".to_string())?;
    let unit = match suffix {
        's' => 1_000_000,
        'm' => 60_000_000,
        'h' => 3_600_000_000,
        'd' => 86_400_000_000,
        _ => unreachable!(),
    };
    count
        .checked_mul(unit)
        .ok_or_else(|| "interval is too large".to_string())
}

/// Floor a raw datetime value using Euclidean division, preserving negatives.
pub fn floor_datetime_raw(value: i64, interval_micros: i64, unit: TimeUnit) -> Result<i64, String> {
    let interval = match unit {
        TimeUnit::Nanoseconds => interval_micros
            .checked_mul(1_000)
            .ok_or_else(|| "bucket interval overflows nanoseconds".to_string())?,
        TimeUnit::Microseconds => interval_micros,
        TimeUnit::Milliseconds => {
            if interval_micros % 1_000 != 0 {
                return Err("millisecond datetime requires an interval divisible by 1ms".into());
            }
            interval_micros / 1_000
        }
    };
    value
        .div_euclid(interval)
        .checked_mul(interval)
        .ok_or_else(|| "bucket floor overflow".to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeConfig {
    pub strict: bool,
    pub input_format: Option<String>,
    pub epoch_unit: Option<EpochUnit>,
    pub timezone: Option<String>,
    pub ambiguous: AmbiguousPolicy,
    pub nonexistent: NonexistentPolicy,
    /// True when any datetime option was explicitly supplied, even if its
    /// value equals the default. This prevents silently ignored options on
    /// already-typed columns.
    pub options_present: bool,
}

impl Default for DateTimeConfig {
    fn default() -> Self {
        Self {
            strict: false,
            input_format: None,
            epoch_unit: None,
            timezone: None,
            ambiguous: AmbiguousPolicy::Error,
            nonexistent: NonexistentPolicy::Error,
            options_present: false,
        }
    }
}

pub fn parse_epoch_unit(value: &str) -> Result<EpochUnit, String> {
    match value.to_ascii_lowercase().as_str() {
        "s" | "sec" | "seconds" => Ok(EpochUnit::Seconds),
        "ms" | "millis" | "milliseconds" => Ok(EpochUnit::Milliseconds),
        "us" | "micros" | "microseconds" => Ok(EpochUnit::Microseconds),
        "ns" | "nanos" | "nanoseconds" => Ok(EpochUnit::Nanoseconds),
        _ => Err("epoch unit must be one of s, ms, us, ns".into()),
    }
}

pub fn parse_ambiguous_policy(value: &str) -> Result<AmbiguousPolicy, String> {
    match value.to_ascii_lowercase().as_str() {
        "error" => Ok(AmbiguousPolicy::Error),
        "earliest" => Ok(AmbiguousPolicy::Earliest),
        "latest" => Ok(AmbiguousPolicy::Latest),
        _ => Err("ambiguous policy must be error, earliest, or latest".into()),
    }
}

pub fn parse_nonexistent_policy(value: &str) -> Result<NonexistentPolicy, String> {
    match value.to_ascii_lowercase().as_str() {
        "error" => Ok(NonexistentPolicy::Error),
        "shift-forward" => Ok(NonexistentPolicy::ShiftForward),
        "shift-backward" => Ok(NonexistentPolicy::ShiftBackward),
        _ => Err("nonexistent policy must be error, shift-forward, or shift-backward".into()),
    }
}

fn parse_epoch(value: &str, unit: EpochUnit) -> Result<NaiveDateTime, String> {
    let number: i128 = value
        .trim()
        .parse()
        .map_err(|_| "epoch value must be an integer when --epoch-unit is used".to_string())?;
    let micros = match unit {
        EpochUnit::Seconds => number.checked_mul(1_000_000),
        EpochUnit::Milliseconds => number.checked_mul(1_000),
        EpochUnit::Microseconds => Some(number),
        EpochUnit::Nanoseconds => {
            if number % 1_000 != 0 {
                return Err(
                    "nanosecond epoch value is below the internal microsecond precision".into(),
                );
            }
            Some(number / 1_000)
        }
    }
    .ok_or_else(|| "epoch value overflows datetime precision".to_string())?;
    let micros =
        i64::try_from(micros).map_err(|_| "epoch value is outside datetime range".to_string())?;
    DateTime::<Utc>::from_timestamp_micros(micros)
        .map(|value| value.naive_utc())
        .ok_or_else(|| "epoch value is outside datetime range".to_string())
}

fn parse_format(value: &str, format: &str) -> Result<NaiveDateTime, String> {
    if let Ok(value) = DateTime::parse_from_str(value, format) {
        return Ok(value.naive_utc());
    }
    if let Ok(value) = NaiveDateTime::parse_from_str(value, format) {
        return Ok(value);
    }
    NaiveDate::parse_from_str(value, format)
        .map(|value| value.and_time(NaiveTime::MIN))
        .map_err(|error| error.to_string())
}

const STRICT_FORMATS: &[(&str, ParserFamily)] = &[
    ("%Y-%m-%dT%H:%M:%S%.f", ParserFamily::IsoDateTime),
    ("%Y-%m-%d %H:%M:%S%.f", ParserFamily::IsoDateTime),
    ("%Y-%m-%d %H:%M:%S", ParserFamily::IsoDateTime),
    ("%Y-%m-%d %H:%M", ParserFamily::IsoDateTime),
    ("%Y-%m-%d", ParserFamily::IsoDateTime),
    ("%Y/%m/%d %H:%M:%S%.f", ParserFamily::IsoDateTime),
    ("%Y/%m/%d %H:%M:%S", ParserFamily::IsoDateTime),
    ("%d/%b/%Y:%H:%M:%S", ParserFamily::VendorApache),
    ("%d-%b-%Y %H:%M:%S", ParserFamily::VendorApache),
    ("%a %b %d %H:%M:%S %Y", ParserFamily::VendorWeekday),
    ("%a, %d %b %Y %H:%M:%S", ParserFamily::VendorWeekday),
];

const FUZZY_FORMATS: &[(&str, ParserFamily)] = &[
    ("%m/%d/%Y %H:%M:%S", ParserFamily::FuzzyNumeric),
    ("%d/%m/%Y %H:%M:%S", ParserFamily::FuzzyNumeric),
    ("%m/%d/%Y", ParserFamily::FuzzyNumeric),
    ("%d/%m/%Y", ParserFamily::FuzzyNumeric),
    ("%B %d, %Y %I:%M %p", ParserFamily::FuzzyMonthName),
    ("%b %d, %Y %I:%M %p", ParserFamily::FuzzyMonthName),
    ("%d %b %Y %H:%M:%S", ParserFamily::FuzzyMonthName),
    ("%b %d %Y %H:%M:%S", ParserFamily::FuzzyMonthName),
];

fn strict_formats(value: &str) -> Vec<(NaiveDateTime, ParserFamily)> {
    let mut parsed = Vec::new();
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        parsed.push((value.naive_utc(), ParserFamily::Rfc3339));
    }
    for (format, family) in STRICT_FORMATS {
        if let Ok(value) = parse_format(value, format) {
            parsed.push((value, *family));
        }
    }
    parsed.sort_unstable_by_key(|(value, _)| *value);
    parsed.dedup_by_key(|(value, _)| *value);
    parsed
}

fn fuzzy_formats(value: &str) -> Vec<(NaiveDateTime, ParserFamily)> {
    let mut parsed = Vec::new();
    for (format, family) in FUZZY_FORMATS {
        if let Ok(value) = parse_format(value, format) {
            parsed.push((value, *family));
        } else if let Ok(date) = NaiveDate::parse_from_str(value, format) {
            parsed.push((date.and_time(NaiveTime::MIN), *family));
        }
    }
    parsed.sort_unstable_by_key(|(value, _)| *value);
    parsed.dedup_by_key(|(value, _)| *value);
    parsed
}

pub fn parse_datetime_detailed(
    value: &str,
    config: &DateTimeConfig,
) -> Result<Option<ParsedDateTime>, String> {
    parse_datetime_detailed_with_diagnostics(value, config, None)
}

pub fn parse_datetime_detailed_with_diagnostics(
    value: &str,
    config: &DateTimeConfig,
    diagnostics: Option<&ParserDiagnostics>,
) -> Result<Option<ParsedDateTime>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if let Some(format) = &config.input_format {
        let authoritative_offset = DateTime::parse_from_str(value, format).is_ok();
        return parse_format(value, format)
            .map(|value| {
                if let Some(diagnostics) = diagnostics {
                    diagnostics.record(ParserFamily::ExplicitFormat);
                }
                Some(ParsedDateTime {
                    value,
                    family: ParserFamily::ExplicitFormat,
                    authoritative_offset,
                })
            })
            .map_err(|error| format!("input format '{format}' did not match: {error}"));
    }
    if let Ok(value) = DateTime::parse_from_rfc3339(value) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.record(ParserFamily::Rfc3339);
        }
        return Ok(Some(ParsedDateTime {
            value: value.naive_utc(),
            family: ParserFamily::Rfc3339,
            authoritative_offset: true,
        }));
    }
    let strict = strict_formats(value);
    if strict.len() == 1 {
        let Some((value, family)) = strict.into_iter().next() else {
            return Err("datetime strict parser produced no interpretation".into());
        };
        if let Some(diagnostics) = diagnostics {
            diagnostics.record(family);
        }
        return Ok(Some(ParsedDateTime {
            value,
            family,
            authoritative_offset: false,
        }));
    }
    if strict.len() > 1 {
        return Err("datetime has multiple strict interpretations; provide --input-format".into());
    }
    if let Some(unit) = config.epoch_unit {
        return parse_epoch(value, unit).map(|value| {
            if let Some(diagnostics) = diagnostics {
                diagnostics.record(ParserFamily::Epoch);
            }
            Some(ParsedDateTime {
                value,
                family: ParserFamily::Epoch,
                authoritative_offset: false,
            })
        });
    }
    if config.strict {
        return Err("datetime did not match a strict RFC3339/ISO/known format".into());
    }
    let fuzzy = fuzzy_formats(value);
    match fuzzy.as_slice() {
        [] => Err("datetime did not match a supported format".into()),
        [(value, family)] => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.record(*family);
            }
            Ok(Some(ParsedDateTime {
                value: *value,
                family: *family,
                authoritative_offset: false,
            }))
        }
        _ => Err("datetime is ambiguous; provide --input-format".into()),
    }
}

pub fn parse_datetime(
    value: &str,
    config: &DateTimeConfig,
) -> Result<Option<NaiveDateTime>, String> {
    Ok(parse_datetime_detailed(value, config)?.map(|parsed| parsed.value))
}

pub fn localize(
    value: NaiveDateTime,
    timezone: &str,
    ambiguous: AmbiguousPolicy,
    nonexistent: NonexistentPolicy,
) -> Result<DateTime<Tz>, String> {
    let timezone: Tz = timezone
        .parse()
        .map_err(|_| format!("invalid timezone '{timezone}'"))?;
    match timezone.from_local_datetime(&value) {
        chrono::LocalResult::Single(value) => Ok(value),
        chrono::LocalResult::Ambiguous(earliest, latest) => match ambiguous {
            AmbiguousPolicy::Error => {
                Err("ambiguous local datetime; choose earliest or latest".into())
            }
            AmbiguousPolicy::Earliest => Ok(earliest),
            AmbiguousPolicy::Latest => Ok(latest),
        },
        chrono::LocalResult::None => match nonexistent {
            NonexistentPolicy::Error => {
                Err("nonexistent local datetime; choose a shift policy".into())
            }
            NonexistentPolicy::ShiftForward => shift_nonexistent(&timezone, value, true),
            NonexistentPolicy::ShiftBackward => shift_nonexistent(&timezone, value, false),
        },
    }
}

/// Apply the same typed DST policies to the process-local timezone.
pub fn localize_local(
    value: NaiveDateTime,
    ambiguous: AmbiguousPolicy,
    nonexistent: NonexistentPolicy,
) -> Result<DateTime<Local>, String> {
    match Local.from_local_datetime(&value) {
        chrono::LocalResult::Single(value) => Ok(value),
        chrono::LocalResult::Ambiguous(earliest, latest) => match ambiguous {
            AmbiguousPolicy::Error => Err("ambiguous local datetime".into()),
            AmbiguousPolicy::Earliest => Ok(earliest),
            AmbiguousPolicy::Latest => Ok(latest),
        },
        chrono::LocalResult::None => match nonexistent {
            NonexistentPolicy::Error => Err("nonexistent local datetime".into()),
            policy => {
                for minutes in 1..=2_880 {
                    let delta = chrono::Duration::minutes(i64::from(minutes));
                    let candidate = if policy == NonexistentPolicy::ShiftForward {
                        value + delta
                    } else {
                        value - delta
                    };
                    if let chrono::LocalResult::Single(value) =
                        Local.from_local_datetime(&candidate)
                    {
                        return Ok(value);
                    }
                }
                Err("could not shift nonexistent local datetime".into())
            }
        },
    }
}

fn shift_nonexistent(
    timezone: &Tz,
    value: NaiveDateTime,
    forward: bool,
) -> Result<DateTime<Tz>, String> {
    // Search in wall-clock minutes so a DST gap (including 30/24-hour gaps)
    // lands on the nearest valid local time while retaining the requested
    // direction and minute/second precision.
    for minutes in 1..=2_880 {
        let delta = chrono::Duration::minutes(i64::from(minutes));
        let candidate = if forward {
            value + delta
        } else {
            value - delta
        };
        if let chrono::LocalResult::Single(value) = timezone.from_local_datetime(&candidate) {
            return Ok(value);
        }
    }
    Err("could not shift nonexistent local datetime into a valid timezone instant".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn parses_offsets_and_iso_as_microsecond_naive_utc() {
        let value = parse_datetime(
            "2024-01-02T03:04:05.123456+09:00",
            &DateTimeConfig::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(value.to_string(), "2024-01-01 18:04:05.123456");
        assert_eq!(
            parse_datetime("2024-01-02", &DateTimeConfig::default())
                .unwrap()
                .unwrap()
                .time(),
            NaiveTime::MIN
        );
    }

    #[test]
    fn strict_disables_fuzzy_and_numeric_dates_are_ambiguous() {
        let fuzzy = parse_datetime("01/02/2024", &DateTimeConfig::default());
        assert!(fuzzy.unwrap_err().contains("ambiguous"));
        let strict = parse_datetime(
            "01/02/2024",
            &DateTimeConfig {
                strict: true,
                ..Default::default()
            },
        );
        assert!(strict.unwrap_err().contains("strict"));
        let explicit = parse_datetime(
            "01/02/2024",
            &DateTimeConfig {
                input_format: Some("%d/%m/%Y".into()),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(explicit.date().to_string(), "2024-02-01");
    }

    #[test]
    fn epoch_units_are_explicit_and_preserve_negative_values() {
        let expected = "1969-12-31 23:59:59.999999";
        for (unit, value) in [
            (EpochUnit::Seconds, "-1"),
            (EpochUnit::Milliseconds, "-1"),
            (EpochUnit::Microseconds, "-1"),
        ] {
            let parsed = parse_datetime(
                value,
                &DateTimeConfig {
                    epoch_unit: Some(unit),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
            if unit == EpochUnit::Seconds {
                assert_eq!(parsed.to_string(), "1969-12-31 23:59:59");
            } else if unit == EpochUnit::Milliseconds {
                assert_eq!(parsed.to_string(), "1969-12-31 23:59:59.999");
            } else {
                assert_eq!(parsed.to_string(), expected);
            }
        }
        assert!(parse_datetime(
            "1",
            &DateTimeConfig {
                epoch_unit: Some(EpochUnit::Nanoseconds),
                ..Default::default()
            }
        )
        .is_err());
    }

    #[test]
    fn dst_policies_are_typed_and_directional() {
        let timezone: Tz = "America/New_York".parse().unwrap();
        let ambiguous = NaiveDate::from_ymd_opt(2024, 11, 3)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        let earliest = localize(
            ambiguous,
            "America/New_York",
            AmbiguousPolicy::Earliest,
            NonexistentPolicy::Error,
        )
        .unwrap();
        let latest = localize(
            ambiguous,
            "America/New_York",
            AmbiguousPolicy::Latest,
            NonexistentPolicy::Error,
        )
        .unwrap();
        assert!(earliest < latest);
        let nonexistent = NaiveDate::from_ymd_opt(2024, 3, 10)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert!(localize(
            nonexistent,
            "America/New_York",
            AmbiguousPolicy::Error,
            NonexistentPolicy::Error
        )
        .is_err());
        let forward = localize(
            nonexistent,
            "America/New_York",
            AmbiguousPolicy::Error,
            NonexistentPolicy::ShiftForward,
        )
        .unwrap();
        let backward = localize(
            nonexistent,
            "America/New_York",
            AmbiguousPolicy::Error,
            NonexistentPolicy::ShiftBackward,
        )
        .unwrap();
        assert_eq!(forward.time().hour(), 3);
        assert_eq!(backward.time().hour(), 1);
        assert_eq!(timezone, "America/New_York".parse().unwrap());
    }

    #[test]
    fn bucket_interval_and_floor_helpers_share_microsecond_semantics() {
        assert_eq!(interval_micros("5m").unwrap(), 300_000_000);
        assert_eq!(
            floor_datetime_raw(-1, 1_000_000, TimeUnit::Microseconds).unwrap(),
            -1_000_000
        );
        assert!(floor_datetime_raw(1, i64::MAX, TimeUnit::Nanoseconds).is_err());
        assert!(interval_micros("0s").is_err());
    }

    #[test]
    fn parser_diagnostics_are_independent_per_expression() {
        let first = ParserDiagnostics::new();
        let second = ParserDiagnostics::new();
        let config = DateTimeConfig::default();
        let parsed =
            parse_datetime_detailed_with_diagnostics("2024-01-02T03:04:05Z", &config, Some(&first))
                .unwrap();
        let parsed_again = parse_datetime_detailed_with_diagnostics(
            "2024-01-02T03:04:05Z",
            &config,
            Some(&second),
        )
        .unwrap();
        assert_eq!(parsed, parsed_again);
        assert!(first.0.lock().unwrap().contains(&ParserFamily::Rfc3339));
        assert!(second.0.lock().unwrap().contains(&ParserFamily::Rfc3339));
    }
}
