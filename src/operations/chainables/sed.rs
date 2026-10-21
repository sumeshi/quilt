use crate::controllers::log::LogController;
use crate::controllers::nested::literal_col;
use crate::error::QuiltError;
use polars::prelude::*;

pub fn sed(
    df: &LazyFrame,
    colname: Option<&str>,
    pattern: &str,
    replacement: &str,
    ignorecase: bool,
) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema("sed", colname, e.to_string()))?;

    let final_pattern = if ignorecase {
        format!("(?i){pattern}") // Prepend (?i) flag for case-insensitivity
    } else {
        pattern.to_string()
    };

    match colname {
        Some(col) => {
            // Apply sed to specific column
            if !schema.iter_names().any(|s| s == col) {
                return Err(QuiltError::schema("sed", Some(col), "column not found"));
            }
            if matches!(
                schema.get(col),
                Some(DataType::Struct(_) | DataType::List(_))
            ) {
                return Err(QuiltError::schema("sed", Some(col), "nested values cannot be replaced as text; use flatten and select a scalar field"));
            }
            LogController::debug("Replacing values in selected column");
            let replace_expr = literal_col(col)
                .cast(DataType::String) // Ensure the column is String
                .str()
                .replace_all(lit(final_pattern), lit(replacement.to_string()), false) // literal: false for regex
                .alias(col);
            Ok(df.clone().with_column(replace_expr))
        }
        None => {
            // Apply sed to all columns
            LogController::debug("Replacing values in all columns");
            let mut result_df = df.clone();
            // The implicit all-column form is text-only so numeric, boolean,
            // and temporal columns keep their original physical and logical
            // types. An explicit target may still be intentionally stringified.
            for (column_name, dtype) in schema.iter() {
                if !matches!(dtype, DataType::String) {
                    continue;
                }
                let col_str = column_name.as_str();
                let replace_expr = literal_col(col_str)
                    .cast(DataType::String) // Ensure the column is String
                    .str()
                    .replace_all(
                        lit(final_pattern.clone()),
                        lit(replacement.to_string()),
                        false,
                    ) // literal: false for regex
                    .alias(col_str);
                result_df = result_df.with_column(replace_expr);
            }
            Ok(result_df)
        }
    }
}
