//! Typed stage execution boundary.

use super::diagnostics::DiagnosticPolicy;
use super::materialization;
use crate::controllers::command_model::{CommandCategory, TypedCommand};
use crate::controllers::executor::CommandExecutor;
use crate::controllers::log::LogController;
use crate::controllers::pipeline::Pipeline;
use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::automation::model::{ConcatStage, JoinStage, PlannedStep};
use polars::prelude::{col, lit, Expr, JoinType, LazyFrame};
use std::collections::HashMap;
use std::path::PathBuf;

pub(super) struct ExecuteStepContext<'a> {
    pub(super) cli_input_files: Option<&'a Vec<PathBuf>>,
    pub(super) results: &'a mut Vec<crate::operations::finalizers::FinalizerResult>,
    pub(super) diagnostics: &'a DiagnosticPolicy,
    pub(super) resources: ExecutionResources,
    pub(super) source_inventory: Option<crate::controllers::pipeline::SourceInventory>,
}

pub(super) struct ProcessRequest<'steps, 'a, 'context> {
    pub stage_name: &'steps str,
    pub steps: &'steps [PlannedStep],
    pub input: Option<LazyFrame>,
    pub context: &'context mut ExecuteStepContext<'a>,
    pub materialize: bool,
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

fn is_stdout_result(result: &crate::operations::finalizers::FinalizerResult) -> bool {
    use crate::operations::finalizers::FinalizerResult;
    matches!(
        result,
        FinalizerResult::Stdout(_)
            | FinalizerResult::Scalar(_)
            | FinalizerResult::PlanTable(_)
            | FinalizerResult::Artifact(_)
    )
}

pub(super) fn execute_process(
    request: ProcessRequest<'_, '_, '_>,
) -> Result<Option<LazyFrame>, QuiltError> {
    let ProcessRequest {
        stage_name,
        steps,
        input: mut stage_output_df,
        context: step_context,
        materialize,
    } = request;
    let mut materialized = false;

    for step in steps {
        let step_index = step.index;
        let command_name = step.command.name();
        LogController::debug(&format!("Applying step {step_index}: {command_name}"));
        if step.load_paths_omitted && stage_output_df.is_some() {
            LogController::debug("Skipping load because stage data already exists");
            continue;
        }
        let sensitive_step = step_context
            .diagnostics
            .step_is_sensitive(stage_name, step_index);
        let mut command = step.command.clone();
        if let TypedCommand::Load(load) = &mut command {
            if step.load_paths_omitted {
                load.paths = step_context
                    .cli_input_files
                    .filter(|files| !files.is_empty())
                    .ok_or_else(|| {
                        step_error(
                            stage_name,
                            step_index,
                            command_name,
                            QuiltError::usage(format!(
                                "No data source specified for load in stage '{stage_name}'."
                            )),
                        )
                    })?
                    .clone();
            }
        }

        if materialize && !materialized && matches!(command.category(), CommandCategory::Finalizer)
        {
            if let Some(frame) = stage_output_df.take() {
                let (frame, resources) =
                    materialization::materialize_frame(frame, &step_context.resources).map_err(
                        |error| {
                            step_context
                                .diagnostics
                                .sanitize_step_error(error, sensitive_step)
                        },
                    )?;
                stage_output_df = Some(frame);
                step_context.resources = resources;
                materialized = true;
            }
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
            CommandExecutor::from_pipeline(
                crate::controllers::pipeline::Pipeline::new(frame, step_context.resources.clone())
                    .with_source_inventory(step_context.source_inventory.clone()),
            )
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
        let results = executor
            .finalizer_results()
            .iter()
            .cloned()
            .map(|result| match result {
                crate::operations::finalizers::FinalizerResult::PlanTable(text) => {
                    crate::operations::finalizers::FinalizerResult::PlanTable(
                        step_context.diagnostics.sanitize_plan(text, stage_name),
                    )
                }
                other => other,
            })
            .collect::<Vec<_>>();
        if step_context
            .results
            .iter()
            .chain(&results)
            .filter(|result| is_stdout_result(result))
            .count()
            > 1
        {
            return Err(QuiltError::usage("run produces multiple stdout results; select one stage with --stage NAME or use file outputs"));
        }
        step_context.results.extend(results);
        stage_output_df = executor.into_pipeline().map(|pipeline| {
            step_context.source_inventory = pipeline.source_inventory();
            pipeline.into_parts().0
        });
    }

    if materialize && !materialized {
        if let Some(frame) = stage_output_df.take() {
            let (frame, resources) =
                materialization::materialize_frame(frame, &step_context.resources).map_err(
                    |error| {
                        step_context.diagnostics.sanitize_step_error(
                            error,
                            step_context.diagnostics.frame_is_sensitive(stage_name),
                        )
                    },
                )?;
            stage_output_df = Some(frame);
            step_context.resources = resources;
        }
    }
    Ok(stage_output_df)
}

pub(super) fn execute_concat_stage(
    stage_name: &str,
    stage_config: &ConcatStage,
    stage_results: &HashMap<String, Pipeline>,
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
            dataframes_to_concat.push(source_df.frame().clone());
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
    mut left_df: LazyFrame,
    mut right_df: LazyFrame,
    _stage_name: &str,
    join_type: JoinType,
    coalesce: bool,
    key_spec: &JoinKeySpec,
) -> Result<LazyFrame, String> {
    if matches!(key_spec, JoinKeySpec::Cross) {
        let left_schema = left_df
            .collect_schema()
            .map_err(|error| error.to_string())?;
        let right_schema = right_df
            .collect_schema()
            .map_err(|error| error.to_string())?;
        let mut cross_key = "__qlt_run_cross_join_key".to_string();
        let mut suffix = 0usize;
        while left_schema.get(&cross_key).is_some() || right_schema.get(&cross_key).is_some() {
            suffix += 1;
            cross_key = format!("__qlt_run_cross_join_key_{suffix}");
        }
        let join_args = polars::prelude::JoinArgs::new(JoinType::Inner)
            .with_coalesce(polars::prelude::JoinCoalesce::CoalesceColumns);
        return Ok(left_df
            .with_column(lit(0u8).alias(&cross_key))
            .join(
                right_df.with_column(lit(0u8).alias(&cross_key)),
                &[col(&cross_key)],
                &[col(&cross_key)],
                join_args,
            )
            .drop([cross_key]));
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
    let left_on_exprs: Vec<_> = left_on
        .iter()
        .map(|name| Expr::Column(name.into()))
        .collect();
    let right_on_exprs: Vec<_> = right_on
        .iter()
        .map(|name| Expr::Column(name.into()))
        .collect();
    LogController::debug("Joining dataframes");
    Ok(left_df.join(right_df, &left_on_exprs, &right_on_exprs, join_args))
}

pub(super) fn execute_join_stage(
    stage_name: &str,
    stage_config: &JoinStage,
    stage_results: &HashMap<String, Pipeline>,
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
        dataframes.push(df.frame().clone());
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
        )?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::diagnostics::DiagnosticPolicy;
    use super::{execute_process, ExecuteStepContext, ProcessRequest};
    use crate::controllers::resources::ExecutionResources;
    use polars::{df, prelude::IntoLazy};

    #[test]
    fn process_preserves_artifact_resource_ownership() {
        let resources = ExecutionResources::new();
        let mut results = Vec::new();
        let diagnostics = DiagnosticPolicy::default();
        let steps = vec![crate::operations::automation::model::PlannedStep {
            index: 0,
            command: crate::controllers::command_model::TypedCommand::Show(
                crate::controllers::command_model::ShowArgs { debug: false },
            ),
            load_paths_omitted: false,
        }];

        let output = {
            let mut context = ExecuteStepContext {
                cli_input_files: None,
                results: &mut results,
                diagnostics: &diagnostics,
                resources: resources.clone(),
                source_inventory: None,
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
        assert_eq!(results.len(), 1);
        let artifact_path = match &results[0] {
            crate::operations::finalizers::FinalizerResult::Artifact(artifact) => {
                artifact.path().to_path_buf()
            }
            other => panic!("show must produce an artifact, got {other:?}"),
        };
        assert!(artifact_path.exists());

        drop(output);
        drop(resources);
        assert!(artifact_path.exists());

        drop(results);
        assert!(!artifact_path.exists());
    }
}
