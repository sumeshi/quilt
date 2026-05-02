use crate::controllers::log::LogController;
use polars::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

pub fn partition(df: &LazyFrame, colname: &str, output_dir: &str) {
    // First, check if the column exists in the schema without collecting the DataFrame
    let schema = match df.clone().collect_schema() {
        Ok(schema) => schema,
        Err(e) => {
            eprintln!("Error getting schema for partition operation: {e}");
            std::process::exit(1);
        }
    };

    if schema.get(colname).is_none() {
        eprintln!("Error: Column '{colname}' not found in DataFrame for partition operation");
        std::process::exit(1);
    }

    LogController::debug(&format!(
        "Partitioning data by column '{colname}' into directory '{output_dir}'"
    ));

    // Create output directory if it doesn't exist
    let output_path = Path::new(output_dir);
    if let Err(e) = fs::create_dir_all(output_path) {
        eprintln!("Error creating output directory '{output_dir}': {e}");
        std::process::exit(1);
    }

    // Collect the DataFrame once
    let collected_df = match df.clone().collect() {
        Ok(df) => df,
        Err(e) => {
            eprintln!("Error collecting DataFrame for partition: {e}");
            std::process::exit(1);
        }
    };

    // Use partition_by for efficient grouping
    match collected_df.partition_by([colname], true) {
        Ok(groups) => {
            let num_groups = groups.len();
            LogController::info(&format!("Found {num_groups} unique groups to partition."));

            let mut files_created = 0;
            for mut group_df in groups {
                // The first value in the partition column determines the file name
                let value_any = match group_df.column(colname).and_then(|c| c.get(0)) {
                    Ok(val) => val,
                    Err(e) => {
                        eprintln!("Error: Could not get partition value: {e}");
                        continue;
                    }
                };

                let value_str = anyvalue_to_string(value_any);
                let safe_filename = sanitize_filename(&value_str);
                let output_file = output_path.join(format!("{safe_filename}.csv"));

                // Write the group DataFrame to a CSV file
                match write_csv_file(&mut group_df, &output_file) {
                    Ok(_) => {
                        files_created += 1;
                        LogController::info(&format!(
                            "Created partition file: {} ({} rows)",
                            output_file.display(),
                            group_df.height()
                        ));
                    }
                    Err(e) => {
                        LogController::error(&format!(
                            "Error writing partition file '{}': {}",
                            output_file.display(),
                            e
                        ));
                    }
                }
            }
            LogController::info(&format!(
                "Partition complete: {files_created} files created in '{output_dir}'"
            ));
        }
        Err(e) => {
            eprintln!("Error partitioning DataFrame: {e}");
            std::process::exit(1);
        }
    }
}

fn anyvalue_to_string(val: AnyValue) -> String {
    match val {
        AnyValue::Null => "null".to_string(),
        AnyValue::String(s) => s.to_string(),
        AnyValue::StringOwned(s) => s.to_string(),
        AnyValue::Boolean(b) => b.to_string(),
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
        _ => val.to_string(),
    }
}

fn sanitize_filename(filename: &str) -> String {
    // Replace invalid filename characters with underscores
    filename
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn write_csv_file(
    df: &mut DataFrame,
    output_path: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::create(output_path)?;
    CsvWriter::new(file)
        .include_header(true)
        .finish(df)
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(())
}
