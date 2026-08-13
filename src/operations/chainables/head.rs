use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;
pub fn head(df: &LazyFrame, n: usize) -> Result<LazyFrame, QuiltError> {
    LogController::debug(&format!("Applying head: n={n}"));
    Ok(df.clone().slice(0, n as u32))
}
