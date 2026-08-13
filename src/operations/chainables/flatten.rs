use polars::prelude::*;
use std::collections::HashSet;

fn fail(message: impl AsRef<str>) -> ! {
    eprintln!("Error: {}", message.as_ref());
    std::process::exit(1);
}

fn field_with_parent_validity(parent: &StructChunked, field: Series) -> PolarsResult<Series> {
    // Struct child arrays can retain values in rows where the parent struct is null.
    // Wrapping the child and combining the outer validity makes those rows null too.
    let mut wrapper =
        StructChunked::from_series(parent.name().clone(), parent.len(), std::iter::once(&field))?;
    wrapper.zip_outer_validity(parent);
    wrapper
        .fields_as_series()
        .into_iter()
        .next()
        .ok_or_else(|| PolarsError::ComputeError("struct field disappeared".into()))
}

fn collect_leaves(
    parent: &StructChunked,
    prefix: &str,
    leaves: &mut Vec<Series>,
) -> PolarsResult<()> {
    for field in parent.fields_as_series() {
        let field = field_with_parent_validity(parent, field)?;
        let path = format!("{prefix}.{}", field.name());
        if matches!(field.dtype(), DataType::Struct(_)) {
            collect_leaves(field.struct_()?, &path, leaves)?;
        } else {
            leaves.push(field.with_name(path.into()));
        }
    }
    Ok(())
}

pub fn flatten(df: &LazyFrame) -> LazyFrame {
    let frame = df.clone().collect().unwrap_or_else(|error| {
        fail(format!("Failed to evaluate input before flatten: {error}"));
    });

    let original_names = frame
        .get_columns()
        .iter()
        .map(|column| column.name().to_string())
        .collect::<HashSet<_>>();
    let mut scalar_names = HashSet::new();
    let mut flattened = Vec::with_capacity(frame.width());
    let mut generated_names = HashSet::new();

    for column in frame.get_columns() {
        if matches!(column.dtype(), DataType::Struct(_)) {
            let mut leaves = Vec::new();
            collect_leaves(
                column.struct_().unwrap_or_else(|error| {
                    fail(format!(
                        "Cannot inspect struct column '{}': {error}",
                        column.name()
                    ));
                }),
                column.name(),
                &mut leaves,
            )
            .unwrap_or_else(|error| {
                fail(format!(
                    "Cannot flatten struct column '{}': {error}",
                    column.name()
                ));
            });
            for leaf in leaves {
                if !generated_names.insert(leaf.name().to_string()) {
                    fail(format!(
                        "Flatten output column '{}' would be generated more than once",
                        leaf.name()
                    ));
                }
                flattened.push(leaf);
            }
        } else {
            scalar_names.insert(column.name().to_string());
        }
    }

    for name in &generated_names {
        if scalar_names.contains(name) {
            fail(format!(
                "Flatten output column '{name}' collides with an existing column"
            ));
        }
    }

    // Preserve source-column order: each struct is replaced by its leaves in field order.
    let mut output = Vec::with_capacity(frame.width() + flattened.len());
    let mut flattened_index = 0;
    for column in frame.get_columns() {
        if matches!(column.dtype(), DataType::Struct(_)) {
            let count = count_leaves(column.struct_().unwrap_or_else(|error| {
                fail(format!(
                    "Cannot inspect struct column '{}': {error}",
                    column.name()
                ));
            }));
            output.extend(
                flattened[flattened_index..flattened_index + count]
                    .iter()
                    .cloned()
                    .map(Column::from),
            );
            flattened_index += count;
        } else {
            output.push(column.clone());
        }
    }

    // `original_names` intentionally participates in the preflight, making the collision
    // policy explicit even if Polars changes its duplicate-column handling in the future.
    if generated_names
        .iter()
        .any(|name| original_names.contains(name) && !scalar_names.contains(name))
    {
        fail("Flatten output contains a duplicate source path");
    }

    DataFrame::new(output)
        .unwrap_or_else(|error| fail(format!("Failed to build flattened frame: {error}")))
        .lazy()
}

fn count_leaves(parent: &StructChunked) -> usize {
    parent
        .fields_as_series()
        .into_iter()
        .map(|field| {
            if matches!(field.dtype(), DataType::Struct(_)) {
                count_leaves(field.struct_().expect("struct dtype checked"))
            } else {
                1
            }
        })
        .sum()
}
