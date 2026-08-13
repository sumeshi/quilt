use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;
pub fn uniq(df: &LazyFrame) -> Result<LazyFrame, QuiltError> {
    // Duplicate elimination is a global barrier at execution time, but this
    // operation only appends the lazy unique node to the plan.
    LogController::debug("Applying uniq - removing duplicates based on all columns");
    Ok(df.clone().unique_stable(None, UniqueKeepStrategy::First))
}
