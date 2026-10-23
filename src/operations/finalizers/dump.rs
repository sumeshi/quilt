use crate::error::QuiltError;
use crate::operations::finalizers::{
    atomic_path, file_sink_target, validate_destination, FinalizerResult,
};
use polars::prelude::*;
use std::path::{Path, PathBuf};

fn output_path(path: Option<&str>) -> PathBuf {
    path.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(format!(
            "dump_{}.csv",
            chrono::Local::now().format("%Y%m%d_%H%M%S_%9f")
        ))
    })
}

pub fn dump(
    df: &LazyFrame,
    output_path_opt: Option<&str>,
    separator: char,
) -> Result<FinalizerResult, QuiltError> {
    let separator = crate::controllers::csv::separator_byte(&separator.to_string())?;
    let path = output_path(output_path_opt);
    if path == Path::new("-") {
        return Err(QuiltError::usage(
            "Error: The 'dump' command requires a file path. To print to stdout, use the 'show' command instead.",
        ));
    }
    validate_destination(&path, "write dump")?;
    atomic_path(&path, "write dump", |temp| {
        let frame = super::csv::csv_frame(df, "write dump")?;
        let target = file_sink_target(temp, "write dump")?;
        let options = CsvWriterOptions {
            serialize_options: SerializeOptions {
                separator,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        };
        frame
            .sink(
                SinkDestination::File { target },
                FileWriteFormat::Csv(options),
                UnifiedSinkArgs::default(),
            )
            .and_then(|frame| frame.collect())
            .map(|_| ())
            .map_err(|error| QuiltError::finalizer("write dump", error.to_string()))
    })?;
    Ok(FinalizerResult::File(path))
}
