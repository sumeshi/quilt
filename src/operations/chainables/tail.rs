use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;
pub fn tail(df: &LazyFrame, n: usize) -> Result<LazyFrame, QuiltError> {
    // Tail is a sink-time barrier in the execution engine because the last n
    // rows cannot be known without consuming the upstream input.
    LogController::debug("Applying tail");
    Ok(df.clone().tail(n as u32))
}
