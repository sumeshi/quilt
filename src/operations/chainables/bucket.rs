use polars::prelude::*;

fn interval_micros(interval: &str) -> Result<i64, String> {
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
    let unit_micros = match suffix {
        's' => 1_000_000,
        'm' => 60_000_000,
        'h' => 3_600_000_000,
        'd' => 86_400_000_000,
        _ => unreachable!(),
    };
    count
        .checked_mul(unit_micros)
        .ok_or_else(|| "interval is too large".to_string())
}

pub fn bucket(df: &LazyFrame, column: &str, interval: &str, output: Option<&str>) -> LazyFrame {
    let interval = interval_micros(interval).unwrap_or_else(|error| {
        eprintln!("Error: Invalid bucket interval '{interval}': {error}");
        std::process::exit(1);
    });
    let output = output
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{column}_bucket"));

    let mut frame = df.clone().collect().unwrap_or_else(|error| {
        eprintln!("Error: Failed to evaluate input before bucket: {error}");
        std::process::exit(1);
    });
    let source = frame.column(column).unwrap_or_else(|_| {
        eprintln!("Error: Column '{column}' not found for bucket operation");
        std::process::exit(1);
    });
    if !matches!(source.dtype(), DataType::Datetime(_, _)) {
        eprintln!("Error: Bucket column '{column}' must have datetime type");
        std::process::exit(1);
    }
    if frame.column(&output).is_ok() {
        eprintln!("Error: Bucket output column '{output}' already exists");
        std::process::exit(1);
    }

    let datetime = source.datetime().unwrap_or_else(|error| {
        eprintln!("Error: Cannot read datetime column '{column}' for bucket: {error}");
        std::process::exit(1);
    });
    let micros = datetime.cast_time_unit(TimeUnit::Microseconds);
    let values = micros.into_iter().map(|value| {
        value.map(|timestamp| {
            let quotient = timestamp.div_euclid(interval);
            quotient.checked_mul(interval).unwrap_or_else(|| {
                eprintln!(
                    "Error: Bucket floor overflow for column '{column}' and interval {interval}"
                );
                std::process::exit(1);
            })
        })
    });
    let bucketed = Int64Chunked::from_iter_options(output.clone().into(), values)
        .into_datetime(TimeUnit::Microseconds, None);
    frame.with_column(bucketed).unwrap_or_else(|error| {
        eprintln!("Error: Failed to add bucket column '{output}': {error}");
        std::process::exit(1);
    });
    frame.lazy()
}
