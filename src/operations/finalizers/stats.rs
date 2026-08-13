use crate::controllers::log::LogController;
use crate::error::QuiltError;
use crate::operations::finalizers::FinalizerResult;
use comfy_table::{presets::UTF8_FULL, Cell, Color, Table};
use polars::prelude::*;

pub fn stats(df: &LazyFrame) -> Result<FinalizerResult, QuiltError> {
    LogController::debug("Calculating statistics for DataFrame using lazy evaluation");

    // Get schema to understand the columns and their types
    let schema = df.clone().collect_schema().map_err(|e| {
        QuiltError::schema(
            "stats",
            None::<String>,
            format!("Error: Failed to get DataFrame schema: {e}"),
        )
    })?;

    let column_names: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();

    // Statistics are one explicit aggregate barrier. The result is rendered
    // only after this single-row aggregate has completed.
    let stats_data = calculate_stats_lazy(df, &column_names, &schema).map_err(|error| {
        QuiltError::operation("stats", format!("failed to evaluate statistics: {error}"))
    })?;
    {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL);

        let mut header_cells = vec![Cell::new("Statistic").fg(Color::Green)];
        for name in &column_names {
            header_cells.push(Cell::new(name).fg(Color::Green));
        }
        table.set_header(header_cells);

        table.add_row(build_stat_row("count", &stats_data.counts));
        table.add_row(build_stat_row("null_count", &stats_data.null_counts));
        table.add_row(build_stat_row("datatype", &stats_data.dtypes));
        table.add_row(build_stat_row("mean", &stats_data.means));
        table.add_row(build_stat_row("std", &stats_data.stds));
        table.add_row(build_stat_row("min", &stats_data.mins));
        table.add_row(build_stat_row("25%", &stats_data.p25s));
        table.add_row(build_stat_row("50% (median)", &stats_data.p50s));
        table.add_row(build_stat_row("75%", &stats_data.p75s));
        table.add_row(build_stat_row("max", &stats_data.maxs));

        Ok(FinalizerResult::Stdout(format!("{table}\n")))
    }
}

struct StatsData {
    counts: Vec<String>,
    null_counts: Vec<String>,
    dtypes: Vec<String>,
    means: Vec<String>,
    stds: Vec<String>,
    mins: Vec<String>,
    maxs: Vec<String>,
    p25s: Vec<String>,
    p50s: Vec<String>,
    p75s: Vec<String>,
}

fn calculate_stats_lazy(
    df: &LazyFrame,
    column_names: &[String],
    schema: &Schema,
) -> Result<StatsData, Box<dyn std::error::Error>> {
    let mut stats_data = StatsData {
        counts: Vec::new(),
        null_counts: Vec::new(),
        dtypes: Vec::new(),
        means: Vec::new(),
        stds: Vec::new(),
        mins: Vec::new(),
        maxs: Vec::new(),
        p25s: Vec::new(),
        p50s: Vec::new(),
        p75s: Vec::new(),
    };

    // Separate numeric and string columns for aggregate processing.
    let column_count = column_names.len();
    let mut numeric_cols = Vec::with_capacity(column_count);
    let mut string_cols = Vec::with_capacity(column_count);

    for col_name in column_names {
        let dtype = schema
            .get(col_name)
            .ok_or_else(|| format!("column '{col_name}' disappeared from the collected schema"))?;
        stats_data.dtypes.push(dtype.to_string());

        if is_numeric_dtype(dtype) {
            numeric_cols.push(col_name.as_str());
        } else if dtype == &DataType::String {
            string_cols.push(col_name.as_str());
        }
    }

    // Get basic statistics in one batch operation
    // Estimate capacity: 1 total_count + column_count null_counts + numeric stats + string stats
    let estimated_expr_count =
        1 + column_count + (numeric_cols.len() * 7) + (string_cols.len() * 2);
    let mut basic_exprs = Vec::with_capacity(estimated_expr_count);
    basic_exprs.push(len().alias("total_count"));

    // Add null count for all columns
    for col_name in column_names {
        basic_exprs.push(
            col(col_name)
                .null_count()
                .alias(format!("null_count_{col_name}")),
        );
    }

    // Add numeric statistics for numeric columns
    for col_name in &numeric_cols {
        basic_exprs.extend([
            col(*col_name).mean().alias(format!("mean_{col_name}")),
            col(*col_name).std(1).alias(format!("std_{col_name}")),
            col(*col_name).min().alias(format!("min_{col_name}")),
            col(*col_name).max().alias(format!("max_{col_name}")),
            col(*col_name)
                .quantile(lit(0.25), QuantileMethod::Linear)
                .alias(format!("p25_{col_name}")),
            col(*col_name)
                .quantile(lit(0.50), QuantileMethod::Linear)
                .alias(format!("p50_{col_name}")),
            col(*col_name)
                .quantile(lit(0.75), QuantileMethod::Linear)
                .alias(format!("p75_{col_name}")),
        ]);
    }

    // Add string statistics for string columns
    for col_name in &string_cols {
        basic_exprs.extend([
            col(*col_name).min().alias(format!("min_{col_name}")),
            col(*col_name).max().alias(format!("max_{col_name}")),
        ]);
    }

    // Execute all statistics in a single batch operation
    let stats_df = df.clone().select(basic_exprs).collect()?;

    // Extract total count
    let total_count = stats_df
        .column("total_count")?
        .get(0)?
        .try_extract::<i64>()
        .unwrap_or(0);

    // Process results for each column
    for col_name in column_names {
        let dtype = schema
            .get(col_name)
            .ok_or_else(|| format!("column '{col_name}' disappeared from the collected schema"))?;
        stats_data.counts.push(total_count.to_string());

        // Extract null count
        let null_count = stats_df
            .column(&format!("null_count_{col_name}"))?
            .get(0)?
            .try_extract::<u32>()
            .unwrap_or(0);
        stats_data.null_counts.push(null_count.to_string());

        if is_numeric_dtype(dtype) {
            // Extract numeric statistics
            stats_data.means.push(format_numeric_stat(
                stats_df.column(&format!("mean_{col_name}"))?.get(0)?,
            ));
            stats_data.stds.push(format_numeric_stat(
                stats_df.column(&format!("std_{col_name}"))?.get(0)?,
            ));
            stats_data.mins.push(format_numeric_stat(
                stats_df.column(&format!("min_{col_name}"))?.get(0)?,
            ));
            stats_data.maxs.push(format_numeric_stat(
                stats_df.column(&format!("max_{col_name}"))?.get(0)?,
            ));
            stats_data.p25s.push(format_numeric_stat(
                stats_df.column(&format!("p25_{col_name}"))?.get(0)?,
            ));
            stats_data.p50s.push(format_numeric_stat(
                stats_df.column(&format!("p50_{col_name}"))?.get(0)?,
            ));
            stats_data.p75s.push(format_numeric_stat(
                stats_df.column(&format!("p75_{col_name}"))?.get(0)?,
            ));
        } else if dtype == &DataType::String {
            // Extract string statistics
            stats_data.means.push("-".to_string());
            stats_data.stds.push("-".to_string());
            stats_data.mins.push(format_string_stat(
                stats_df.column(&format!("min_{col_name}"))?.get(0)?,
            ));
            stats_data.maxs.push(format_string_stat(
                stats_df.column(&format!("max_{col_name}"))?.get(0)?,
            ));
            stats_data.p25s.push("-".to_string());
            stats_data.p50s.push("-".to_string());
            stats_data.p75s.push("-".to_string());
        } else {
            // For other types, fill with dashes
            stats_data.means.push("-".to_string());
            stats_data.stds.push("-".to_string());
            stats_data.mins.push("-".to_string());
            stats_data.maxs.push("-".to_string());
            stats_data.p25s.push("-".to_string());
            stats_data.p50s.push("-".to_string());
            stats_data.p75s.push("-".to_string());
        }
    }

    Ok(stats_data)
}

fn is_numeric_dtype(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    )
}

fn format_numeric_stat(val: AnyValue) -> String {
    match val {
        AnyValue::Null => "-".to_string(),
        AnyValue::Float64(f) => format!("{f:.4}"),
        AnyValue::Float32(f) => format!("{f:.4}"),
        _ => val.to_string(),
    }
}

fn format_string_stat(val: AnyValue) -> String {
    match val {
        AnyValue::Null => "-".to_string(),
        _ => val.to_string(),
    }
}

fn build_stat_row(stat_name: &str, values: &[String]) -> Vec<Cell> {
    let mut row = vec![Cell::new(stat_name)];
    for value in values {
        row.push(Cell::new(value));
    }
    row
}
