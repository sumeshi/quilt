use polars::prelude::*;

/// Encode exactly one JSON value per cell without changing the input frame.
pub(crate) fn json_expr(column: Expr) -> Expr {
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

/// Search nested values through their JSON representation, retaining the
/// original typed columns in the filtered frame.
pub(crate) fn search_expr(name: &str, dtype: &DataType) -> Expr {
    let column = col(name);
    if matches!(dtype, DataType::Struct(_) | DataType::List(_)) {
        json_expr(column)
    } else {
        column.cast(DataType::String)
    }
}
