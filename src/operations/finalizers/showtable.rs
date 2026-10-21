use crate::controllers::log::LogController;
use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, ContentArrangement, Table};
use polars::prelude::*;
use unicode_width::UnicodeWidthStr;

const MAX_DISPLAY_ROWS: usize = 8;
const MAX_DISPLAY_WIDTH: usize = 64;
const MAX_DISPLAY_COLUMNS: usize = 8;

pub fn showtable(df: &LazyFrame) -> Result<FinalizerResult, QuiltError> {
    LogController::debug("Applying showtable (display DataFrame as a formatted table)");

    Ok(FinalizerResult::PlanTable(format!(
        "{}\n",
        render_table(df)?
    )))
}

pub fn render_table(df: &LazyFrame) -> Result<String, QuiltError> {
    let flattened = crate::operations::chainables::flatten::flatten(df)?;
    let schema = flattened
        .clone()
        .collect_schema()
        .map_err(|error| QuiltError::operation("showtable", error.to_string()))?;
    let total_columns = schema.len();
    let columns: Vec<Expr> = schema
        .iter_names()
        .take(MAX_DISPLAY_COLUMNS)
        .map(|name| crate::controllers::nested::literal_col(name.as_str()))
        .collect();
    let preview = super::csv::table_frame(&flattened.select(columns), "showtable")?;
    // Try to estimate the size using limit + head approach to avoid full collection
    let head_df = match preview.limit((MAX_DISPLAY_ROWS + 1) as u32).collect() {
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
            shape.0, total_columns, MAX_DISPLAY_ROWS
        ));
    } else {
        output.push_str(&format!("shape: ({}, {})", shape.0, total_columns));
    }
    if total_columns > MAX_DISPLAY_COLUMNS {
        output.push_str(&format!(" [showing first {MAX_DISPLAY_COLUMNS} columns; {} columns omitted; use select to choose columns]", total_columns - MAX_DISPLAY_COLUMNS));
    }
    output.push_str(" [cells/headers limited to 64 display columns; … marks truncation]");
    output.push('\n');

    let mut table = Table::new();
    table.load_style(UTF8_FULL);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    let header_cells: Vec<Cell> = colnames
        .iter()
        .map(|name| Cell::new(truncate_display(name)))
        .collect();
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
                Err(error) => return Err(QuiltError::operation("showtable", error.to_string())),
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
        AnyValue::StringOwned(s) => s.to_string(),
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
        _ => format!("{val}"),
    }
}

fn truncate_display(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        let token = match character {
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            c if c.is_control()
                || matches!(c, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}') =>
            {
                c.escape_unicode().to_string()
            }
            c => c.to_string(),
        };
        escaped.push_str(&token);
        if escaped.width() > MAX_DISPLAY_WIDTH {
            while escaped.width() >= MAX_DISPLAY_WIDTH {
                escaped.pop();
            }
            escaped.push('…');
            break;
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_cells_use_human_units() {
        assert_eq!(format_anyvalue(&AnyValue::Date(0)), "1970-01-01");
        assert_eq!(
            format_anyvalue(&AnyValue::Time(3_600_000_000_000)),
            "01:00:00"
        );
        assert_eq!(
            format_anyvalue(&AnyValue::Duration(60_000_000, TimeUnit::Microseconds)),
            "1m"
        );
        assert_eq!(format_anyvalue(&AnyValue::Null), "null");
    }

    #[test]
    fn display_escapes_controls_and_bounds_unicode_width() {
        let text = truncate_display("日本語😀\n\t\r\u{1b}[31m\u{202e}");
        assert!(text.contains("日本語😀\\n\\t\\r"));
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{202e}'));
        let long = truncate_display(&"漢😀".repeat(100_000));
        assert!(long.ends_with('…'));
        assert!(long.width() <= MAX_DISPLAY_WIDTH);
        assert_eq!(
            truncate_display(&"a".repeat(MAX_DISPLAY_WIDTH)),
            "a".repeat(MAX_DISPLAY_WIDTH)
        );
    }
}
