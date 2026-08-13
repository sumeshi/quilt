use crate::controllers::log::LogController;
use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use polars::prelude::*;

pub fn showquery(df: &LazyFrame) -> Result<FinalizerResult, QuiltError> {
    LogController::debug("Showing query plan for DataFrame");
    let logical = df
        .clone()
        .describe_plan()
        .map_err(|e| QuiltError::Finalizer {
            operation: "showquery logical plan".into(),
            message: e.to_string(),
        })?;
    let optimized = df
        .clone()
        .describe_optimized_plan()
        .map_err(|e| QuiltError::Finalizer {
            operation: "showquery optimized plan".into(),
            message: e.to_string(),
        })?;
    Ok(FinalizerResult::PlanTable(format!(
        "Logical query plan:\n{logical}\n\nOptimized query plan:\n{optimized}\n"
    )))
}
