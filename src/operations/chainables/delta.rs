use polars::prelude::*;

fn fail(message: &str) -> ! {
    eprintln!("Error: {message}");
    std::process::exit(1);
}

fn numeric_dtype(dtype: &DataType) -> bool {
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

fn check_output_name(frame: &DataFrame, output: &str) {
    if frame.column(output).is_ok() {
        fail(&format!("Delta output column '{output}' already exists"));
    }
}

pub fn delta(df: &LazyFrame, column: &str, output: Option<&str>) -> LazyFrame {
    let mut frame = df.clone().collect().unwrap_or_else(|error| {
        fail(&format!("Failed to evaluate input before delta: {error}"));
    });
    let source = frame
        .column(column)
        .unwrap_or_else(|_| fail(&format!("Column '{column}' not found for delta operation")));
    let output = output
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{column}_delta"));
    check_output_name(&frame, &output);

    if numeric_dtype(source.dtype()) {
        let float_source = source.cast(&DataType::Float64).unwrap_or_else(|error| {
            fail(&format!("Cannot read numeric column '{column}': {error}"))
        });
        let values = float_source.f64().unwrap_or_else(|error| {
            fail(&format!("Cannot read numeric column '{column}': {error}"))
        });
        let mut previous: Option<f64> = None;
        let deltas = values
            .into_iter()
            .map(|current| {
                let delta = match (previous, current) {
                    (Some(previous), Some(current)) => {
                        let delta = current - previous;
                        if current.is_finite() && previous.is_finite() && !delta.is_finite() {
                            fail("Numeric delta overflow");
                        }
                        Some(delta)
                    }
                    _ => None,
                };
                previous = current;
                delta
            })
            .collect::<Vec<_>>();
        frame
            .with_column(Series::new(output.into(), deltas))
            .unwrap_or_else(|error| fail(&format!("Failed to add delta column: {error}")));
    } else if matches!(source.dtype(), DataType::Datetime(_, _)) {
        let datetime = source.datetime().unwrap_or_else(|error| {
            fail(&format!("Cannot read datetime column '{column}': {error}"))
        });
        let source_unit = datetime.time_unit();
        let mut previous: Option<i64> = None;
        let deltas = datetime.into_iter().map(|current| {
            let current = current.map(|value| match source_unit {
                TimeUnit::Nanoseconds => value.div_euclid(1_000),
                TimeUnit::Microseconds => value,
                TimeUnit::Milliseconds => value
                    .checked_mul(1_000)
                    .unwrap_or_else(|| fail("Datetime value cannot be represented in μs")),
            });
            let delta = match (previous, current) {
                (Some(previous), Some(current)) => Some(
                    current
                        .checked_sub(previous)
                        .unwrap_or_else(|| fail("Datetime delta overflow")),
                ),
                _ => None,
            };
            previous = current;
            delta
        });
        let duration = Int64Chunked::from_iter_options(output.into(), deltas)
            .into_duration(TimeUnit::Microseconds);
        frame
            .with_column(duration)
            .unwrap_or_else(|error| fail(&format!("Failed to add delta column: {error}")));
    } else {
        fail(&format!(
            "Delta column '{column}' must be numeric or datetime, found {}",
            source.dtype()
        ));
    }
    frame.lazy()
}
