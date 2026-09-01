use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;
pub fn head(df: &LazyFrame, n: usize) -> Result<LazyFrame, QuiltError> {
    LogController::debug("Applying head");
    Ok(df.clone().slice(0, u32::try_from(n).unwrap_or(u32::MAX)))
}
