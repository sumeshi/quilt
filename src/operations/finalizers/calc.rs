use polars::prelude::*;

pub const MODES: [&str; 6] = ["sum", "avg", "min", "max", "median", "std"];

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("Error: {}", message.as_ref());
    std::process::exit(1);
}

fn is_numeric_dtype(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    )
}

fn scalar_text(value: &AnyValue<'_>) -> String {
    match value {
        AnyValue::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

pub fn calc(df: &LazyFrame, column: &str, mode: &str) {
    if !MODES.contains(&mode) {
        fail(format!("Unknown calc aggregation '{mode}'"));
    }

    let frame = df.clone().collect().unwrap_or_else(|error| {
        fail(format!("Failed to evaluate input before calc: {error}"));
    });
    let source = frame.column(column).unwrap_or_else(|_| {
        fail(format!("Column '{column}' not found for calc"));
    });
    if !is_numeric_dtype(source.dtype()) {
        fail(format!(
            "Calc column '{column}' must be numeric, found {}",
            source.dtype()
        ));
    }

    let series = source.as_materialized_series();
    let count = series.len().saturating_sub(series.null_count());
    if count == 0 || (mode == "std" && count < 2) {
        println!("null");
        return;
    }

    let output = match mode {
        "sum" => scalar_text(
            series
                .sum_reduce()
                .unwrap_or_else(|error| fail(format!("Failed to calculate sum: {error}")))
                .value(),
        ),
        "min" => scalar_text(
            series
                .min_reduce()
                .unwrap_or_else(|error| fail(format!("Failed to calculate min: {error}")))
                .value(),
        ),
        "max" => scalar_text(
            series
                .max_reduce()
                .unwrap_or_else(|error| fail(format!("Failed to calculate max: {error}")))
                .value(),
        ),
        "avg" => series
            .mean()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        "median" => series
            .median()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        "std" => series
            .std(1)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_string()),
        _ => unreachable!(),
    };
    println!("{output}");
}
