use crate::controllers::log::LogController;
use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Table};
use polars::prelude::*;

const MAX_DISPLAY_ROWS: usize = 8;
const MAX_DISPLAY_WIDTH: usize = 40;

pub fn showtable(df: &LazyFrame) -> Result<FinalizerResult, QuiltError> {
    LogController::debug("Applying showtable (display DataFrame as a formatted table)");

    Ok(FinalizerResult::PlanTable(format!(
        "{}\n",
        render_table(df)?
    )))
}

pub fn render_table(df: &LazyFrame) -> Result<String, QuiltError> {
    // Try to estimate the size using limit + head approach to avoid full collection
    let head_df = match df.clone().limit((MAX_DISPLAY_ROWS + 1) as u32).collect() {
        Ok(df) => df,
        Err(e) => {
            return Err(QuiltError::operation(
                "showtable",
                format!("Error: Failed to collect DataFrame for showtable: {e}"),
            ));
        }
    };

    let is_truncated = head_df.height() > MAX_DISPLAY_ROWS;
    let display_df = if is_truncated {
        // If we have more rows than display limit, take only the first MAX_DISPLAY_ROWS
        head_df.slice(0, MAX_DISPLAY_ROWS)
    } else {
        head_df
    };

    let shape = display_df.shape();
    let colnames: Vec<String> = display_df
        .get_column_names_owned()
        .into_iter()
        .map(|s| s.to_string())
        .collect();

    // Display table size information
    let mut output = String::new();
    if is_truncated {
        output.push_str(&format!(
            "shape: ({}+, {}) [showing first {} rows]",
            shape.0, shape.1, MAX_DISPLAY_ROWS
        ));
    } else {
        output.push_str(&format!("shape: ({}, {})", shape.0, shape.1));
    }
    output.push('\n');

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    let header_cells: Vec<Cell> = colnames.iter().map(Cell::new).collect();
    table.set_header(header_cells);

    // Add data rows
    for row_idx in 0..shape.0 {
        let mut row_cells = Vec::new();
        for col_name in &colnames {
            let val_result = display_df
                .column(col_name)
                .map_err(|error| {
                    QuiltError::schema("showtable", Some(col_name), error.to_string())
                })?
                .get(row_idx);
            let cell_content = match val_result {
                Ok(val) => truncate_display(&format_anyvalue(&val)),
                Err(_) => "Error".to_string(),
            };
            row_cells.push(Cell::new(cell_content));
        }
        table.add_row(row_cells);
    }

    // Add truncation indicator if needed
    if is_truncated {
        let mut truncation_row = Vec::new();
        for _ in &colnames {
            truncation_row.push(Cell::new("⋮"));
        }
        table.add_row(truncation_row);
    }

    output.push_str(&table.to_string());
    Ok(output)
}

fn format_anyvalue(val: &AnyValue) -> String {
    match val {
        AnyValue::Null => "null".to_string(),
        AnyValue::Boolean(b) => b.to_string(),
        AnyValue::String(s) => s.to_string(),
        AnyValue::Int8(i) => i.to_string(),
        AnyValue::Int16(i) => i.to_string(),
        AnyValue::Int32(i) => i.to_string(),
        AnyValue::Int64(i) => i.to_string(),
        AnyValue::UInt8(i) => i.to_string(),
        AnyValue::UInt16(i) => i.to_string(),
        AnyValue::UInt32(i) => i.to_string(),
        AnyValue::UInt64(i) => i.to_string(),
        AnyValue::Float32(f) => f.to_string(),
        AnyValue::Float64(f) => f.to_string(),
        AnyValue::Date(d) => d.to_string(),
        AnyValue::Datetime(dt, _, _) => dt.to_string(),
        AnyValue::Time(t) => t.to_string(),
        AnyValue::Duration(d, _) => d.to_string(),
        _ => format!("{val}"),
    }
}

fn truncate_display(value: &str) -> String {
    if value.chars().count() <= MAX_DISPLAY_WIDTH {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(MAX_DISPLAY_WIDTH - 1)
        .collect::<String>();
    truncated.push('…');
    truncated
}
