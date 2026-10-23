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

    LogController::debug("Applying contains");
    let searchable = crate::controllers::nested::search_expr(colname, schema.get(colname).unwrap());

    // Use Polars' native string operations for better performance
    let expr = if ignorecase {
        // For case-insensitive search, use regex with (?i) flag
        let pattern_regex = format!("(?i){}", regex::escape(pattern));
        searchable.str().contains(lit(pattern_regex), true) // strict=true: regex compilation errors propagate
    } else {
        // For case-sensitive search, use literal contains
        searchable.str().contains_literal(lit(pattern))
    };

    Ok(df.clone().filter(expr))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_sensitive_contains_treats_regex_metacharacters_literally() {
        let df =
            DataFrame::new_infer_height(vec![
                Series::new("value".into(), ["a.b", "axb", "a[b"]).into_column()
            ])
            .unwrap()
            .lazy();
        let result = contains(&df, "value", ".", false)
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(result.height(), 1);
        assert_eq!(
            result.column("value").unwrap().get(0).unwrap(),
            AnyValue::String("a.b")
        );
        let bracket = contains(&df, "value", "[", false)
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(bracket.height(), 1);
        assert_eq!(
            bracket.column("value").unwrap().get(0).unwrap(),
            AnyValue::String("a[b")
        );
    }
}
