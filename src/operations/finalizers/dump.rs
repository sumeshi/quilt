use crate::error::QuiltError;
use crate::operations::finalizers::{atomic_path, validate_destination, FinalizerResult};
use polars::prelude::*;
use std::path::{Path, PathBuf};

fn output_path(path: Option<&str>) -> PathBuf {
    path.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(format!(
            "dump_{}.csv",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ))
    })
}

fn validate_separator(separator: char) -> Result<u8, QuiltError> {
    if !separator.is_ascii() {
        return Err(QuiltError::usage(
            "Error: Separator must be a single ASCII character",
        ));
    }
    Ok(separator as u8)
}

pub fn dump(
    df: &LazyFrame,
    output_path_opt: Option<&str>,
    separator: char,
) -> Result<FinalizerResult, QuiltError> {
    let separator = validate_separator(separator)?;
    let path = output_path(output_path_opt);
    if path == Path::new("-") {
        return Err(QuiltError::usage(
            "Error: The 'dump' command requires a file path. To print to stdout, use the 'show' command instead.",
        ));
    }
    validate_destination(&path, "write dump")?;
    atomic_path(&path, "write dump", |temp| {
        let options = CsvWriterOptions {
            serialize_options: SerializeOptions {
                separator,
                ..Default::default()
            },
            ..Default::default()
        };
        df.clone()
            .sink_csv(
                SinkTarget::Path(std::sync::Arc::new(temp.to_path_buf())),
                options,
                None,
                SinkOptions::default(),
            )
            .and_then(|frame| frame.collect())
            .map(|_| ())
            .map_err(|error| QuiltError::finalizer("write dump", error.to_string()))
    })?;
    Ok(FinalizerResult::File(path))
}
