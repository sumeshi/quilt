use crate::controllers::log::LogController;
use crate::error::QuiltError;
use polars::prelude::*;
use regex;

pub fn contains(
    df: &LazyFrame,
    colname: &str,
    pattern: &str,
    ignorecase: bool,
) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("contains", Some(colname), e.to_string()))?;

    if !schema.iter_names().any(|s| s == colname) {
        return Err(QuiltError::schema(
            "contains",
            Some(colname),
            "column not found",
        ));
    }

    LogController::debug(&format!(
        "Applying contains: column={colname} pattern='{pattern}' ignorecase={ignorecase}"
    ));

    // Use Polars' native string operations for better performance
    let expr = if ignorecase {
        // For case-insensitive search, use regex with (?i) flag
        let pattern_regex = format!("(?i){}", regex::escape(pattern));
        col(colname)
            .cast(DataType::String)
            .str()
            .contains(lit(pattern_regex), false) // literal=false for regex
    } else {
        // For case-sensitive search, use literal contains
        col(colname)
            .cast(DataType::String)
            .str()
            .contains(lit(pattern), true) // literal=true for exact string match
    };

    Ok(df.clone().filter(expr))
}
