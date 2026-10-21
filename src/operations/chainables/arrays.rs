//! Explicit operations for typed List columns.
use crate::controllers::nested::literal_col;
use crate::error::QuiltError;
use polars::prelude::*;

fn schema(df: &LazyFrame, op: &str) -> Result<SchemaRef, QuiltError> {
    df.clone()
        .collect_schema()
        .map_err(|e| QuiltError::schema(op, None::<String>, e.to_string()))
}
fn list_dtype<'a>(schema: &'a Schema, column: &str, op: &str) -> Result<&'a DataType, QuiltError> {
    match schema.get(column) {
        None => Err(QuiltError::schema(op, Some(column), "column not found")),
        Some(DataType::List(inner)) => Ok(inner),
        Some(t) => Err(QuiltError::schema(
            op,
            Some(column),
            format!("expected a list column, found {t}"),
        )),
    }
}
fn integral(t: &DataType) -> bool {
    matches!(
        t,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
    )
}
fn signed(t: &DataType) -> bool {
    matches!(
        t,
        DataType::Int8 | DataType::Int16 | DataType::Int32 | DataType::Int64
    )
}
fn scalar(value: &str, dtype: &DataType, column: &str) -> Result<Expr, QuiltError> {
    let bad = || {
        QuiltError::usage(format!(
            "array value '{value}' is invalid for list column '{column}' with element type {dtype}"
        ))
    };
    let e =
        match dtype {
            DataType::String => {
                if value.eq_ignore_ascii_case("null") {
                    lit(NULL).cast(DataType::String)
                } else {
                    let string_value = value
                        .strip_prefix('\'')
                        .and_then(|v| v.strip_suffix('\''))
                        .or_else(|| value.strip_prefix('"').and_then(|v| v.strip_suffix('"')))
                        .unwrap_or(value);
                    lit(string_value)
                }
            }
            DataType::Boolean => {
                if value.eq_ignore_ascii_case("null") {
                    lit(NULL).cast(DataType::Boolean)
                } else {
                    lit(value.parse::<bool>().map_err(|_| bad())?)
                }
            }
            d if integral(d) => {
                if value.eq_ignore_ascii_case("null") {
                    lit(NULL).cast(d.clone())
                } else if signed(d) {
                    let n = value.parse::<i64>().map_err(|_| bad())?;
                    let (min, max) = match d {
                        DataType::Int8 => (i8::MIN as i64, i8::MAX as i64),
                        DataType::Int16 => (i16::MIN as i64, i16::MAX as i64),
                        DataType::Int32 => (i32::MIN as i64, i32::MAX as i64),
                        _ => (i64::MIN, i64::MAX),
                    };
                    if n < min || n > max {
                        return Err(bad());
                    }
                    lit(n).cast(d.clone())
                } else {
                    let n = value.parse::<u64>().map_err(|_| bad())?;
                    let max = match d {
                        DataType::UInt8 => u8::MAX as u64,
                        DataType::UInt16 => u16::MAX as u64,
                        DataType::UInt32 => u32::MAX as u64,
                        _ => u64::MAX,
                    };
                    if n > max {
                        return Err(bad());
                    }
                    lit(n).cast(d.clone())
                }
            }
            DataType::Float32 | DataType::Float64 => {
                if value.eq_ignore_ascii_case("null") {
                    lit(NULL).cast(dtype.clone())
                } else {
                    let f = value.parse::<f64>().map_err(|_| bad())?;
                    if !f.is_finite() {
                        return Err(bad());
                    }
                    if matches!(dtype, DataType::Float32) {
                        let narrowed = f as f32;
                        if !narrowed.is_finite() {
                            return Err(bad());
                        }
                        lit(narrowed)
                    } else {
                        lit(f)
                    }
                }
            }
            DataType::Null => {
                if value.eq_ignore_ascii_case("null") {
                    lit(NULL)
                } else {
                    return Err(bad());
                }
            }
            DataType::List(_) | DataType::Struct(_) => return Err(QuiltError::schema(
                "array contains",
                Some(column),
                "nested object membership is unsupported; select or flatten a scalar field first",
            )),
            _ => {
                return Err(QuiltError::schema(
                    "array contains",
                    Some(column),
                    format!("unsupported list element type {dtype}"),
                ))
            }
        };
    Ok(e)
}

/// Keep rows whose list contains the parsed, typed scalar value. The string
/// `null` denotes a null list element; a quoted `"null"` denotes the string
/// value. A null list remains null in the result.
pub fn contains(df: &LazyFrame, column: &str, value: &str) -> Result<LazyFrame, QuiltError> {
    let s = schema(df, "array contains")?;
    let inner = list_dtype(&s, column, "array contains")?;
    let item = scalar(value, inner, column)?;
    Ok(df
        .clone()
        .filter(literal_col(column).list().contains(item, true)))
}

/// Add a scalar element from a list to a new column. Out-of-bounds indexes
/// yield null. Negative indexes count from the end as in Polars list.get.
pub fn get(
    df: &LazyFrame,
    column: &str,
    index: i64,
    output: &str,
) -> Result<LazyFrame, QuiltError> {
    let s = schema(df, "array get")?;
    let _ = list_dtype(&s, column, "array get")?;
    if s.get(output).is_some() {
        return Err(QuiltError::schema(
            "array get",
            Some(output),
            "output column already exists",
        ));
    }
    Ok(df.clone().with_columns([literal_col(column)
        .list()
        .get(lit(index), true)
        .alias(output)]))
}

/// Explicitly explode selected list columns. No operation expands lists
/// implicitly; callers must name each column to change row cardinality.
pub fn explode(df: &LazyFrame, columns: &[String]) -> Result<LazyFrame, QuiltError> {
    if columns.is_empty() {
        return Err(QuiltError::usage(
            "array explode requires at least one column",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if columns.iter().any(|column| !seen.insert(column.as_str())) {
        return Err(QuiltError::usage(
            "array explode column list contains a duplicate column",
        ));
    }
    let s = schema(df, "array explode")?;
    for c in columns {
        let _ = list_dtype(&s, c, "array explode")?;
    }
    let selected = cols(columns.iter().map(|c| c.as_str()));
    Ok(df.clone().explode(
        selected,
        ExplodeOptions {
            empty_as_null: true,
            keep_nulls: true,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_membership_distinguishes_null_from_text_null() {
        let mut lists = ListStringChunkedBuilder::new("items".into(), 2, 2);
        lists.append_trusted_len_iter([None].iter().copied());
        lists.append_values_iter(["null"].into_iter());
        let df = DataFrame::new_infer_height(vec![
            lists.finish().into_series().into_column(),
            Series::new("kind".into(), ["null_item", "text_item"]).into_column(),
        ])
        .unwrap()
        .lazy();

        let null_item = contains(&df, "items", "null").unwrap().collect().unwrap();
        let text_item = contains(&df, "items", "'null'").unwrap().collect().unwrap();
        assert_eq!(null_item.height(), 1);
        assert_eq!(text_item.height(), 1);
        assert_eq!(
            null_item.column("kind").unwrap().get(0).unwrap(),
            AnyValue::String("null_item")
        );
        assert_eq!(
            text_item.column("kind").unwrap().get(0).unwrap(),
            AnyValue::String("text_item")
        );
    }

    #[test]
    fn typed_membership_rejects_out_of_range_and_nested_values() {
        assert!(scalar("256", &DataType::UInt8, "items").is_err());
        assert!(scalar("[]", &DataType::List(Box::new(DataType::String)), "items").is_err());
        assert!(scalar("x", &DataType::Null, "items").is_err());
        assert!(scalar("null", &DataType::Null, "items").is_ok());
        assert!(scalar("3.5e38", &DataType::Float32, "items").is_err());
        assert!(scalar("1e38", &DataType::Float32, "items").is_ok());
    }

    #[test]
    fn get_and_explode_keep_their_null_and_empty_list_contract() {
        let mut lists =
            ListPrimitiveChunkedBuilder::<Int64Type>::new("items".into(), 3, 2, DataType::Int64);
        lists.append_slice(&[10, 20]);
        lists.append_slice(&[]);
        lists.append_null();
        let df = DataFrame::new_infer_height(vec![lists.finish().into_series().into_column()])
            .unwrap()
            .lazy();

        assert!(explode(&df, &[]).is_err());
        assert!(explode(&df, &["items".to_string(), "items".to_string()]).is_err());
        let scalar_df =
            DataFrame::new_infer_height(vec![Series::new("value".into(), [1i64]).into_column()])
                .unwrap()
                .lazy();
        assert!(explode(&scalar_df, &["value".to_string()]).is_err());

        let got = get(&df, "items", 4, "item").unwrap().collect().unwrap();
        assert_eq!(got.column("item").unwrap().get(0).unwrap(), AnyValue::Null);

        let exploded = explode(&df, &["items".to_string()])
            .unwrap()
            .collect()
            .unwrap();
        assert_eq!(exploded.height(), 4);
        assert_eq!(
            exploded.column("items").unwrap().get(2).unwrap(),
            AnyValue::Null
        );
        assert_eq!(
            exploded.column("items").unwrap().get(3).unwrap(),
            AnyValue::Null
        );
    }
}
