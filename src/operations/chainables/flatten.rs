use crate::error::QuiltError;
use polars::prelude::*;
use std::collections::HashSet;

fn leaf_expr(expr: Expr, dtype: &DataType, prefix: &str, output: &mut Vec<(Expr, String)>) {
    if let DataType::Struct(fields) = dtype {
        for field in fields {
            let path = format!("{prefix}.{}", field.name);
            leaf_expr(
                expr.clone().struct_().field_by_name(&field.name),
                &field.dtype,
                &path,
                output,
            );
        }
    } else {
        output.push((expr.alias(prefix), prefix.to_string()));
    }
}

/// Lazy struct flattening. Schema inspection is metadata-only; field extraction
/// remains in the logical plan and executes at the eventual sink/finalizer.
pub fn flatten(df: &LazyFrame) -> Result<LazyFrame, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("flatten", None::<String>, error.to_string()))?;
    let mut output = Vec::new();
    let mut names = HashSet::new();
    for field in schema.iter_fields() {
        if matches!(field.dtype(), DataType::Struct(_)) {
            let mut leaves = Vec::new();
            let field_name = field.name().to_string();
            leaf_expr(
                col(field_name.clone()),
                field.dtype(),
                &field_name,
                &mut leaves,
            );
            for (expression, name) in leaves {
                if !names.insert(name.clone())
                    || (schema.get(&name).is_some() && name != field.name().as_str())
                {
                    return Err(QuiltError::schema(
                        "flatten",
                        Some(name),
                        "output column collides with an existing column",
                    ));
                }
                output.push(expression);
            }
        } else {
            names.insert(field.name().to_string());
            output.push(col(field.name().to_string()));
        }
    }
    Ok(df.clone().select(output))
}
