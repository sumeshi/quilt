use crate::error::QuiltError;
use polars::prelude::*;

pub fn timeinfo(df: &LazyFrame, column: &str, by: &[String]) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("timeinfo", None::<String>, error.to_string()))?;
    match schema.get(column) {
        Some(DataType::Datetime(_, _)) => {}
        Some(_) => {
            return Err(QuiltError::schema(
                "timeinfo",
                Some(column),
                "expected Datetime; use cast <column> datetime first",
            ))
        }
        None => {
            return Err(QuiltError::schema(
                "timeinfo",
                Some(column),
                "column not found",
            ))
        }
    }
    for (index, name) in by.iter().enumerate() {
        if schema.get(name).is_none() {
            return Err(QuiltError::schema(
                "timeinfo",
                Some(name),
                "group column not found",
            ));
        }
        if by[..index].contains(name)
            || [
                "rows",
                "valid_times",
                "null_times",
                "first_seen",
                "last_seen",
            ]
            .contains(&name.as_str())
        {
            return Err(QuiltError::usage(format!(
                "duplicate or reserved timeinfo group column '{name}'"
            )));
        }
    }
    let aggregates = [
        len().alias("rows"),
        col(column).count().alias("valid_times"),
        col(column).null_count().alias("null_times"),
        col(column).min().alias("first_seen"),
        col(column).max().alias("last_seen"),
    ];
    Ok(if by.is_empty() {
        df.clone().select(aggregates)
    } else {
        df.clone()
            .group_by(by.iter().map(col).collect::<Vec<_>>())
            .agg(aggregates)
    })
}
