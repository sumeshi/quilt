//! Typed stage execution boundary.

use super::diagnostics::DiagnosticPolicy;
use super::materialization;
use crate::controllers::command_model::{
    parse_automation_step, CommandCategory, DumpArgs, TypedCommand,
};
use crate::controllers::executor::CommandExecutor;
use crate::controllers::log::LogController;
use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::automation::model::{ConcatStage, JoinStage};
use polars::prelude::{col, lit, JoinType, LazyFrame};
use serde_yml::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(super) struct ExecuteStepContext<'a> {
    pub(super) config_path: &'a Path,
    pub(super) cli_input_files: Option<&'a Vec<PathBuf>>,
    pub(super) results: &'a mut Vec<crate::operations::finalizers::FinalizerResult>,
    pub(super) diagnostics: &'a DiagnosticPolicy,
    pub(super) resources: ExecutionResources,
}

pub(super) struct ProcessRequest<'a, 'context> {
    pub stage_name: &'a str,
    pub steps: &'a [Value],
    pub input: Option<LazyFrame>,
    pub context: &'context mut ExecuteStepContext<'a>,
    pub materialize: bool,
}

pub(super) fn execute_process(
    request: ProcessRequest<'_, '_>,
) -> Result<Option<LazyFrame>, QuiltError> {
    execute_steps(
        request.stage_name,
        &Value::Sequence(request.steps.to_vec()),
        request.input,
        request.context,
        request.materialize,
    )
}

pub(crate) fn step_error(
    stage: &str,
    index: usize,
    command: &str,
    error: QuiltError,
) -> QuiltError {
    match error {
        QuiltError::Automation { .. } => error,
        error => QuiltError::automation_with_source(
            stage,
            Some(format!("steps[{index}]/{command}")),
            error,
        ),
    }
}

pub(super) fn execute_steps(
    stage_name: &str,
    steps: &Value,
    mut stage_output_df: Option<LazyFrame>,
    step_context: &mut ExecuteStepContext<'_>,
    materialize: bool,
) -> Result<Option<LazyFrame>, QuiltError> {
    let parsed_steps = super::document::parse_steps(steps)?;
    let mut finalizer_seen = false;
    let mut materialized = false;

    for (step_index, raw_command_name, command_args_val) in parsed_steps {
        let command_name = raw_command_name.as_str();
        LogController::debug(&format!("Applying step {step_index}: {command_name}"));

        let has_yaml_paths = command_args_val
            .as_mapping()
            .map(|mapping| {
                mapping.contains_key(Value::String("path".to_string()))
                    || mapping.contains_key(Value::String("paths".to_string()))
            })
            .unwrap_or(false);
        if command_name == "load" && !has_yaml_paths && stage_output_df.is_some() {
            LogController::debug("Skipping load because stage data already exists");
            continue;
        }

        let command_args = if command_name == "load" && !has_yaml_paths {
            let mut value = command_args_val.clone();
            if let Some(mapping) = value.as_mapping_mut() {
                let files = step_context.cli_input_files.ok_or_else(|| {
                    step_error(
                        stage_name,
                        step_index,
                        command_name,
                        QuiltError::usage(format!(
                            "No data source specified for load in stage '{stage_name}'."
                        )),
                    )
                })?;
                if files.is_empty() {
                    return Err(step_error(
                        stage_name,
                        step_index,
                        command_name,
                        QuiltError::usage(format!(
                            "No data source specified for load in stage '{stage_name}'."
                        )),
                    ));
                }
                mapping.insert(
                    Value::String("paths".to_string()),
                    Value::Sequence(
                        files
                            .iter()
                            .map(|path| Value::String(path.to_string_lossy().into_owned()))
                            .collect(),
                    ),
                );
            }
            value
        } else {
            command_args_val
        };
        let sensitive_step = step_context
            .diagnostics
            .step_is_sensitive(stage_name, step_index);

        let mut command = parse_automation_step(command_name, &command_args).map_err(|error| {
            step_context.diagnostics.sanitize_step_error(
                step_error(stage_name, step_index, command_name, error),
                sensitive_step,
            )
        })?;

        if materialize && !materialized && matches!(command.category(), CommandCategory::Finalizer)
        {
            if let Some(frame) = stage_output_df.take() {
                stage_output_df = Some(materialization::materialize_frame(
                    frame,
                    &step_context.resources,
                )?);
                materialized = true;
            }
        }

        if finalizer_seen && !matches!(command.category(), CommandCategory::Finalizer) {
            return Err(step_context.diagnostics.sanitize_step_error(
                step_error(
                    stage_name,
                    step_index,
                    command_name,
                    QuiltError::usage(format!(
                        "Record step '{command_name}' cannot follow a finalizer in stage '{stage_name}'."
                    )),
                ),
                sensitive_step,
            ));
        }

        if let TypedCommand::Load(load) = &mut command {
            for path in &mut load.paths {
                if path.is_relative() {
                    let config_relative = step_context
                        .config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(&*path);
                    *path = config_relative;
                }
            }
        }
        if let TypedCommand::Dump(DumpArgs {
            output: Some(path), ..
        }) = &mut command
        {
            let path_buf = PathBuf::from(&*path);
            if path_buf.is_relative() {
                *path = step_context
                    .config_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path_buf)
                    .to_string_lossy()
                    .into_owned();
            }
        }
        if let TypedCommand::DumpCache(crate::controllers::command_model::DumpCacheArgs {
            output: Some(path),
        }) = &mut command
        {
            let path_buf = PathBuf::from(&*path);
            if path_buf.is_relative() {
                *path = step_context
                    .config_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path_buf)
                    .to_string_lossy()
                    .into_owned();
            }
        }
        if let TypedCommand::Partition(args) = &mut command {
            let path_buf = PathBuf::from(&args.output_dir);
            if path_buf.is_relative() {
                args.output_dir = step_context
                    .config_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(path_buf)
                    .to_string_lossy()
                    .into_owned();
            }
        }

        if matches!(command.category(), CommandCategory::Finalizer) {
            finalizer_seen = true;
        }

        if !matches!(command, TypedCommand::Load(_)) && stage_output_df.is_none() {
            return Err(step_context.diagnostics.sanitize_step_error(
                step_error(
                    stage_name,
                    step_index,
                    command_name,
                    QuiltError::usage(format!(
                        "No DataFrame available for step '{command_name}' in stage '{stage_name}'. Load data first or specify a valid source."
                    )),
                ),
                sensitive_step,
            ));
        }

        let mut executor = if let Some(frame) = stage_output_df.take() {
            CommandExecutor::from_frame_with_resources(frame, step_context.resources.clone())
        } else {
            CommandExecutor::new_with_resources(step_context.resources.clone())
        };
        if let Err(error) = executor.execute(&command) {
            return Err(step_context.diagnostics.sanitize_step_error(
                QuiltError::automation_with_source(
                    stage_name,
                    Some(format!("steps[{step_index}]/{command_name}")),
                    error,
                ),
                sensitive_step,
            ));
        }
        step_context
            .results
            .extend(executor.finalizer_results().iter().cloned());
        stage_output_df = executor
            .into_pipeline()
            .map(|pipeline| pipeline.into_parts().0);
    }

    if materialize && !materialized {
        if let Some(frame) = stage_output_df.take() {
            stage_output_df = Some(materialization::materialize_frame(
                frame,
                &step_context.resources,
            )?);
        }
    }
    Ok(stage_output_df)
}

pub(super) fn execute_concat_stage(
    stage_name: &str,
    stage_config: &ConcatStage,
    stage_results: &HashMap<String, LazyFrame>,
) -> Result<LazyFrame, String> {
    let sources_vec = &stage_config.concat.inputs;
    if sources_vec.len() < 2 {
        return Err(format!(
            "Concat stage '{stage_name}' must have at least two sources. Found {}.",
            sources_vec.len()
        ));
    }
    let mut dataframes_to_concat = Vec::new();
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
        .concat
        .how
        .clone()
        .unwrap_or_else(|| "vertical".to_string());
    match concat_how.to_lowercase().as_str() {
        "vertical" | "v" => {
            let mut iter = dataframes_to_concat.into_iter();
            let mut result = iter
                .next()
                .ok_or_else(|| format!("Concat stage '{stage_name}' has no valid sources."))?;
            for df in iter {
                result = polars::prelude::concat(
                    [result, df],
                    polars::prelude::UnionArgs::default(),
                )
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

fn parse_join_type(_stage_name: &str, how_str: &str) -> JoinType {
    match how_str.to_lowercase().as_str() {
        "inner" => JoinType::Inner,
        "left" => JoinType::Left,
        "outer" | "full" => JoinType::Full,
        "cross" => JoinType::Cross,
        _ => {
            LogController::warn("Unsupported join type; defaulting to inner join");
            JoinType::Inner
        }
    }
}

fn join_pair(
    left_df: LazyFrame,
    right_df: LazyFrame,
    _stage_name: &str,
    join_type: JoinType,
    coalesce: bool,
    key_spec: &JoinKeySpec,
) -> LazyFrame {
    if matches!(key_spec, JoinKeySpec::Cross) {
        let cross_key = "__qlt_run_cross_join_key";
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
    LogController::debug("Joining dataframes");
    left_df.join(right_df, &left_on_exprs, &right_on_exprs, join_args)
}

pub(super) fn execute_join_stage(
    stage_name: &str,
    stage_config: &JoinStage,
    stage_results: &HashMap<String, LazyFrame>,
) -> Result<LazyFrame, String> {
    let sources = &stage_config.join.inputs;
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
    let how_str = stage_config
        .join
        .how
        .clone()
        .unwrap_or_else(|| "inner".to_string());
    let join_type = parse_join_type(stage_name, &how_str);
    let coalesce = stage_config.join.coalesce.unwrap_or(false);
    let key_spec = if let Some(on) = &stage_config.join.on {
        JoinKeySpec::Symmetric(on.clone())
    } else if let (Some(left), Some(right)) =
        (&stage_config.join.left_on, &stage_config.join.right_on)
    {
        JoinKeySpec::Asymmetric {
            left: left.clone(),
            right: right.clone(),
        }
    } else if matches!(join_type, JoinType::Cross) {
        JoinKeySpec::Cross
    } else {
        return Err(format!(
            "Join stage '{stage_name}' requires 'on' or both 'left-on' and 'right-on'."
        ));
    };
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

#[cfg(test)]
mod tests {
    use super::super::diagnostics::DiagnosticPolicy;
    use super::{execute_process, ExecuteStepContext, ProcessRequest};
    use crate::controllers::resources::ExecutionResources;
    use polars::{df, prelude::IntoLazy};
    use serde_yml::Value;
    use std::path::Path;

    #[test]
    fn process_boundary_has_explicit_request_context() {
        assert!(std::mem::size_of::<super::ProcessRequest<'static, 'static>>() > 0);
    }

    #[test]
    fn process_preserves_finalizer_order_and_resource_ownership() {
        let resources = ExecutionResources::new();
        let mut results = Vec::new();
        let diagnostics = DiagnosticPolicy::default();
        let steps = vec![
            serde_yml::from_str::<Value>("{headers: {plain: true}}").unwrap(),
            serde_yml::from_str::<Value>("{show: {debug: false}}").unwrap(),
        ];

        let output = {
            let mut context = ExecuteStepContext {
                config_path: Path::new("run.yaml"),
                cli_input_files: None,
                results: &mut results,
                diagnostics: &diagnostics,
                resources: resources.clone(),
            };
            let output = execute_process(ProcessRequest {
                stage_name: "input",
                steps: &steps,
                input: Some(df!("value" => &[1i64, 2]).unwrap().lazy()),
                context: &mut context,
                materialize: false,
            })
            .unwrap();
            drop(context);
            output
        };

        assert!(
            output.is_some(),
            "process must retain the frame after finalizers"
        );
        assert_eq!(results.len(), 2);
        assert!(matches!(
            &results[0],
            crate::operations::finalizers::FinalizerResult::Stdout(text)
                if text == "value\n"
        ));
        let artifact_path = match &results[1] {
            crate::operations::finalizers::FinalizerResult::Artifact(artifact) => {
                artifact.path().to_path_buf()
            }
            other => panic!("second finalizer must produce an artifact, got {other:?}"),
        };
        assert!(artifact_path.exists());

        drop(output);
        drop(resources);
        assert!(artifact_path.exists());

        drop(results);
        assert!(!artifact_path.exists());
    }
}
