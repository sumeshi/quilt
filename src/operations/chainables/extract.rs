use polars::prelude::*;
use regex::Regex;

fn fail(message: &str) -> ! {
    eprintln!("Error: {message}");
    std::process::exit(1);
}

pub fn extract(df: &LazyFrame, column: &str, pattern: &str) -> LazyFrame {
    let regex = Regex::new(pattern)
        .unwrap_or_else(|error| fail(&format!("Invalid extract regex: {error}")));
    let group_names = regex
        .capture_names()
        .flatten()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if group_names.is_empty() {
        fail("Extract regex must contain at least one named capture group");
    }

    let mut frame = df.clone().collect().unwrap_or_else(|error| {
        fail(&format!("Failed to evaluate input before extract: {error}"));
    });
    let source = frame.column(column).unwrap_or_else(|_| {
        fail(&format!(
            "Column '{column}' not found for extract operation"
        ))
    });
    if source.dtype() != &DataType::String {
        fail(&format!(
            "Extract column '{column}' must be string, found {}",
            source.dtype()
        ));
    }
    for group_name in &group_names {
        if frame.column(group_name).is_ok() {
            fail(&format!(
                "Extract output column '{group_name}' already exists"
            ));
        }
    }

    let source_values = source
        .str()
        .unwrap_or_else(|error| fail(&format!("Cannot read extract column '{column}': {error}")));
    let source_values = source_values
        .into_iter()
        .map(|value| value.map(ToOwned::to_owned))
        .collect::<Vec<_>>();
    let captures = source_values
        .iter()
        .map(|value| value.as_deref().and_then(|value| regex.captures(value)))
        .collect::<Vec<_>>();
    for group_name in group_names {
        let values = captures
            .iter()
            .map(|capture| {
                capture.as_ref().and_then(|capture| {
                    capture
                        .name(&group_name)
                        .map(|value| value.as_str().to_owned())
                })
            })
            .collect::<Vec<_>>();
        frame
            .with_column(Series::new(group_name.into(), values))
            .unwrap_or_else(|error| fail(&format!("Failed to add extract column: {error}")));
    }
    frame.lazy()
}
