use crate::error::QuiltError;
use polars::prelude::*;
use regex::Regex;

pub fn extract(df: &LazyFrame, column: &str, pattern: &str) -> Result<LazyFrame, QuiltError> {
    let regex = Regex::new(pattern).map_err(|error| {
        QuiltError::conversion(
            "extract",
            Some(column),
            format!("Invalid extract regex: {error}"),
        )
    })?;
    let group_names = regex
        .capture_names()
        .flatten()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if group_names.is_empty() {
        return Err(QuiltError::conversion(
            "extract",
            Some(column),
            "regex must contain at least one named capture group",
        ));
    }

    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("extract", None::<String>, error.to_string()))?;
    let source_dtype = schema
        .get(column)
        .ok_or_else(|| QuiltError::schema("extract", Some(column), "column not found"))?;
    if source_dtype != &DataType::String {
        return Err(QuiltError::schema(
            "extract",
            Some(column),
            format!("column must be string, found {}", source_dtype),
        ));
    }
    for group_name in &group_names {
        if schema.get(group_name).is_some() {
            return Err(QuiltError::schema(
                "extract",
                Some(group_name),
                "output column already exists",
            ));
        }
    }

    let expressions = group_names.into_iter().map(|group_name| {
        let regex = regex.clone();
        let capture_name = group_name.clone();
        col(column)
            .map(
                move |series| {
                    let values = series
                        .str()?
                        .into_iter()
                        .map(|value| {
                            value.and_then(|value| {
                                regex.captures(value).and_then(|captures| {
                                    captures
                                        .name(&capture_name)
                                        .map(|capture| capture.as_str().to_string())
                                })
                            })
                        })
                        .collect::<Vec<_>>();
                    Ok(Some(Column::from(Series::new(
                        capture_name.clone().into(),
                        values,
                    ))))
                },
                GetOutput::from_type(DataType::String),
            )
            .alias(group_name)
    });
    Ok(df.clone().with_columns(expressions.collect::<Vec<_>>()))
}
