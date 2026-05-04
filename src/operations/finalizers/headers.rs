use crate::controllers::log::LogController;
#[cfg(feature = "table")]
use comfy_table::presets::UTF8_FULL;
#[cfg(feature = "table")]
use comfy_table::{Cell, Table};
use polars::prelude::*;

pub fn headers(df: &LazyFrame, plain: bool) {
    match render_headers(df, plain) {
        Ok(output) => print!("{output}"),
        Err(e) => eprintln!("{e}"),
    }
}

pub fn render_headers(df: &LazyFrame, plain: bool) -> Result<String, String> {
    // Get schema from LazyFrame without collecting
    let schema = match df.clone().collect_schema() {
        Ok(schema) => schema,
        Err(e) => {
            return Err(format!("Error getting schema: {e}"));
        }
    };

    let column_names: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();
    LogController::debug(&format!("Showing headers: {} columns", column_names.len()));

    if plain {
        Ok(column_names.join("\n") + "\n")
    } else {
        #[cfg(feature = "table")]
        {
            let mut table = Table::new();
            table.load_preset(UTF8_FULL);
            table.set_header(vec!["#", "Column Name"]);
            for (i, name) in column_names.iter().enumerate() {
                table.add_row(vec![Cell::new(format!("{i:02}")), Cell::new(name)]);
            }
            Ok(format!("{table}\n"))
        }

        #[cfg(not(feature = "table"))]
        {
            let mut output = String::from("#\tColumn Name\n");
            for (i, name) in column_names.iter().enumerate() {
                output.push_str(&format!("{i:02}\t{name}\n"));
            }
            Ok(output)
        }
    }
}
