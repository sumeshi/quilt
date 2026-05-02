use crate::controllers::command::parse_batch_size;
use crate::controllers::dataframe::DataFrameController;
use crate::controllers::log::LogController;
use polars::prelude::{col, lit, JoinType, LazyFrame};
use serde::{Deserialize, Serialize};
use serde_yml::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
// Re-import operations to call them directly with LazyFrame
use crate::operations::chainables::{
    changetz, contains, convert, count, grep, head, isin, pivot, renamecol, sed, select, sort,
    tail, timeline, timeround, timeslice, uniq,
};
use crate::operations::finalizers::{
    dump as dump_op, dumpcache as dumpcache_op, headers as headers_op, partition as partition_op,
    show as show_op, showquery as showquery_op, showtable as showtable_op, stats as stats_op,
};
use crate::operations::initializers::load as load_op;
// Type alias for chainable operation functions
type ChainableOperation = fn(&LazyFrame, &Value) -> LazyFrame;
type FinalizerOperation = fn(&LazyFrame, &Value);
// Create a dispatch table for chainable operations
fn create_chainable_dispatch_table() -> HashMap<&'static str, ChainableOperation> {
    let mut table: HashMap<&'static str, ChainableOperation> = HashMap::new();
    table.insert("select", |df, args| {
        let colnames = if let Some(colnames_str) = get_string_from_value(args, "colnames") {
            colnames_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        } else if let Some(colnames_vec) = get_string_vec_from_value(args, "colnames") {
            colnames_vec
        } else {
            vec!["*".to_string()]
        };
        select::select(df, &colnames)
    });
    table.insert("isin", |df, args| {
        let colname = get_string_from_value(args, "colname").unwrap_or_default();
        let values = get_string_vec_from_value(args, "values").unwrap_or_default();
        isin::isin(df, &colname, &values)
    });
    table.insert("contains", |df, args| {
        let colname = get_string_from_value(args, "colname").unwrap_or_default();
        let pattern = get_string_from_value(args, "pattern").unwrap_or_default();
        let ignorecase = get_bool_from_value(args, "ignorecase");
        contains::contains(df, &colname, &pattern, ignorecase)
    });
    table.insert("sed", |df, args| {
        let colname = get_string_from_value(args, "colname");
        let pattern = get_string_from_value(args, "pattern").unwrap_or_default();
        let replacement = get_string_from_value(args, "replacement").unwrap_or_default();
        let ignorecase = get_bool_from_value(args, "ignorecase");
        sed::sed(df, colname.as_deref(), &pattern, &replacement, ignorecase)
    });
    table.insert("grep", |df, args| {
        let pattern = get_string_from_value(args, "pattern").unwrap_or_default();
        let ignorecase = get_bool_from_value(args, "ignorecase");
        let is_inverted = get_bool_from_value(args, "invert_match");
        grep::grep(df, &pattern, ignorecase, is_inverted)
    });
    table.insert("head", |df, args| {
        let n = get_usize_from_value(args, "number")
            .or_else(|| args.as_u64().and_then(|u| usize::try_from(u).ok()))
            .unwrap_or(5);
        head::head(df, n)
    });
    table.insert("tail", |df, args| {
        let n = get_usize_from_value(args, "number")
            .or_else(|| args.as_u64().and_then(|u| usize::try_from(u).ok()))
            .unwrap_or(5);
        tail::tail(df, n)
    });
    table.insert("sort", |df, args| {
        let colnames = if let Some(colnames_str) = get_string_from_value(args, "colnames") {
            colnames_str
                .split(',')
                .map(|s| s.trim().to_string())
                .collect()
        } else if let Some(colnames_vec) = get_string_vec_from_value(args, "colnames") {
            colnames_vec
        } else {
            vec!["*".to_string()]
        };
        let desc = get_bool_from_value(args, "desc");
        sort::sort(df, &colnames, desc)
    });
    table.insert("count", |df, _args| count::count(df));
    table.insert("uniq", |df, _args| uniq::uniq(df));
    table.insert("changetz", |df, args| {
        let colname = get_string_from_value(args, "colname").unwrap_or_default();
        let from_tz = get_string_from_value(args, "from-tz").unwrap_or_default();
        let to_tz = get_string_from_value(args, "to-tz").unwrap_or_default();
        let input_format = get_string_from_value(args, "input_format")
            .or_else(|| get_string_from_value(args, "input-format"))
            .or_else(|| get_string_from_value(args, "format"));
        let output_format = get_string_from_value(args, "output_format")
            .or_else(|| get_string_from_value(args, "output-format"));
        let ambiguous = get_string_from_value(args, "ambiguous");
        changetz::changetz(
            df,
            &colname,
            &from_tz,
            &to_tz,
            input_format.as_deref().unwrap_or("auto"),
            output_format.as_deref().unwrap_or("auto"),
            ambiguous.as_deref().unwrap_or("earliest"),
        )
    });
    table.insert("convert", |df, args| {
        let colname = get_string_from_value(args, "colname").unwrap_or_default();
        let from_format = get_string_from_value(args, "from")
            .or_else(|| get_string_from_value(args, "from_format"))
            .unwrap_or_default();
        let to_format = get_string_from_value(args, "to")
            .or_else(|| get_string_from_value(args, "to_format"))
            .unwrap_or_default();
        convert::convert(df, &colname, &from_format, &to_format)
    });
    table.insert("renamecol", |df, args| {
        let old_name = get_string_from_value(args, "old_name")
            .or_else(|| get_string_from_value(args, "from"))
            .unwrap_or_default();
        let new_name = get_string_from_value(args, "new_name")
            .or_else(|| get_string_from_value(args, "to"))
            .unwrap_or_default();
        renamecol::renamecol(df, &old_name, &new_name)
    });
    table.insert("timeline", |df, args| {
        let time_column = get_string_from_value(args, "time_column").unwrap_or_default();
        let interval = get_string_from_value(args, "interval").unwrap_or_default();
        let agg_type =
            get_string_from_value(args, "agg_type").unwrap_or_else(|| "count".to_string());
        let agg_column = get_string_from_value(args, "agg_column");
        timeline::timeline(
            df,
            &time_column,
            &interval,
            &agg_type,
            agg_column.as_deref(),
        )
    });
    table.insert("timeslice", |df, args| {
        let time_column = get_string_from_value(args, "time_column").unwrap_or_default();
        let start_time = get_string_from_value(args, "start");
        let end_time = get_string_from_value(args, "end");
        timeslice::timeslice(df, &time_column, start_time.as_deref(), end_time.as_deref())
    });
    table.insert("timeround", |df, args| {
        let colname = get_string_from_value(args, "colname").unwrap_or_default();
        let unit = get_string_from_value(args, "unit").unwrap_or_default();
        let output_colname = get_string_from_value(args, "output");
        timeround::timeround(df, &colname, &unit, output_colname.as_deref())
    });
    table.insert("pivot", |df, args| {
        let rows_str = get_string_from_value(args, "rows").unwrap_or_default();
        let cols_str = get_string_from_value(args, "cols")
            .or_else(|| get_string_from_value(args, "columns"))
            .unwrap_or_default();
        let values = get_string_from_value(args, "values")
            .or_else(|| get_string_from_value(args, "value"))
            .unwrap_or_default();
        let agg_func = get_string_from_value(args, "agg")
            .or_else(|| get_string_from_value(args, "aggregation"))
            .unwrap_or_else(|| "sum".to_string());
        let rows: Vec<String> = if rows_str.is_empty() {
            Vec::new()
        } else {
            rows_str.split(',').map(|s| s.trim().to_string()).collect()
        };
        let columns: Vec<String> = if cols_str.is_empty() {
            Vec::new()
        } else {
            cols_str.split(',').map(|s| s.trim().to_string()).collect()
        };
        pivot::pivot(df, &rows, &columns, &values, &agg_func)
    });
    table
}
// Create a dispatch table for finalizer operations
fn create_finalizer_dispatch_table() -> HashMap<&'static str, FinalizerOperation> {
    let mut table: HashMap<&'static str, FinalizerOperation> = HashMap::new();
    table.insert("show", |df, args| {
        if let Some(batch_size_str) = get_string_from_value(args, "batch-size") {
            match parse_batch_size(&batch_size_str) {
                Ok(batch_size) => show_op::show_with_batch_size(df, batch_size),
                Err(e) => eprintln!("Error parsing batch-size for show: {e}"),
            }
        } else {
            show_op::show(df);
        }
    });
    table.insert("showtable", |df, _args| {
        showtable_op::showtable(df);
    });
    table.insert("headers", |df, args| {
        let plain = get_bool_from_value(args, "plain");
        headers_op::headers(df, plain);
    });
    table.insert("stats", |df, _args| {
        stats_op::stats(df);
    });
    table.insert("showquery", |df, _args| {
        showquery_op::showquery(df);
    });
    table.insert("dump", |df, args| {
        let path_from_yaml = get_string_from_value(args, "path")
            .or_else(|| get_string_from_value(args, "output"))
            .unwrap_or_else(|| "output.csv".to_string());
        let separator = get_string_from_value(args, "separator")
            .and_then(|s| s.chars().next())
            .unwrap_or(',');

        if let Some(batch_size_str) = get_string_from_value(args, "batch-size") {
            match parse_batch_size(&batch_size_str) {
                Ok(batch_size) => {
                    dump_op::dump_with_batch_size(df, Some(&path_from_yaml), separator, batch_size)
                }
                Err(e) => eprintln!("Error parsing batch-size for dump: {e}"),
            }
        } else {
            dump_op::dump(df, Some(&path_from_yaml), separator);
        }
    });
    table.insert("dumpcache", |df, args| {
        let output_path = get_string_from_value(args, "output");
        dumpcache_op::dumpcache(df, output_path.as_deref());
    });
    table.insert("partition", |df, args| {
        let colname = get_string_from_value(args, "colname").unwrap_or_default();
        let output_dir = get_string_from_value(args, "output_dir")
            .or_else(|| get_string_from_value(args, "output_directory"))
            .unwrap_or_else(|| "./partitions".to_string());
        partition_op::partition(df, &colname, &output_dir);
    });
    table
}
#[derive(Debug, Serialize, Deserialize)]
pub struct QuiltConfig {
    pub title: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub stages: serde_yml::Mapping,
}
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StageConfig {
    #[serde(rename = "type")]
    pub stage_type: String,
    pub source: Option<String>,
    pub sources: Option<Vec<String>>,
    pub params: Option<Value>,
    pub steps: Option<serde_yml::Mapping>,
}
fn get_string_from_value(val: &Value, key: &str) -> Option<String> {
    val.get(key).and_then(|v| v.as_str().map(String::from))
}
fn get_string_vec_from_value(val: &Value, key: &str) -> Option<Vec<String>> {
    val.get(key).and_then(|v| v.as_sequence()).map(|seq| {
        seq.iter()
            .filter_map(|item| item.as_str().map(String::from))
            .collect()
    })
}
fn get_string_list_from_value(val: &Value, key: &str) -> Option<Vec<String>> {
    if let Some(value) = get_string_from_value(val, key) {
        let items: Vec<String> = value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if items.is_empty() {
            None
        } else {
            Some(items)
        }
    } else {
        get_string_vec_from_value(val, key)
    }
}
fn get_bool_from_value(val: &Value, key: &str) -> bool {
    val.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}
fn get_usize_from_value(val: &Value, key: &str) -> Option<usize> {
    val.get(key)
        .and_then(|v| v.as_u64().and_then(|u| usize::try_from(u).ok()))
}
pub fn quilt(
    controller: &mut DataFrameController,
    config_path_str: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    output_path_str: Option<&str>,
) {
    let config_path = Path::new(config_path_str);
    let config_content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading config file {}: {}", config_path.display(), e);
            std::process::exit(1);
        }
    };
    let quilt_config: QuiltConfig = match serde_yml::from_str(&config_content) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error parsing YAML config: {e}");
            std::process::exit(1);
        }
    };
    LogController::info(&format!(
        "Executing quilt '{}' with {} stage entries in YAML",
        quilt_config.title,
        quilt_config.stages.len()
    ));
    let chainable_ops = create_chainable_dispatch_table();
    let finalizer_ops = create_finalizer_dispatch_table();
    let mut stage_results: HashMap<String, LazyFrame> = HashMap::new();
    let mut last_processed_df: Option<LazyFrame> = None;
    for (stage_name_val, stage_config_val) in &quilt_config.stages {
        let stage_name = stage_name_val
            .as_str()
            .unwrap_or("unknown_stage")
            .to_string();
        let stage_config: StageConfig = match serde_yml::from_value(stage_config_val.clone()) {
            Ok(sc) => sc,
            Err(e) => {
                LogController::error(&format!(
                    "Error parsing config for stage '{stage_name}': {e}. Skipping."
                ));
                continue;
            }
        };
        LogController::debug(&format!(
            "Processing stage: {} (type: {})",
            stage_name, stage_config.stage_type
        ));
        let mut current_stage_input_df: Option<LazyFrame> = None;
        if let Some(source_name) = &stage_config.source {
            if let Some(df) = stage_results.get(source_name) {
                current_stage_input_df = Some(df.clone());
                LogController::debug(&format!(
                    "Stage '{stage_name}' is using data from source stage '{source_name}'"
                ));
            } else {
                LogController::error(&format!(
                    "Source stage '{source_name}' not found for stage '{stage_name}'. Skipping stage."
                ));
                continue;
            }
        }
        let mut stage_output_df: Option<LazyFrame> = current_stage_input_df.clone();
        if stage_config.stage_type == "process" {
            if let Some(steps) = &stage_config.steps {
                for (command_name_val, command_args_val) in steps {
                    // Handle command name with trailing underscores (for duplicates)
                    let raw_command_name = command_name_val.as_str().unwrap_or("");
                    let command_name = if raw_command_name.ends_with('_') {
                        raw_command_name.trim_end_matches('_')
                    } else {
                        raw_command_name
                    };
                    LogController::debug(&format!(
                        "Applying step: {command_name} to stage '{stage_name}'"
                    ));
                    if command_name != "load" && stage_output_df.is_none() {
                        LogController::error(&format!("No DataFrame available for step '{command_name}' in stage '{stage_name}'. Load data first or specify a valid source. Skipping step."));
                        continue;
                    }
                    match command_name {
                        "load" => {
                            let file_to_load_str = get_string_from_value(command_args_val, "path");
                            let mut loaded_df: Option<LazyFrame> = None;
                            if let Some(file_str) = file_to_load_str {
                                let source_path = Path::new(&file_str);
                                let path_to_load = if source_path.is_absolute() {
                                    source_path.to_path_buf()
                                } else {
                                    config_path
                                        .parent()
                                        .unwrap_or_else(|| Path::new("."))
                                        .join(source_path)
                                };
                                LogController::debug(&format!("Loading data from: {} (specified in quilt YAML for stage '{}')", path_to_load.display(), stage_name));
                                let separator =
                                    get_string_from_value(command_args_val, "separator")
                                        .unwrap_or_else(|| ",".to_string());
                                let low_memory =
                                    get_bool_from_value(command_args_val, "low_memory");
                                let no_headers =
                                    get_bool_from_value(command_args_val, "no_headers");
                                let chunk_size =
                                    get_usize_from_value(command_args_val, "chunk_size");
                                loaded_df = Some(load_op::load(
                                    &[path_to_load],
                                    &separator,
                                    low_memory,
                                    no_headers,
                                    chunk_size,
                                ));
                            } else if let Some(ref cli_files) = cli_input_files {
                                if stage_output_df.is_none() && !cli_files.is_empty() {
                                    LogController::debug(&format!(
                                        "Loading data from CLI for stage '{stage_name}': {cli_files:?}"
                                    ));
                                    loaded_df =
                                        Some(load_op::load(cli_files, ",", false, false, None));
                                } else if stage_output_df.is_some() {
                                    LogController::debug(&format!("Stage '{stage_name}' already has data from source, 'load' step without path will not use CLI files."));
                                } else {
                                    LogController::warn(&format!("Load step in YAML for stage '{stage_name}' has no path, and no files provided via CLI for this quilt command, or stage already sourced."));
                                }
                            } else {
                                LogController::warn(&format!("No data source specified for load in stage '{stage_name}'. Trying default test data."));
                                let default_data_path = config_path
                                    .parent()
                                    .unwrap_or_else(|| Path::new("."))
                                    .join("../sample/simple.csv");
                                if default_data_path.exists() {
                                    loaded_df = Some(load_op::load(
                                        &[default_data_path],
                                        ",",
                                        false,
                                        false,
                                        None,
                                    ));
                                }
                            }
                            if let Some(ref new_lf) = loaded_df {
                                stage_output_df = Some(new_lf.clone());
                            } else if stage_output_df.is_none() {
                                LogController::error(&format!("Failed to load any data for stage '{stage_name}' via 'load' step and no prior data for stage."));
                                continue;
                            }
                        }
                        _ => {
                            // Try chainable operations first
                            if let Some(operation) = chainable_ops.get(command_name) {
                                if let Some(ref df) = stage_output_df {
                                    stage_output_df = Some(operation(df, command_args_val));
                                } else {
                                    LogController::error(&format!("No DataFrame available for chainable operation '{command_name}' in stage '{stage_name}'"));
                                }
                            }
                            // Try finalizer operations
                            else if let Some(operation) = finalizer_ops.get(command_name) {
                                if let Some(ref df) = stage_output_df {
                                    operation(df, command_args_val);
                                } else {
                                    LogController::warn(&format!("No DataFrame available for finalizer operation '{command_name}' in stage '{stage_name}'"));
                                }
                            }
                            // Unknown operation
                            else {
                                LogController::error(&format!("Error: Unknown or unsupported step '{command_name}' in 'process' stage '{stage_name}'. Halting quilt execution."));
                                eprintln!("Error: Unknown or unsupported step '{command_name}' in 'process' stage '{stage_name}'. See qsv logs for more details.");
                                std::process::exit(1);
                            }
                        }
                    }
                }
            } else {
                LogController::warn(&format!(
                    "Stage '{stage_name}' is of type 'process' but has no steps defined."
                ));
            }
        } else if stage_config.stage_type == "concat" {
            if let Some(sources_vec) = &stage_config.sources {
                if sources_vec.len() >= 2 {
                    let mut dataframes_to_concat: Vec<LazyFrame> = Vec::new();
                    let mut missing_sources = Vec::new();

                    for source_name in sources_vec {
                        if let Some(source_df) = stage_results.get(source_name) {
                            dataframes_to_concat.push(source_df.clone());
                        } else {
                            missing_sources.push(source_name.as_str());
                        }
                    }

                    if !missing_sources.is_empty() {
                        LogController::error(&format!(
                            "Could not find source DataFrame(s): {missing_sources:?} for concat stage '{stage_name}'. Skipping."
                        ));
                        continue;
                    }

                    if dataframes_to_concat.len() >= 2 {
                        // Get concatenation method from params.how (default: vertical)
                        let concat_how = stage_config
                            .params
                            .as_ref()
                            .and_then(|p| get_string_from_value(p, "how"))
                            .unwrap_or_else(|| "vertical".to_string());

                        let result_df = match concat_how.to_lowercase().as_str() {
                            "vertical" | "v" => {
                                // Vertical concatenation (row-wise) - default behavior
                                let mut result = dataframes_to_concat[0].clone();
                                for df in dataframes_to_concat.into_iter().skip(1) {
                                    result = match polars::prelude::concat(
                                        [result, df],
                                        polars::prelude::UnionArgs::default(),
                                    ) {
                                        Ok(concat_df) => concat_df,
                                        Err(e) => {
                                            eprintln!(
                                                "Error: Failed to concatenate DataFrames vertically in stage '{stage_name}': {e}"
                                            );
                                            std::process::exit(1);
                                        }
                                    };
                                }
                                result
                            }
                            "horizontal" | "h" => {
                                // Horizontal concatenation (column-wise) - Not yet supported
                                LogController::error(&format!(
                                    "Horizontal concatenation is not yet implemented for stage '{stage_name}'. Use 'vertical' instead."
                                ));
                                continue;
                            }
                            _ => {
                                LogController::error(&format!(
                                    "Invalid concat method '{concat_how}' for stage '{stage_name}'. Use 'vertical' or 'horizontal'. Skipping."
                                ));
                                continue;
                            }
                        };

                        stage_output_df = Some(result_df);
                        LogController::debug(&format!(
                            "Concat stage '{stage_name}' completed, concatenated {} sources: {sources_vec:?} using {concat_how} method",
                            sources_vec.len()
                        ));
                    } else {
                        LogController::warn(&format!(
                            "Concat stage '{stage_name}' needs at least 2 valid DataFrames, found {}. Skipping.",
                            dataframes_to_concat.len()
                        ));
                    }
                } else {
                    LogController::error(&format!(
                        "Concat stage '{stage_name}' must have at least two sources. Found {}. Skipping.",
                        sources_vec.len()
                    ));
                    continue;
                }
            } else {
                LogController::error(&format!(
                    "Concat stage '{stage_name}' missing 'sources' parameter. Skipping."
                ));
                continue;
            }
        } else if stage_config.stage_type == "join" {
            if let Some(sources_string_vec) = &stage_config.sources {
                if sources_string_vec.len() == 2 {
                    let left_name: &str = sources_string_vec[0].as_str();
                    let right_name: &str = sources_string_vec[1].as_str();
                    if left_name.is_empty() || right_name.is_empty() {
                        LogController::error(&format!(
                            "Join stage '{stage_name}' has empty source names. Skipping."
                        ));
                        continue;
                    }
                    if let (Some(left_df), Some(right_df)) =
                        (stage_results.get(left_name), stage_results.get(right_name))
                    {
                        let join_params = stage_config.params.as_ref();
                        let how_str = join_params
                            .and_then(|p| get_string_from_value(p, "how"))
                            .unwrap_or_else(|| "inner".to_string());
                        let join_type = match how_str.to_lowercase().as_str() {
                            "inner" => JoinType::Inner,
                            "left" => JoinType::Left,
                            "outer" | "full" => JoinType::Full,
                            "cross" => JoinType::Cross,
                            _ => {
                                LogController::warn(&format!("Unsupported join type '{how_str}' for stage '{stage_name}'. Defaulting to inner join."));
                                JoinType::Inner
                            }
                        };
                        let is_cross_join = matches!(join_type, JoinType::Cross);
                        let coalesce = join_params
                            .and_then(|p| p.get("coalesce"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let joined_df_result = if is_cross_join {
                            let cross_key = "__qsv_quilt_cross_join_key";
                            let mut join_args = polars::prelude::JoinArgs::new(JoinType::Inner);
                            if coalesce {
                                join_args = join_args
                                    .with_coalesce(polars::prelude::JoinCoalesce::CoalesceColumns);
                            }
                            left_df
                                .clone()
                                .with_column(lit(cross_key).alias(cross_key))
                                .join(
                                    right_df
                                        .clone()
                                        .with_column(lit(cross_key).alias(cross_key)),
                                    &[col(cross_key)],
                                    &[col(cross_key)],
                                    join_args,
                                )
                                .select([col("*").exclude([cross_key])])
                        } else {
                            let mut join_args = polars::prelude::JoinArgs::new(join_type.clone());
                            if coalesce {
                                join_args = join_args
                                    .with_coalesce(polars::prelude::JoinCoalesce::CoalesceColumns);
                            }
                            let (left_on, right_on) = if let Some(on_cols) = join_params
                                .and_then(|p| get_string_list_from_value(p, "on"))
                                .filter(|cols| !cols.is_empty())
                            {
                                (on_cols.clone(), on_cols)
                            } else if let Some(key_cols) = join_params
                                .and_then(|p| get_string_list_from_value(p, "key"))
                                .filter(|cols| !cols.is_empty())
                            {
                                (key_cols.clone(), key_cols)
                            } else {
                                let left_cols = join_params
                                    .and_then(|p| get_string_list_from_value(p, "left_on"))
                                    .unwrap_or_default();
                                let right_cols = join_params
                                    .and_then(|p| get_string_list_from_value(p, "right_on"))
                                    .unwrap_or_default();
                                if left_cols.is_empty() || right_cols.is_empty() {
                                    LogController::error(&format!(
                                        "Join stage '{stage_name}' missing 'key'/'on' or 'left_on'/'right_on' parameter(s). Skipping."
                                    ));
                                    continue;
                                }
                                if left_cols.len() != right_cols.len() {
                                    LogController::error(&format!(
                                        "Join stage '{stage_name}' has mismatched left_on/right_on column counts. Skipping."
                                    ));
                                    continue;
                                }
                                (left_cols, right_cols)
                            };
                            let left_on_exprs: Vec<_> =
                                left_on.iter().map(|name| col(name)).collect();
                            let right_on_exprs: Vec<_> =
                                right_on.iter().map(|name| col(name)).collect();
                            left_df.clone().join(
                                right_df.clone(),
                                &left_on_exprs,
                                &right_on_exprs,
                                join_args,
                            )
                        };
                        stage_output_df = Some(joined_df_result); // Result is a LazyFrame, not Result<LazyFrame, Error>
                        LogController::debug(&format!(
                            "Join stage '{stage_name}' completed using type '{how_str}', coalesce: {coalesce}"
                        ));
                    } else {
                        let mut missing_sources = Vec::new();
                        if !stage_results.contains_key(left_name) {
                            missing_sources.push(left_name);
                        }
                        if !stage_results.contains_key(right_name) {
                            missing_sources.push(right_name);
                        }
                        LogController::error(&format!("Could not find source DataFrame(s): {missing_sources:?} for join stage '{stage_name}'. Skipping."));
                        continue;
                    }
                } else {
                    LogController::error(&format!(
                        "Join stage '{}' must have exactly two sources. Found {}. Skipping.",
                        stage_name,
                        sources_string_vec.len()
                    ));
                    continue;
                }
            } else {
                LogController::error(&format!(
                    "Join stage '{stage_name}' is missing 'sources' attribute. Skipping."
                ));
                continue;
            }
        } else {
            LogController::warn(&format!(
                "Unknown stage type: {} for stage '{}'",
                stage_config.stage_type, stage_name
            ));
        }
        if let Some(df_to_store) = &stage_output_df {
            stage_results.insert(stage_name.clone(), df_to_store.clone());
            last_processed_df = Some(df_to_store.clone());
            LogController::debug(&format!(
                "Finished processing stage '{stage_name}'. Result stored."
            ));
        } else {
            LogController::warn(&format!(
                "Stage '{stage_name}' did not produce a DataFrame."
            ));
        }
    }
    LogController::info(&format!(
        "Quilt '{}' execution processing finished.",
        quilt_config.title
    ));
    if let Some(path_str) = output_path_str {
        if let Some(final_df_to_dump) = last_processed_df {
            LogController::info(&format!("Saving final quilt output to: {path_str}"));
            let final_output_path = Path::new(path_str);
            let absolute_path = if final_output_path.is_absolute() {
                final_output_path.to_path_buf()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| Path::new(".").to_path_buf())
                    .join(final_output_path)
            };
            if let Some(parent) = absolute_path.parent() {
                if !parent.exists() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        eprintln!("Error creating directory {}: {}", parent.display(), e);
                    }
                }
            }
            dump_op::dump(
                &final_df_to_dump,
                Some(absolute_path.to_str().unwrap_or(path_str)),
                ',',
            );
        } else {
            LogController::warn(
                "No final DataFrame from quilt execution to save for --output CLI option.",
            );
        }
    } else {
        // If no CLI output, the last stage might have a showtable or show.
        // If not, and if the main qsv CLI expects something in controller.df, we might set it.
        // For now, if no --output, rely on YAML steps for display.
        if let Some(final_df_state) = last_processed_df {
            // If no output path and no explicit display in last stage, perhaps default to showtable?
            // This depends on how quilt is meant to integrate with the main qsv loop's default display.
            // For now, we ensure the main `controller`'s `df` is updated so `qsv` can show it if quilt is the last command.
            controller.set_df(final_df_state);
        }
        LogController::debug("Quilt finished. Output handled by steps in YAML or by main CLI flow if no explicit output/show in YAML.");
    }
}
