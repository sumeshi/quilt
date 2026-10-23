use crate::controllers::nested::literal_col;
use crate::error::QuiltError;
use polars::prelude::*;
use regex::Regex;

pub fn validate(pattern: &str) -> Result<(), QuiltError> {
    let regex = Regex::new(pattern).map_err(|error| {
        QuiltError::conversion(
            "extract",
            None::<String>,
            format!("Invalid extract regex: {error}"),
        )
    })?;
    if regex.capture_names().flatten().next().is_none() {
        return Err(QuiltError::conversion(
            "extract",
            None::<String>,
            "regex must contain at least one named capture group",
        ));
    }
    Ok(())
}

pub fn extract(df: &LazyFrame, column: &str, pattern: &str) -> Result<LazyFrame, QuiltError> {
    validate(pattern)?;
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
        literal_col(column)
            .map(
                move |series| {
                    let values = series
                        .str()?
                        .iter()
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
                    Ok(Column::from(Series::new(
                        capture_name.clone().into(),
                        values,
                    )))
                },
                |_, field| Ok(Field::new(field.name().clone(), DataType::String)),
            )
            .alias(group_name)
    });
    Ok(df.clone().with_columns(expressions.collect::<Vec<_>>()))
}

#[cfg(test)]
mod validation_tests {
    #[test]
    fn extract_requires_valid_named_capture_syntax() {
        assert!(super::validate("[").is_err());
        assert!(super::validate("(plain)").is_err());
        assert!(super::validate("(?P<value>.+)").is_ok());
    }
}
