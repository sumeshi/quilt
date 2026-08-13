use crate::error::QuiltError;
use crate::operations::finalizers::{atomic_path, validate_destination, FinalizerResult};
use polars::prelude::*;
use std::path::PathBuf;

pub fn dumpcache(
    df: &LazyFrame,
    output_path_opt: Option<&str>,
) -> Result<FinalizerResult, QuiltError> {
    let requested = output_path_opt.map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(format!(
            "cache_{}.parquet",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ))
    });
    if requested.as_os_str() == "-" {
        return Err(QuiltError::usage(
            "Error: The 'dumpcache' command requires a file path",
        ));
    }
    let path = if requested
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("parquet")
    {
        requested.with_extension("parquet")
    } else {
        requested
    };
    validate_destination(&path, "write dumpcache")?;
    atomic_path(&path, "write dumpcache", |temp| {
        df.clone()
            .sink_parquet(
                SinkTarget::Path(std::sync::Arc::new(temp.to_path_buf())),
                ParquetWriteOptions {
                    compression: ParquetCompression::Snappy,
                    ..Default::default()
                },
                None,
                SinkOptions::default(),
            )
            .and_then(|frame| frame.collect())
            .map(|_| ())
            .map_err(|error| QuiltError::finalizer("write dumpcache", error.to_string()))
    })?;
    Ok(FinalizerResult::File(path))
}
