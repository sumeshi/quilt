use crate::controllers::log::LogController;
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
            LogController::debug(&format!(
                "Replacing values in '{col}' column using regex pattern '{pattern}' -> '{replacement}' (case-insensitive: {ignorecase})"
            ));
            let replace_expr = polars::prelude::col(col)
                .cast(DataType::String) // Ensure the column is String
                .str()
                .replace_all(lit(final_pattern), lit(replacement.to_string()), false) // literal: false for regex
                .alias(col);
            Ok(df.clone().with_column(replace_expr))
        }
        None => {
            // Apply sed to all columns
            LogController::debug(&format!(
                "Replacing values in all columns using regex pattern '{pattern}' -> '{replacement}' (case-insensitive: {ignorecase})"
            ));
            let mut result_df = df.clone();
            // Apply replacement to all columns
            for (column_name, _) in schema.iter() {
                let col_str = column_name.as_str();
                let replace_expr = polars::prelude::col(col_str)
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
