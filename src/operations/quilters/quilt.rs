use crate::controllers::command::parse_batch_size;
use crate::controllers::dataframe::DataFrameController;
use crate::controllers::log::LogController;
use crate::operations::quilters::sigma_json;
use polars::prelude::{col, lit, JoinType, LazyFrame};
use serde::{Deserialize, Serialize};
use serde_yml::Value;
use std::collections::{HashMap, HashSet};
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
type ChainableOperation = Box<dyn Fn(&LazyFrame, &Value) -> LazyFrame + Send + Sync>;
type FinalizerOperation = fn(&LazyFrame, &Value);
// Create a dispatch table for chainable operations
fn create_chainable_dispatch_table(
    global_field_map: Option<HashMap<String, String>>,
) -> HashMap<&'static str, ChainableOperation> {
    let mut table: HashMap<&'static str, ChainableOperation> = HashMap::new();
    table.insert(
        "select",
        Box::new(|df, args| {
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
        }),
    );
    table.insert(
        "isin",
        Box::new(|df, args| {
            let colname = get_string_from_value(args, "colname").unwrap_or_default();
            let values = get_string_vec_from_value(args, "values").unwrap_or_default();
            isin::isin(df, &colname, &values)
        }),
    );
    table.insert(
        "contains",
        Box::new(|df, args| {
            let colname = get_string_from_value(args, "colname").unwrap_or_default();
            let pattern = get_string_from_value(args, "pattern").unwrap_or_default();
            let ignorecase = get_bool_from_value(args, "ignorecase");
            contains::contains(df, &colname, &pattern, ignorecase)
        }),
    );
    table.insert(
        "where",
        Box::new(move |df, args| {
            let sql = get_string_from_value(args, "sql").unwrap_or_default();
            let step_field_map = get_string_mapping_from_value(args, "field_map");
            let field_map = merge_field_maps(global_field_map.as_ref(), step_field_map.as_ref());
            let annotate = get_bool_from_value(args, "annotate");
            let with_annotation_columns = |frame: LazyFrame| {
                if annotate {
                    frame.with_columns([
                        lit(get_string_from_value(args, "sigma_title").unwrap_or_default())
                            .alias("sigma_title"),
                        lit(get_string_from_value(args, "sigma_id").unwrap_or_default())
                            .alias("sigma_id"),
                        lit(get_string_from_value(args, "sigma_level").unwrap_or_default())
                            .alias("sigma_level"),
                        lit(get_string_from_value(args, "sigma_tags").unwrap_or_default())
                            .alias("sigma_tags"),
                    ])
                } else {
                    frame
                }
            };
            let schema = match df.clone().collect_schema() {
                Ok(schema) => schema,
                Err(e) => {
                    eprintln!("Error: Failed to get schema for where step: {e}");
                    std::process::exit(1);
                }
            };
            let available_cols: Vec<String> = schema.iter_names().map(|s| s.to_string()).collect();
            let expr =
                match sigma_json::sql_to_polars_expr(&sql, &available_cols, field_map.as_ref()) {
                    Some(expr) => expr,
                    None => {
                        LogController::debug("where step produced no matching expression.");
                        return with_annotation_columns(df.clone().filter(lit(false)));
                    }
                };

            with_annotation_columns(df.clone().filter(expr))
        }),
    );
    table.insert(
        "sed",
        Box::new(|df, args| {
            let colname = get_string_from_value(args, "colname");
            let pattern = get_string_from_value(args, "pattern").unwrap_or_default();
            let replacement = get_string_from_value(args, "replacement").unwrap_or_default();
            let ignorecase = get_bool_from_value(args, "ignorecase");
            sed::sed(df, colname.as_deref(), &pattern, &replacement, ignorecase)
        }),
    );
    table.insert(
        "grep",
        Box::new(|df, args| {
            let pattern = get_string_from_value(args, "pattern").unwrap_or_default();
            let ignorecase = get_bool_from_value(args, "ignorecase");
            let is_inverted = get_bool_from_value(args, "invert_match");
            grep::grep(df, &pattern, ignorecase, is_inverted)
        }),
    );
    table.insert(
        "head",
        Box::new(|df, args| {
            let n = get_usize_from_value(args, "number")
                .or_else(|| args.as_u64().and_then(|u| usize::try_from(u).ok()))
                .unwrap_or(5);
            head::head(df, n)
        }),
    );
    table.insert(
        "tail",
        Box::new(|df, args| {
            let n = get_usize_from_value(args, "number")
                .or_else(|| args.as_u64().and_then(|u| usize::try_from(u).ok()))
                .unwrap_or(5);
            tail::tail(df, n)
        }),
    );
    table.insert(
        "sort",
        Box::new(|df, args| {
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
        }),
    );
    table.insert("count", Box::new(|df, _args| count::count(df)));
    table.insert("uniq", Box::new(|df, _args| uniq::uniq(df)));
    table.insert(
        "changetz",
        Box::new(|df, args| {
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
        }),
    );
    table.insert(
        "convert",
        Box::new(|df, args| {
            let colname = get_string_from_value(args, "colname").unwrap_or_default();
            let from_format = get_string_from_value(args, "from")
                .or_else(|| get_string_from_value(args, "from_format"))
                .unwrap_or_default();
            let to_format = get_string_from_value(args, "to")
                .or_else(|| get_string_from_value(args, "to_format"))
                .unwrap_or_default();
            convert::convert(df, &colname, &from_format, &to_format)
        }),
    );
    table.insert(
        "renamecol",
        Box::new(|df, args| {
            let old_name = get_string_from_value(args, "old_name")
                .or_else(|| get_string_from_value(args, "from"))
                .unwrap_or_default();
            let new_name = get_string_from_value(args, "new_name")
                .or_else(|| get_string_from_value(args, "to"))
                .unwrap_or_default();
            renamecol::renamecol(df, &old_name, &new_name)
        }),
    );
    table.insert(
        "timeline",
        Box::new(|df, args| {
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
        }),
    );
    table.insert("timeslice", Box::new(|df, args| {
        let time_column = get_string_from_value(args, "time_column").unwrap_or_default();
        let start_time = get_string_from_value(args, "start");
        let end_time = get_string_from_value(args, "end");
        if start_time.is_none() && end_time.is_none() {
            eprintln!(
                "Error: timeslice in quilt requires at least one of 'start' or 'end' to be specified."
            );
            std::process::exit(1);
        }
        timeslice::timeslice(df, &time_column, start_time.as_deref(), end_time.as_deref())
    }));
    table.insert(
        "timeround",
        Box::new(|df, args| {
            let colname = get_string_from_value(args, "colname").unwrap_or_default();
            let unit = get_string_from_value(args, "unit").unwrap_or_default();
            let output_colname = get_string_from_value(args, "output");
            timeround::timeround(df, &colname, &unit, output_colname.as_deref())
        }),
    );
    table.insert(
        "pivot",
        Box::new(|df, args| {
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
        }),
    );
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
    pub steps: Option<Value>,
    pub then_steps: Option<Value>,
    pub else_steps: Option<Value>,
}

fn normalize_step_name(raw_command_name: &str) -> &str {
    raw_command_name.trim_end_matches('_')
}

fn parse_steps(steps: &Value) -> Result<Vec<(String, Value)>, String> {
    match steps {
        Value::Mapping(mapping) => Ok(mapping
            .iter()
            .map(|(command_name_val, command_args_val)| {
                (
                    command_name_val.as_str().unwrap_or("").to_string(),
                    command_args_val.clone(),
                )
            })
            .collect()),
        Value::Sequence(sequence) => {
            let mut parsed_steps = Vec::with_capacity(sequence.len());

            for (index, step) in sequence.iter().enumerate() {
                match step {
                    Value::Mapping(mapping) if mapping.len() == 1 => {
                        if let Some((command_name_val, command_args_val)) = mapping.iter().next() {
                            parsed_steps.push((
                                command_name_val.as_str().unwrap_or("").to_string(),
                                command_args_val.clone(),
                            ));
                        }
                    }
                    Value::Mapping(mapping) => {
                        return Err(format!(
                            "Step {} must contain exactly one command entry, found {}.",
                            index + 1,
                            mapping.len()
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "Step {} must be a single-entry mapping like '- grep: {{...}}'.",
                            index + 1
                        ));
                    }
                }
            }

            Ok(parsed_steps)
        }
        _ => Err("Process stage 'steps' must be a mapping or a sequence.".to_string()),
    }
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

fn get_string_mapping_from_value(val: &Value, key: &str) -> Option<HashMap<String, String>> {
    val.get(key).and_then(|v| v.as_mapping()).map(|mapping| {
        mapping
            .iter()
            .filter_map(|(map_key, map_value)| {
                Some((
                    map_key.as_str()?.to_string(),
                    map_value.as_str()?.to_string(),
                ))
            })
            .collect()
    })
}

fn merge_field_maps(
    global_field_map: Option<&HashMap<String, String>>,
    step_field_map: Option<&HashMap<String, String>>,
) -> Option<HashMap<String, String>> {
    let mut merged = HashMap::new();

    if let Some(mapping) = global_field_map {
        for (key, value) in mapping {
            if !value.trim().is_empty() {
                merged.insert(key.clone(), value.clone());
            }
        }
    }

    if let Some(mapping) = step_field_map {
        for (key, value) in mapping {
            if !value.trim().is_empty() {
                merged.insert(key.clone(), value.clone());
            }
        }
    }

    if merged.is_empty() {
        None
    } else {
        Some(merged)
    }
}

fn load_mapping_file(mapping_path: &Path) -> Result<HashMap<String, String>, String> {
    let content = fs::read_to_string(mapping_path).map_err(|e| {
        format!(
            "Failed to read mapping file {}: {e}",
            mapping_path.display()
        )
    })?;
    let raw_mapping: HashMap<String, String> = serde_json::from_str(&content).map_err(|e| {
        format!(
            "Failed to parse mapping file {}: {e}",
            mapping_path.display()
        )
    })?;

    Ok(raw_mapping
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .collect())
}

fn parse_quilt_vars(quilt_vars: &[String]) -> Result<HashMap<String, String>, String> {
    let mut parsed_vars = HashMap::new();
    for entry in quilt_vars {
        let (key, value) = entry
            .split_once('=')
            .ok_or_else(|| format!("Invalid --var '{entry}'. Expected key=value."))?;
        if key.trim().is_empty() {
            return Err(format!("Invalid --var '{entry}'. Key cannot be empty."));
        }
        parsed_vars.insert(key.trim().to_string(), value.to_string());
    }
    Ok(parsed_vars)
}

fn apply_quilt_vars(config_content: &str, vars: &HashMap<String, String>) -> String {
    let mut rendered = config_content.to_string();
    for (key, value) in vars {
        rendered = rendered.replace(&format!("${{{key}}}"), value);
    }
    rendered
}

fn collect_stage_configs(
    stages: &serde_yml::Mapping,
) -> Result<(Vec<String>, HashMap<String, StageConfig>), String> {
    let mut stage_order = Vec::with_capacity(stages.len());
    let mut stage_configs = HashMap::with_capacity(stages.len());

    for (stage_name_val, stage_config_val) in stages {
        let stage_name = stage_name_val
            .as_str()
            .ok_or_else(|| "Stage names must be YAML strings.".to_string())?
            .to_string();
        let stage_config: StageConfig = serde_yml::from_value(stage_config_val.clone())
            .map_err(|e| format!("Error parsing config for stage '{stage_name}': {e}"))?;
        stage_order.push(stage_name.clone());
        stage_configs.insert(stage_name, stage_config);
    }

    Ok((stage_order, stage_configs))
}

fn get_stage_dependencies(stage_config: &StageConfig) -> Vec<String> {
    let mut deps = Vec::new();
    if let Some(source) = &stage_config.source {
        deps.push(source.clone());
    }
    if let Some(sources) = &stage_config.sources {
        deps.extend(sources.iter().cloned());
    }
    deps
}

fn visit_stage(
    stage_name: &str,
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
    visiting: &mut HashSet<String>,
    visited: &mut HashSet<String>,
    resolved: &mut Vec<String>,
) -> Result<(), String> {
    if visited.contains(stage_name) {
        return Ok(());
    }
    if !visiting.insert(stage_name.to_string()) {
        return Err(format!(
            "Circular stage dependency detected while visiting '{stage_name}'."
        ));
    }

    let stage_config = stage_configs
        .get(stage_name)
        .ok_or_else(|| format!("Stage '{stage_name}' not found during dependency resolution."))?;

    for dep in get_stage_dependencies(stage_config) {
        if !stage_configs.contains_key(&dep) {
            return Err(format!(
                "Stage '{stage_name}' depends on missing stage '{dep}'."
            ));
        }
        visit_stage(
            &dep,
            stage_order,
            stage_configs,
            visiting,
            visited,
            resolved,
        )?;
    }

    visiting.remove(stage_name);
    visited.insert(stage_name.to_string());
    resolved.push(stage_name.to_string());

    // keep signature stable for future ordering rules
    let _ = stage_order;
    Ok(())
}

fn resolve_stage_execution_order(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
) -> Result<Vec<String>, String> {
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut resolved = Vec::with_capacity(stage_order.len());

    for stage_name in stage_order {
        visit_stage(
            stage_name,
            stage_order,
            stage_configs,
            &mut visiting,
            &mut visited,
            &mut resolved,
        )?;
    }

    Ok(resolved)
}

fn parse_condition(condition: &str) -> Result<(&str, &str, usize), String> {
    let parts: Vec<&str> = condition.split_whitespace().collect();
    if parts.len() != 3 {
        return Err(format!(
            "Unsupported branch condition '{condition}'. Expected format like 'count > 100'."
        ));
    }
    let lhs = parts[0];
    let op = parts[1];
    let rhs = parts[2]
        .parse::<usize>()
        .map_err(|_| format!("Invalid branch threshold '{}'.", parts[2]))?;
    Ok((lhs, op, rhs))
}

fn evaluate_branch_condition(df: &LazyFrame, condition: &str) -> Result<bool, String> {
    let (lhs, op, rhs) = parse_condition(condition)?;
    let actual = match lhs {
        "count" | "rows" => df
            .clone()
            .collect()
            .map_err(|e| format!("Failed to evaluate branch condition: {e}"))?
            .height(),
        _ => {
            return Err(format!(
                "Unsupported branch condition lhs '{lhs}'. Use 'count' or 'rows'."
            ));
        }
    };

    Ok(match op {
        ">" => actual > rhs,
        ">=" => actual >= rhs,
        "<" => actual < rhs,
        "<=" => actual <= rhs,
        "==" => actual == rhs,
        "!=" => actual != rhs,
        _ => {
            return Err(format!("Unsupported branch condition operator '{op}'."));
        }
    })
}

fn debug_output_requested(args: &Value) -> bool {
    get_bool_from_value(args, "debug")
        || get_bool_from_value(args, "debug_show")
        || get_bool_from_value(args, "stderr")
}

fn run_finalizer(
    command_name: &str,
    df: &LazyFrame,
    args: &Value,
    finalizer_ops: &HashMap<&'static str, FinalizerOperation>,
) -> Result<(), String> {
    if debug_output_requested(args) {
        match command_name {
            "show" => {
                let csv_output = show_op::render_csv(df)?;
                eprint!("{csv_output}");
                return Ok(());
            }
            "showtable" => {
                let table_output = showtable_op::render_table(df)?;
                eprintln!("{table_output}");
                return Ok(());
            }
            "headers" => {
                let plain = get_bool_from_value(args, "plain");
                let headers_output = headers_op::render_headers(df, plain)?;
                eprint!("{headers_output}");
                return Ok(());
            }
            _ => {
                LogController::warn(&format!(
                    "Debug output redirection is not implemented for finalizer '{command_name}'. Using normal stdout handling."
                ));
            }
        }
    }

    let operation = finalizer_ops
        .get(command_name)
        .ok_or_else(|| format!("Unknown finalizer operation '{command_name}'."))?;
    operation(df, args);
    Ok(())
}

fn execute_load_step(
    stage_name: &str,
    command_args_val: &Value,
    config_path: &Path,
    cli_input_files: Option<&Vec<PathBuf>>,
    stage_output_df: &Option<LazyFrame>,
) -> Result<Option<LazyFrame>, String> {
    let file_to_load_str = get_string_from_value(command_args_val, "path");
    let mut loaded_df: Option<LazyFrame> = None;

    if let Some(file_str) = file_to_load_str {
        let source_path = Path::new(&file_str);
        let path_to_load = if source_path.is_absolute() {
            source_path.to_path_buf()
        } else {
            let config_relative = config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(source_path);
            if config_relative.exists() {
                config_relative
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(source_path)
            }
        };
        LogController::debug(&format!(
            "Loading data from: {} (specified in quilt YAML for stage '{}')",
            path_to_load.display(),
            stage_name
        ));
        let separator =
            get_string_from_value(command_args_val, "separator").unwrap_or_else(|| ",".to_string());
        let low_memory = get_bool_from_value(command_args_val, "low_memory");
        let no_headers = get_bool_from_value(command_args_val, "no_headers");
        let chunk_size = get_usize_from_value(command_args_val, "chunk_size");
        loaded_df = Some(load_op::load(
            &[path_to_load],
            &separator,
            low_memory,
            no_headers,
            chunk_size,
        ));
    } else if let Some(cli_files) = cli_input_files {
        if stage_output_df.is_none() && !cli_files.is_empty() {
            LogController::debug(&format!(
                "Loading data from CLI for stage '{stage_name}': {cli_files:?}"
            ));
            loaded_df = Some(load_op::load(cli_files, ",", false, false, None));
        } else if stage_output_df.is_some() {
            LogController::debug(&format!(
                "Stage '{stage_name}' already has data from source, 'load' step without path will not use CLI files."
            ));
        } else {
            LogController::warn(&format!(
                "Load step in YAML for stage '{stage_name}' has no path, and no files provided via CLI for this quilt command, or stage already sourced."
            ));
        }
    } else {
        LogController::warn(&format!(
            "No data source specified for load in stage '{stage_name}'."
        ));
    }

    Ok(loaded_df)
}

fn execute_steps(
    stage_name: &str,
    steps: &Value,
    mut stage_output_df: Option<LazyFrame>,
    step_context: &ExecuteStepContext<'_>,
) -> Result<Option<LazyFrame>, String> {
    let parsed_steps = parse_steps(steps)?;

    for (raw_command_name, command_args_val) in parsed_steps {
        let command_name = normalize_step_name(&raw_command_name);
        LogController::debug(&format!(
            "Applying step: {command_name} to stage '{stage_name}'"
        ));

        if command_name != "load" && stage_output_df.is_none() {
            return Err(format!(
                "No DataFrame available for step '{command_name}' in stage '{stage_name}'. Load data first or specify a valid source."
            ));
        }

        if command_name == "load" {
            if step_context.finalizer_only {
                return Err(format!(
                    "Output stage '{stage_name}' cannot contain a 'load' step."
                ));
            }
            let loaded_df = execute_load_step(
                stage_name,
                &command_args_val,
                step_context.config_path,
                step_context.cli_input_files,
                &stage_output_df,
            )?;
            if let Some(new_df) = loaded_df {
                stage_output_df = Some(new_df);
            } else if stage_output_df.is_none() {
                return Err(format!(
                    "Failed to load any data for stage '{stage_name}' via 'load' step and no prior data for stage."
                ));
            }
            continue;
        }

        if let Some(operation) = step_context.chainable_ops.get(command_name) {
            if step_context.finalizer_only {
                return Err(format!(
                    "Output stage '{stage_name}' cannot contain chainable step '{command_name}'."
                ));
            }
            if let Some(ref df) = stage_output_df {
                stage_output_df = Some(operation(df, &command_args_val));
            }
            continue;
        }

        if let Some(ref df) = stage_output_df {
            run_finalizer(
                command_name,
                df,
                &command_args_val,
                step_context.finalizer_ops,
            )?;
            continue;
        }

        return Err(format!(
            "Unknown or unsupported step '{command_name}' in stage '{stage_name}'."
        ));
    }

    Ok(stage_output_df)
}

struct ExecuteStepContext<'a> {
    config_path: &'a Path,
    cli_input_files: Option<&'a Vec<PathBuf>>,
    chainable_ops: &'a HashMap<&'static str, ChainableOperation>,
    finalizer_ops: &'a HashMap<&'static str, FinalizerOperation>,
    finalizer_only: bool,
}

fn execute_concat_stage(
    stage_name: &str,
    stage_config: &StageConfig,
    stage_results: &HashMap<String, LazyFrame>,
) -> Result<LazyFrame, String> {
    let sources_vec = stage_config
        .sources
        .as_ref()
        .ok_or_else(|| format!("Concat stage '{stage_name}' missing 'sources' parameter."))?;

    if sources_vec.len() < 2 {
        return Err(format!(
            "Concat stage '{stage_name}' must have at least two sources. Found {}.",
            sources_vec.len()
        ));
    }

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
        return Err(format!(
            "Could not find source DataFrame(s): {missing_sources:?} for concat stage '{stage_name}'."
        ));
    }

    let concat_how = stage_config
        .params
        .as_ref()
        .and_then(|p| get_string_from_value(p, "how"))
        .unwrap_or_else(|| "vertical".to_string());

    match concat_how.to_lowercase().as_str() {
        "vertical" | "v" => {
            let mut iter = dataframes_to_concat.into_iter();
            let mut result = iter
                .next()
                .ok_or_else(|| format!("Concat stage '{stage_name}' has no valid sources."))?;
            for df in iter {
                result = polars::prelude::concat([result, df], polars::prelude::UnionArgs::default())
                    .map_err(|e| {
                        format!(
                            "Failed to concatenate DataFrames vertically in stage '{stage_name}': {e}"
                        )
                    })?;
            }
            Ok(result)
        }
        "horizontal" | "h" => Err(format!(
            "Horizontal concatenation is not yet implemented for stage '{stage_name}'. Use 'vertical' instead."
        )),
        _ => Err(format!(
            "Invalid concat method '{concat_how}' for stage '{stage_name}'. Use 'vertical' or 'horizontal'."
        )),
    }
}

#[derive(Clone)]
enum JoinKeySpec {
    Cross,
    Symmetric(Vec<String>),
    Asymmetric {
        left: Vec<String>,
        right: Vec<String>,
    },
}

fn parse_join_type(stage_name: &str, how_str: &str) -> JoinType {
    match how_str.to_lowercase().as_str() {
        "inner" => JoinType::Inner,
        "left" => JoinType::Left,
        "outer" | "full" => JoinType::Full,
        "cross" => JoinType::Cross,
        _ => {
            LogController::warn(&format!(
                "Unsupported join type '{how_str}' for stage '{stage_name}'. Defaulting to inner join."
            ));
            JoinType::Inner
        }
    }
}

fn resolve_join_keys(
    stage_name: &str,
    join_params: Option<&Value>,
    allow_asymmetric: bool,
) -> Result<JoinKeySpec, String> {
    let how_str = join_params
        .and_then(|p| get_string_from_value(p, "how"))
        .unwrap_or_else(|| "inner".to_string());

    if matches!(parse_join_type(stage_name, &how_str), JoinType::Cross) {
        return Ok(JoinKeySpec::Cross);
    }

    if let Some(on_cols) = join_params
        .and_then(|p| get_string_list_from_value(p, "on"))
        .filter(|cols| !cols.is_empty())
    {
        return Ok(JoinKeySpec::Symmetric(on_cols));
    }
    if let Some(key_cols) = join_params
        .and_then(|p| get_string_list_from_value(p, "key"))
        .filter(|cols| !cols.is_empty())
    {
        return Ok(JoinKeySpec::Symmetric(key_cols));
    }

    if !allow_asymmetric {
        return Err(format!(
            "Join stage '{stage_name}' with more than two sources requires 'key' or 'on'."
        ));
    }

    let left_cols = join_params
        .and_then(|p| get_string_list_from_value(p, "left_on"))
        .unwrap_or_default();
    let right_cols = join_params
        .and_then(|p| get_string_list_from_value(p, "right_on"))
        .unwrap_or_default();

    if left_cols.is_empty() || right_cols.is_empty() {
        return Err(format!(
            "Join stage '{stage_name}' missing 'key'/'on' or 'left_on'/'right_on' parameter(s)."
        ));
    }
    if left_cols.len() != right_cols.len() {
        return Err(format!(
            "Join stage '{stage_name}' has mismatched left_on/right_on column counts."
        ));
    }

    Ok(JoinKeySpec::Asymmetric {
        left: left_cols,
        right: right_cols,
    })
}

fn join_pair(
    left_df: LazyFrame,
    right_df: LazyFrame,
    stage_name: &str,
    join_type: JoinType,
    coalesce: bool,
    key_spec: &JoinKeySpec,
) -> LazyFrame {
    if matches!(key_spec, JoinKeySpec::Cross) {
        let cross_key = "__qsv_quilt_cross_join_key";
        let mut join_args = polars::prelude::JoinArgs::new(JoinType::Inner);
        if coalesce {
            join_args = join_args.with_coalesce(polars::prelude::JoinCoalesce::CoalesceColumns);
        }
        return left_df
            .with_column(lit(cross_key).alias(cross_key))
            .join(
                right_df.with_column(lit(cross_key).alias(cross_key)),
                &[col(cross_key)],
                &[col(cross_key)],
                join_args,
            )
            .select([col("*").exclude([cross_key])]);
    }

    let mut join_args = polars::prelude::JoinArgs::new(join_type);
    if coalesce {
        join_args = join_args.with_coalesce(polars::prelude::JoinCoalesce::CoalesceColumns);
    }

    let (left_on, right_on) = match key_spec {
        JoinKeySpec::Symmetric(cols) => (cols.clone(), cols.clone()),
        JoinKeySpec::Asymmetric { left, right } => (left.clone(), right.clone()),
        JoinKeySpec::Cross => unreachable!("cross join handled earlier"),
    };

    let left_on_exprs: Vec<_> = left_on.iter().map(col).collect();
    let right_on_exprs: Vec<_> = right_on.iter().map(col).collect();

    LogController::debug(&format!(
        "Joining stage '{stage_name}' with keys left={left_on:?} right={right_on:?}"
    ));

    left_df.join(right_df, &left_on_exprs, &right_on_exprs, join_args)
}

fn execute_join_stage(
    stage_name: &str,
    stage_config: &StageConfig,
    stage_results: &HashMap<String, LazyFrame>,
) -> Result<LazyFrame, String> {
    let sources = stage_config
        .sources
        .as_ref()
        .ok_or_else(|| format!("Join stage '{stage_name}' is missing 'sources' attribute."))?;

    if sources.len() < 2 {
        return Err(format!(
            "Join stage '{stage_name}' must have at least two sources. Found {}.",
            sources.len()
        ));
    }

    let mut dataframes = Vec::with_capacity(sources.len());
    for source_name in sources {
        let df = stage_results.get(source_name).ok_or_else(|| {
            format!(
                "Could not find source DataFrame '{source_name}' for join stage '{stage_name}'."
            )
        })?;
        dataframes.push(df.clone());
    }

    let join_params = stage_config.params.as_ref();
    let how_str = join_params
        .and_then(|p| get_string_from_value(p, "how"))
        .unwrap_or_else(|| "inner".to_string());
    let join_type = parse_join_type(stage_name, &how_str);
    let coalesce = join_params
        .and_then(|p| p.get("coalesce"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let key_spec = resolve_join_keys(stage_name, join_params, sources.len() == 2)?;

    let mut iter = dataframes.into_iter();
    let mut result = iter
        .next()
        .ok_or_else(|| format!("Join stage '{stage_name}' has no valid sources."))?;
    for right_df in iter {
        result = join_pair(
            result,
            right_df,
            stage_name,
            join_type.clone(),
            coalesce,
            &key_spec,
        );
    }

    Ok(result)
}

pub fn quilt(
    controller: &mut DataFrameController,
    config_path_str: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    output_path_str: Option<&str>,
    mapping_path_str: Option<&str>,
    quilt_vars: &[String],
) {
    let config_path = Path::new(config_path_str);
    let raw_config_content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading config file {}: {}", config_path.display(), e);
            std::process::exit(1);
        }
    };

    let parsed_vars = match parse_quilt_vars(quilt_vars) {
        Ok(vars) => vars,
        Err(e) => {
            eprintln!("Error parsing quilt vars: {e}");
            std::process::exit(1);
        }
    };
    let global_field_map = match mapping_path_str {
        Some(path) => {
            let mapping_path = Path::new(path);
            match load_mapping_file(mapping_path) {
                Ok(mapping) => Some(mapping),
                Err(e) => {
                    eprintln!("Error loading mapping file: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => None,
    };
    let config_content = apply_quilt_vars(&raw_config_content, &parsed_vars);

    let quilt_config: QuiltConfig = match serde_yml::from_str(&config_content) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error parsing YAML config: {e}");
            std::process::exit(1);
        }
    };
    let (stage_order, stage_configs) = match collect_stage_configs(&quilt_config.stages) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let execution_order = match resolve_stage_execution_order(&stage_order, &stage_configs) {
        Ok(order) => order,
        Err(e) => {
            eprintln!("Error validating quilt stage dependencies: {e}");
            std::process::exit(1);
        }
    };

    LogController::info(&format!(
        "Executing quilt '{}' with {} stage entries in YAML",
        quilt_config.title,
        quilt_config.stages.len()
    ));
    let chainable_ops = create_chainable_dispatch_table(global_field_map);
    let finalizer_ops = create_finalizer_dispatch_table();
    let mut stage_results: HashMap<String, LazyFrame> = HashMap::new();
    let mut last_processed_df: Option<LazyFrame> = None;

    for stage_name in execution_order {
        let stage_config = match stage_configs.get(&stage_name) {
            Some(sc) => sc,
            None => {
                eprintln!("Error: Stage '{stage_name}' disappeared during execution.");
                std::process::exit(1);
            }
        };

        LogController::debug(&format!(
            "Processing stage: {} (type: {})",
            stage_name, stage_config.stage_type
        ));

        let current_stage_input_df = stage_config
            .source
            .as_ref()
            .and_then(|source_name| stage_results.get(source_name))
            .cloned();
        let process_step_context = ExecuteStepContext {
            config_path,
            cli_input_files: cli_input_files.as_ref(),
            chainable_ops: &chainable_ops,
            finalizer_ops: &finalizer_ops,
            finalizer_only: false,
        };
        let output_step_context = ExecuteStepContext {
            config_path,
            cli_input_files: cli_input_files.as_ref(),
            chainable_ops: &chainable_ops,
            finalizer_ops: &finalizer_ops,
            finalizer_only: true,
        };

        let stage_output_df = match stage_config.stage_type.as_str() {
            "process" => {
                if let Some(steps) = &stage_config.steps {
                    match execute_steps(
                        &stage_name,
                        steps,
                        current_stage_input_df.clone(),
                        &process_step_context,
                    ) {
                        Ok(df) => df,
                        Err(e) => {
                            eprintln!("Error executing process stage '{stage_name}': {e}");
                            std::process::exit(1);
                        }
                    }
                } else {
                    LogController::warn(&format!(
                        "Stage '{stage_name}' is of type 'process' but has no steps defined."
                    ));
                    current_stage_input_df.clone()
                }
            }
            "output" => {
                let steps = match &stage_config.steps {
                    Some(steps) => steps,
                    None => {
                        eprintln!("Error: Output stage '{stage_name}' requires steps.");
                        std::process::exit(1);
                    }
                };
                match execute_steps(
                    &stage_name,
                    steps,
                    current_stage_input_df.clone(),
                    &output_step_context,
                ) {
                    Ok(df) => df,
                    Err(e) => {
                        eprintln!("Error executing output stage '{stage_name}': {e}");
                        std::process::exit(1);
                    }
                }
            }
            "branch" => {
                let input_df = match current_stage_input_df.clone() {
                    Some(df) => df,
                    None => {
                        eprintln!("Error: Branch stage '{stage_name}' requires a valid source.");
                        std::process::exit(1);
                    }
                };
                let condition = match stage_config
                    .params
                    .as_ref()
                    .and_then(|p| get_string_from_value(p, "condition"))
                {
                    Some(condition) => condition,
                    None => {
                        eprintln!("Error: Branch stage '{stage_name}' requires params.condition.");
                        std::process::exit(1);
                    }
                };
                let condition_result = match evaluate_branch_condition(&input_df, &condition) {
                    Ok(result) => result,
                    Err(e) => {
                        eprintln!("Error evaluating branch stage '{stage_name}': {e}");
                        std::process::exit(1);
                    }
                };
                let selected_steps = if condition_result {
                    stage_config.then_steps.as_ref()
                } else {
                    stage_config.else_steps.as_ref()
                };
                if let Some(steps) = selected_steps {
                    match execute_steps(
                        &stage_name,
                        steps,
                        Some(input_df.clone()),
                        &process_step_context,
                    ) {
                        Ok(df) => df,
                        Err(e) => {
                            eprintln!("Error executing branch stage '{stage_name}': {e}");
                            std::process::exit(1);
                        }
                    }
                } else {
                    Some(input_df)
                }
            }
            "concat" => match execute_concat_stage(&stage_name, stage_config, &stage_results) {
                Ok(df) => Some(df),
                Err(e) => {
                    eprintln!("Error executing concat stage '{stage_name}': {e}");
                    std::process::exit(1);
                }
            },
            "join" => match execute_join_stage(&stage_name, stage_config, &stage_results) {
                Ok(df) => Some(df),
                Err(e) => {
                    eprintln!("Error executing join stage '{stage_name}': {e}");
                    std::process::exit(1);
                }
            },
            other => {
                eprintln!("Error: Unknown stage type '{other}' for stage '{stage_name}'.");
                std::process::exit(1);
            }
        };

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
        if let Some(final_df_state) = last_processed_df {
            controller.set_df(final_df_state);
        }
        LogController::debug(
            "Quilt finished. Output handled by YAML output/finalizer steps or by main CLI flow if no explicit output/show in YAML.",
        );
    }
}
