use super::datetime::parse_datetime_auto;
use polars::prelude::*;

fn conversion_error(column: &str, target: &str, row: usize, value: &str) -> ! {
    eprintln!("Error: Cannot cast column '{column}' value '{value}' at row {row} to {target}");
    std::process::exit(1);
}

fn source_as_strings(series: &Column, column: &str, target: &str) -> Vec<Option<String>> {
    let string_series = series.cast(&DataType::String).unwrap_or_else(|error| {
        eprintln!("Error: Cannot cast column '{column}' to {target}: {error}");
        std::process::exit(1);
    });
    string_series
        .str()
        .unwrap_or_else(|error| {
            eprintln!("Error: Cannot read column '{column}' as text: {error}");
            std::process::exit(1);
        })
        .into_iter()
        .map(|value| value.map(ToOwned::to_owned))
        .collect()
}

pub fn cast(df: &LazyFrame, column: &str, target: &str) -> LazyFrame {
    let target = target.to_ascii_lowercase();
    if !matches!(
        target.as_str(),
        "int" | "uint" | "float" | "string" | "bool" | "datetime"
    ) {
        eprintln!(
            "Error: Unsupported cast type '{target}'. Expected int, uint, float, string, bool, or datetime"
        );
        std::process::exit(1);
    }

    let mut frame = df.clone().collect().unwrap_or_else(|error| {
        eprintln!("Error: Failed to evaluate input before cast: {error}");
        std::process::exit(1);
    });
    let source = frame.column(column).unwrap_or_else(|_| {
        eprintln!("Error: Column '{column}' not found for cast operation");
        std::process::exit(1);
    });
    let values = source_as_strings(source, column, &target);

    let replacement = match target.as_str() {
        "int" => Series::new(
            column.into(),
            values
                .iter()
                .enumerate()
                .map(|(row, value)| {
                    value.as_deref().map(|value| {
                        value
                            .trim()
                            .parse::<i64>()
                            .unwrap_or_else(|_| conversion_error(column, &target, row, value))
                    })
                })
                .collect::<Vec<_>>(),
        ),
        "uint" => Series::new(
            column.into(),
            values
                .iter()
                .enumerate()
                .map(|(row, value)| {
                    value.as_deref().map(|value| {
                        value
                            .trim()
                            .parse::<u64>()
                            .unwrap_or_else(|_| conversion_error(column, &target, row, value))
                    })
                })
                .collect::<Vec<_>>(),
        ),
        "float" => Series::new(
            column.into(),
            values
                .iter()
                .enumerate()
                .map(|(row, value)| {
                    value.as_deref().map(|value| {
                        value
                            .trim()
                            .parse::<f64>()
                            .unwrap_or_else(|_| conversion_error(column, &target, row, value))
                    })
                })
                .collect::<Vec<_>>(),
        ),
        "string" => Series::new(column.into(), values),
        "bool" => Series::new(
            column.into(),
            values
                .iter()
                .enumerate()
                .map(|(row, value)| {
                    value
                        .as_deref()
                        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
                            "true" => true,
                            "false" => false,
                            _ => conversion_error(column, &target, row, value),
                        })
                })
                .collect::<Vec<_>>(),
        ),
        "datetime" => DatetimeChunked::from_naive_datetime_options(
            column.into(),
            values.iter().enumerate().map(|(row, value)| {
                value.as_deref().map(|value| {
                    parse_datetime_auto(value)
                        .unwrap_or_else(|| conversion_error(column, &target, row, value))
                })
            }),
            TimeUnit::Microseconds,
        )
        .into_series(),
        _ => unreachable!("cast target was validated above"),
    };

    frame.replace(column, replacement).unwrap_or_else(|error| {
        eprintln!("Error: Failed to replace column '{column}' after cast: {error}");
        std::process::exit(1);
    });
    frame.lazy()
}
