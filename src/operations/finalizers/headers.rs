use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Table};
use polars::prelude::*;

pub fn headers(df: &LazyFrame, plain: bool) -> Result<FinalizerResult, QuiltError> {
    let schema = df
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::schema("headers", None::<String>, error.to_string()))?;
    let names = schema
        .iter_names()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if plain {
        Ok(FinalizerResult::Stdout(format!("{}\n", names.join("\n"))))
    } else {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL);
        table.set_header(vec!["#", "Column Name"]);
        for (index, name) in names.iter().enumerate() {
            table.add_row(vec![
                Cell::new(format!("{:02}", index + 1)),
                Cell::new(name),
            ]);
        }
        Ok(FinalizerResult::Stdout(format!("{table}\n")))
    }
}
