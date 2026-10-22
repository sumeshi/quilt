use polars::prelude::*;

/// Reference a schema name literally without `col`'s wildcard/regex selectors.
pub(crate) fn literal_col(name: &str) -> Expr {
    Expr::Column(name.into())
}

/// Encode exactly one JSON value per cell without changing the input frame.
pub(crate) fn json_expr(column: Expr, dtype: &DataType) -> Expr {
    let column = checked_json_column(column, dtype);
    let encoded = as_struct(vec![column.clone().alias("v")])
        .struct_()
        .json_encode()
        .str()
        .strip_prefix(lit("{\"v\":"))
        .str()
        .strip_suffix(lit("}"));
    when(column.is_null())
        .then(lit(NULL).cast(DataType::String))
        .otherwise(encoded)
}

/// Reject floating point values that JSON encoders would otherwise turn into
/// null. The dtype gate keeps ordinary columns (including empty structs) out
/// of the per value map and leaves evaluation lazy.
pub(crate) fn checked_json_column(column: Expr, dtype: &DataType) -> Expr {
    if !dtype_contains_float(dtype) {
        return column;
    }
    column.map(
        |column| {
            if series_has_non_finite(column.as_materialized_series())? {
                let name = column.name();
                return Err(PolarsError::ComputeError(
                    format!("non-finite float in column '{name}' cannot be represented in JSON")
                        .into(),
                ));
            }
            Ok(column)
        },
        |_, field| Ok(field.clone()),
    )
}

fn dtype_contains_float(dtype: &DataType) -> bool {
    match dtype {
        DataType::Float16 | DataType::Float32 | DataType::Float64 => true,
        DataType::List(inner) => dtype_contains_float(inner),
        DataType::Struct(fields) => fields
            .iter()
            .any(|field| dtype_contains_float(field.dtype())),
        _ => false,
    }
}

fn series_has_non_finite(series: &Series) -> PolarsResult<bool> {
    for row in 0..series.len() {
        if value_has_non_finite(series.get(row)?) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn value_has_non_finite(value: AnyValue<'_>) -> bool {
    match value {
        AnyValue::Float16(value) => !value.is_finite(),
        AnyValue::Float32(value) => !value.is_finite(),
        AnyValue::Float64(value) => !value.is_finite(),
        AnyValue::List(values) => (0..values.len())
            .any(|index| values.get(index).map(value_has_non_finite).unwrap_or(false)),
        AnyValue::Struct(_, _, _) => value._iter_struct_av().any(value_has_non_finite),
        AnyValue::StructOwned(values) => values.0.iter().cloned().any(value_has_non_finite),
        _ => false,
    }
}

/// Search nested values through their JSON representation, retaining the
/// original typed columns in the filtered frame.
pub(crate) fn search_expr(name: &str, dtype: &DataType) -> Expr {
    let column = literal_col(name);
    if matches!(dtype, DataType::Struct(_) | DataType::List(_)) {
        json_expr(column, dtype)
    } else {
        column.cast(DataType::String)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_json_search_rejects_non_finite_values_and_keeps_finite_json() {
        let values = Series::new("value".into(), &[1.0f64, f64::INFINITY]);
        let list = Series::new("items".into(), &[Some(values)]);
        let dtype = list.dtype().clone();
        let frame = DataFrame::new_infer_height(vec![list.into()]).unwrap();
        let error = frame
            .clone()
            .lazy()
            .select([search_expr("items", &dtype).alias("items")])
            .collect()
            .unwrap_err();
        assert!(error.to_string().contains("non-finite float"));

        let finite = Series::new(
            "items".into(),
            &[Some(Series::new(PlSmallStr::EMPTY, &[1.0f64]))],
        );
        let dtype = finite.dtype().clone();
        let output = DataFrame::new_infer_height(vec![finite.into()])
            .unwrap()
            .lazy()
            .select([search_expr("items", &dtype).alias("items")])
            .collect()
            .unwrap();
        assert_eq!(
            output.column("items").unwrap().str().unwrap().get(0),
            Some("[1.0]")
        );
    }
}
