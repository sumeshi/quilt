use polars::prelude::*;

const UNITS: [(&str, u128); 9] = [
    ("KiB", 1024),
    ("MiB", 1024 * 1024),
    ("GiB", 1024 * 1024 * 1024),
    ("TiB", 1024 * 1024 * 1024 * 1024),
    ("KB", 1000),
    ("MB", 1000 * 1000),
    ("GB", 1000 * 1000 * 1000),
    ("TB", 1000 * 1000 * 1000 * 1000),
    ("B", 1),
];

fn parse_decimal_bytes(value: &str, multiplier: u128) -> Result<u64, String> {
    if value.starts_with('-') {
        return Err("negative sizes are not allowed".to_string());
    }
    let value = value.strip_prefix('+').unwrap_or(value);
    if value.is_empty() {
        return Err("invalid magnitude".to_string());
    }

    let (integer_part, fraction_part) = match value.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (value, ""),
    };
    if value.matches('.').count() > 1
        || (integer_part.is_empty() && fraction_part.is_empty())
        || !integer_part
            .chars()
            .all(|character| character.is_ascii_digit())
        || !fraction_part
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err("invalid magnitude".to_string());
    }

    // Trailing fractional zeroes do not affect exactness and can otherwise
    // make a perfectly valid value exceed the decimal scale's bounds.
    let fraction_part = fraction_part.trim_end_matches('0');
    let integer_part = integer_part.trim_start_matches('0');
    let digits = format!("{integer_part}{fraction_part}");
    let numerator = if digits.is_empty() {
        0
    } else {
        digits
            .parse::<u128>()
            .map_err(|_| "magnitude overflow".to_string())?
    };
    let scale = 10u128
        .checked_pow(fraction_part.len() as u32)
        .ok_or_else(|| "magnitude overflow".to_string())?;
    let scaled_bytes = numerator
        .checked_mul(multiplier)
        .ok_or_else(|| "byte value overflow".to_string())?;
    if scaled_bytes % scale != 0 {
        return Err("fractional byte result is not integral".to_string());
    }
    let bytes = scaled_bytes / scale;
    u64::try_from(bytes).map_err(|_| "byte value overflow".to_string())
}

fn parse_size(value: &str) -> Result<u64, String> {
    let value = value.trim();
    let (unit, multiplier) = UNITS
        .iter()
        .find(|(unit, _)| value.ends_with(unit))
        .copied()
        .ok_or_else(|| "unknown or missing unit".to_string())?;
    let magnitude = &value[..value.len() - unit.len()];
    parse_decimal_bytes(magnitude, multiplier)
}

pub fn parse_size_column(df: &LazyFrame, column: &str) -> LazyFrame {
    let mut frame = df.clone().collect().unwrap_or_else(|error| {
        eprintln!("Error: Failed to evaluate input before parse-size: {error}");
        std::process::exit(1);
    });
    let source = frame.column(column).unwrap_or_else(|_| {
        eprintln!("Error: Column '{column}' not found for parse-size operation");
        std::process::exit(1);
    });
    let strings = source
        .cast(&DataType::String)
        .unwrap_or_else(|error| {
            eprintln!("Error: Cannot read column '{column}' for parse-size: {error}");
            std::process::exit(1);
        })
        .str()
        .unwrap_or_else(|error| {
            eprintln!("Error: Cannot read column '{column}' as text: {error}");
            std::process::exit(1);
        })
        .into_iter()
        .map(|value| value.map(ToOwned::to_owned))
        .collect::<Vec<_>>();

    let bytes = strings
        .iter()
        .enumerate()
        .map(|(row, value)| {
            value.as_deref().map(|value| {
                parse_size(value).unwrap_or_else(|error| {
                    eprintln!(
                        "Error: Cannot parse size in column '{column}' at row {row}: {error}"
                    );
                    std::process::exit(1);
                })
            })
        })
        .collect::<Vec<_>>();
    frame
        .replace(column, Series::new(column.into(), bytes))
        .unwrap_or_else(|error| {
            eprintln!("Error: Failed to replace column '{column}' after parse-size: {error}");
            std::process::exit(1);
        });
    frame.lazy()
}
