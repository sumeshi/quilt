use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use polars::prelude::*;

/// CSV stdout is an intrinsic finalizer barrier: Polars has no LazyFrame sink
/// for an arbitrary stdout writer, so this stable result is materialized once.
pub fn show(df: &LazyFrame) -> Result<FinalizerResult, QuiltError> {
    let mut frame = df.clone().collect().map_err(|error| {
        QuiltError::operation("show", format!("failed to evaluate input: {error}"))
    })?;
    let mut output = Vec::new();
    CsvWriter::new(&mut output)
        .include_header(true)
        .with_separator(b',')
        .finish(&mut frame)
        .map_err(|error| QuiltError::finalizer("show CSV", error.to_string()))?;
    String::from_utf8(output)
        .map(FinalizerResult::Stdout)
        .map_err(|error| QuiltError::finalizer("show UTF-8", error.to_string()))
}
