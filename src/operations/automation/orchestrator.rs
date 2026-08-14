//! Public run boundary. All execution remains on the shared typed executor
//! and `PipelineState`; this module owns the explicit invocation context.

use super::super::model::{BranchPredicate, RunDocument, StageConfig};
use super::diagnostics;
use super::diagnostics::DiagnosticPolicy;
use super::document;
use super::executor;
use super::materialization;
use super::planner;
use crate::controllers::command_model::{DumpArgs, TypedCommand};
use crate::controllers::executor::CommandExecutor;
use crate::controllers::log::LogController;
use crate::controllers::pipeline::PipelineState;
use crate::error::QuiltError;
use polars::prelude::LazyFrame;
use serde_yml::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;

pub(super) fn run_impl(
    controller: &mut PipelineState,
    config_path_str: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    output_path_str: Option<&str>,
    run_vars: &[String],
    check_only: bool,
) -> Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> {
    let mut diagnostics = DiagnosticPolicy::default();
    let mut finalizer_results = Vec::new();
    let result: Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> = (|| {
        let config_path = Path::new(config_path_str);
        let raw_config_content = fs::read_to_string(config_path).map_err(|error| {
            QuiltError::automation(
                "run",
                None,
                format!(
                    "Error reading config file {}: {error}",
                    config_path.display()
                ),
            )
        })?;

        diagnostics = diagnostics::discover_secrets(&raw_config_content, run_vars);
        let prepared = match document::prepare(document::DocumentInput {
            raw: &raw_config_content,
            path: config_path,
            overrides: run_vars,
        }) {
            Ok(prepared) => prepared,
            Err(parameter_error) => {
                // A missing/invalid parameter must not hide independent static
                // stage diagnostics when the raw document remains structurally
                // deserializable.
                if let Ok(raw_document) = serde_yml::from_str::<RunDocument>(&raw_config_content) {
                    if let Err(stage_error) =
                        document::preflight(&raw_document, &HashMap::new(), config_path)
                    {
                        return Err(QuiltError::validation(vec![
                            parameter_error.to_string(),
                            stage_error.to_string(),
                        ]));
                    }
                }
                return Err(parameter_error);
            }
        };
        diagnostics = prepared.diagnostics;
        let parameters = prepared.parameters;
        let run_document = prepared.document;
        document::schema_version(&run_document)?;
        document::preflight(&run_document, &parameters, config_path)
            .map_err(|error| diagnostics.sanitize_preflight_error(error))?;
        let plan = planner::build(&run_document)?;
        let execution_order = plan.order;
        let stage_configs = plan.stages;
        if check_only {
            return Ok(Vec::new());
        }

        LogController::info(&format!(
            "Executing run document with {} stage entries",
            run_document.stages.len()
        ));
        let mut stage_results: HashMap<String, LazyFrame> = HashMap::new();
        let materialization = materialization::MaterializationPlan::for_run(
            &execution_order,
            &stage_configs,
            output_path_str.is_some(),
            Some(&parameters),
            config_path.parent().unwrap_or_else(|| Path::new(".")),
        )?;
        let mut last_processed_df: Option<LazyFrame> = None;
        let mut skipped_stages = HashSet::new();

        for stage_name in execution_order {
            if skipped_stages.contains(&stage_name) {
                continue;
            }
            let stage_config = match stage_configs.get(&stage_name) {
                Some(sc) => sc,
                None => {
                    return Err(QuiltError::automation(
                        &stage_name,
                        None,
                        format!("Stage '{stage_name}' disappeared during execution."),
                    ));
                }
            };

            // A route can feed an intermediate stage rather than a finalizer. If
            // that route was not selected, propagate the skip through ordinary
            // data dependencies instead of attempting to execute with a missing
            // input frame.
            if stage_config
                .dependencies()
                .iter()
                .any(|dependency| skipped_stages.contains(dependency))
            {
                skipped_stages.insert(stage_name.clone());
                if let StageConfig::Branch(branch) = stage_config {
                    for target in branch
                        .branch
                        .then
                        .iter()
                        .chain(branch.branch.r#else.iter().flatten())
                    {
                        skipped_stages.insert(target.clone());
                    }
                }
                continue;
            }

            LogController::debug("Processing stage");

            let current_stage_input_df = stage_config
                .dependencies()
                .first()
                .and_then(|source_name| stage_results.get(source_name))
                .cloned();
            let mut process_step_context = executor::ExecuteStepContext {
                config_path,
                cli_input_files: cli_input_files.as_ref(),
                results: &mut finalizer_results,
                diagnostics: &diagnostics,
                resources: controller.resources(),
            };

            let mut stage_output_df = match stage_config {
                StageConfig::Process(process) => {
                    executor::execute_process(executor::ProcessRequest {
                        stage_name: &stage_name,
                        steps: &process.steps,
                        input: current_stage_input_df.clone(),
                        context: &mut process_step_context,
                        materialize: materialization.should_materialize(&stage_name),
                    })?
                }
                StageConfig::Join(join) => {
                    executor::execute_join_stage(&stage_name, join, &stage_results)
                        .map(Some)
                        .map_err(|error| QuiltError::automation(&stage_name, None, error))?
                }
                StageConfig::Concat(concat) => {
                    executor::execute_concat_stage(&stage_name, concat, &stage_results)
                        .map(Some)
                        .map_err(|error| QuiltError::automation(&stage_name, None, error))?
                }
                StageConfig::Branch(branch) => {
                    // A branch node is routing metadata, not a user-visible
                    // output. Prevent an input frame from becoming an
                    // unrelated top-level --output result when both routes
                    // are empty.
                    last_processed_df = None;
                    let input_df = current_stage_input_df.ok_or_else(|| {
                        QuiltError::automation(
                            &stage_name,
                            None,
                            "Branch stage requires a valid input stage",
                        )
                    })?;
                    let condition_result = match &branch.branch.when {
                        BranchPredicate::RowCount { .. } => {
                            let rows = input_df
                                .clone()
                                .select([polars::prelude::len().alias("__qlt_row_count")])
                                .collect()
                                .map_err(|error| {
                                    QuiltError::automation(&stage_name, None, error.to_string())
                                })?
                                .column("__qlt_row_count")
                                .and_then(|column| column.u32())
                                .map(|values| values.get(0))
                                .map_err(|_| {
                                    QuiltError::automation(
                                        &stage_name,
                                        None,
                                        "failed to evaluate row-count predicate",
                                    )
                                })?
                                .ok_or_else(|| {
                                    QuiltError::automation(
                                        &stage_name,
                                        None,
                                        "failed to evaluate row-count predicate",
                                    )
                                })? as usize;
                            branch.branch.when.evaluate(rows)
                        }
                        BranchPredicate::Parameter { .. } => branch.branch.when.evaluate_parameter(
                            &parameters,
                            config_path.parent().unwrap_or_else(|| Path::new(".")),
                        ),
                    }
                    .map_err(|error| {
                        let error = QuiltError::automation(&stage_name, None, error);
                        if diagnostics.stage_is_sensitive(&stage_name) {
                            diagnostics.sanitize_sensitive_error(error)
                        } else {
                            diagnostics.sanitize_error(error)
                        }
                    })?;
                    let unselected = if condition_result {
                        branch.branch.r#else.as_ref()
                    } else {
                        Some(&branch.branch.then)
                    };
                    if let Some(targets) = unselected {
                        for target in targets {
                            skipped_stages.insert(target.clone());
                        }
                    }
                    Some(input_df)
                }
            };

            if materialization.should_materialize(&stage_name)
                && !matches!(stage_config, StageConfig::Process(_))
            {
                if let Some(frame) = stage_output_df.take() {
                    stage_output_df = Some(materialization::materialize_frame(
                        frame,
                        &controller.resources(),
                    )?);
                }
            }
            if let Some(df_to_store) = &stage_output_df {
                stage_results.insert(stage_name.clone(), df_to_store.clone());
                if !matches!(stage_config, StageConfig::Branch(_)) {
                    last_processed_df = Some(df_to_store.clone());
                }
                LogController::debug("Finished processing stage; result stored");
            } else {
                LogController::warn("Stage did not produce a DataFrame");
            }
        }

        LogController::info("Run document execution processing finished");
        if let Some(path_str) = output_path_str {
            if let Some(final_df_to_dump) = last_processed_df {
                LogController::info("Saving final run output");
                let final_output_path = Path::new(path_str);
                let absolute_path = if final_output_path.is_absolute() {
                    final_output_path.to_path_buf()
                } else {
                    config_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(final_output_path)
                };
                if let Some(parent) = absolute_path.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|error| {
                            QuiltError::automation(
                                "run",
                                None,
                                format!("Error creating directory {}: {error}", parent.display()),
                            )
                        })?;
                    }
                }
                let mut output_executor = CommandExecutor::from_frame_with_resources(
                    final_df_to_dump,
                    controller.resources(),
                );
                output_executor
                    .execute(&TypedCommand::Dump(DumpArgs {
                        output: Some(absolute_path.to_string_lossy().into_owned()),
                        separator: ',',
                    }))
                    .map_err(|error| {
                        QuiltError::automation_with_source("run", Some("output".into()), error)
                    })?;
                finalizer_results.extend(output_executor.finalizer_results().iter().cloned());
            } else {
                LogController::warn(
                    "No final DataFrame from run execution to save for --output CLI option.",
                );
            }
        } else {
            if let Some(final_df_state) = last_processed_df {
                controller.replace_with_frame(final_df_state);
            }
            LogController::debug(
            "Run document finished. Output handled by YAML finalizer steps or by main CLI flow if no explicit output/show in YAML.",
        );
        }
        Ok(finalizer_results)
    })();
    result.map_err(|error| diagnostics::redact(error, &diagnostics))
}

pub fn run(
    controller: &mut PipelineState,
    config_path: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    output_path: Option<&str>,
    variables: &[String],
    check_only: bool,
) -> Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> {
    run_impl(
        controller,
        config_path,
        cli_input_files,
        output_path,
        variables,
        check_only,
    )
}

pub fn run_show_plan(
    config_path: &str,
    stage_name: &str,
) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
    run_show_plan_impl(config_path, stage_name)
}

pub(super) fn run_show_plan_impl(
    config_path_str: &str,
    stage_name: &str,
) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
    let config_path = Path::new(config_path_str);
    let raw = fs::read_to_string(config_path)
        .map_err(|error| QuiltError::automation("run", None, error.to_string()))?;
    let prepared = document::prepare(document::DocumentInput {
        raw: &raw,
        path: config_path,
        overrides: &[],
    })?;
    document::schema_version(&prepared.document)?;
    document::preflight(&prepared.document, &prepared.parameters, config_path)?;
    let (stage_order, stages) = planner::collect_stage_configs(&prepared.document.stages)
        .map_err(|error| QuiltError::automation("run", None, error))?;
    if !stages.contains_key(stage_name) {
        return Err(QuiltError::usage(format!(
            "run stage '{stage_name}' not found"
        )));
    }
    let mut cache = HashMap::new();
    let mut visiting = HashSet::new();
    let mut ignored_results = Vec::new();
    let frame = build_plan_stage(
        stage_name,
        &stages,
        &mut cache,
        &mut visiting,
        config_path,
        &mut ignored_results,
    )?;
    let plan = crate::operations::finalizers::showquery::showquery(&frame)?;
    let decisions = materialization::materialization_decisions_with_output(
        &stage_order,
        &stages,
        false,
        Some(&prepared.parameters),
        config_path.parent().unwrap_or_else(|| Path::new(".")),
    )
    .map_err(|error| QuiltError::automation("run", None, error))?;
    match plan {
        crate::operations::finalizers::FinalizerResult::PlanTable(text) => {
            let decision = stage_order
                .iter()
                .filter_map(|name| decisions.get(name).map(|decision| (name, decision)))
                .map(|(name, decision)| {
                    format!(
                        "{name}: materialize={} ({})",
                        if decision.materialize { "disk" } else { "lazy" },
                        decision.reason
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            Ok(crate::operations::finalizers::FinalizerResult::PlanTable(
                format!("Materialization decisions:\n{decision}\n\n{text}"),
            ))
        }
        other => Ok(other),
    }
}

pub(super) fn build_plan_stage(
    name: &str,
    stages: &HashMap<String, StageConfig>,
    cache: &mut HashMap<String, LazyFrame>,
    visiting: &mut HashSet<String>,
    config_path: &Path,
    results: &mut Vec<crate::operations::finalizers::FinalizerResult>,
) -> Result<LazyFrame, QuiltError> {
    if let Some(frame) = cache.get(name) {
        return Ok(frame.clone());
    }
    if !visiting.insert(name.to_string()) {
        return Err(QuiltError::automation(
            "run",
            None,
            format!("cycle while building plan at stage '{name}'"),
        ));
    }
    let stage = stages
        .get(name)
        .ok_or_else(|| QuiltError::usage(format!("run stage '{name}' not found")))?;
    let dependencies = stage.dependencies();
    let mut inputs = HashMap::new();
    for dependency in &dependencies {
        inputs.insert(
            dependency.clone(),
            build_plan_stage(dependency, stages, cache, visiting, config_path, results)?,
        );
    }
    let frame = match stage {
        StageConfig::Process(process) => {
            let filtered = process
                .steps
                .iter()
                .filter(|step| {
                    step.as_mapping()
                        .and_then(|mapping| mapping.keys().next())
                        .and_then(|key| key.as_str())
                        .and_then(|key| {
                            crate::controllers::command_model::command_specs()
                                .iter()
                                .find(|spec| spec.name == key)
                        })
                        .is_none_or(|spec| {
                            spec.category
                                != crate::controllers::command_model::CommandCategory::Finalizer
                        })
                })
                .cloned()
                .collect::<Vec<_>>();
            let diagnostics = DiagnosticPolicy::default();
            let mut context = executor::ExecuteStepContext {
                config_path,
                cli_input_files: None,
                results,
                diagnostics: &diagnostics,
                resources: crate::controllers::resources::ExecutionResources::new_plan(),
            };
            executor::execute_steps(
                name,
                &Value::Sequence(filtered),
                dependencies
                    .first()
                    .and_then(|dependency| inputs.get(dependency).cloned()),
                &mut context,
                false,
            )?
            .ok_or_else(|| {
                QuiltError::usage(format!("run stage '{name}' did not produce a frame"))
            })?
        }
        StageConfig::Join(join) => executor::execute_join_stage(name, join, &inputs)
            .map_err(|error| QuiltError::automation(name, None, error))?,
        StageConfig::Concat(concat) => executor::execute_concat_stage(name, concat, &inputs)
            .map_err(|error| QuiltError::automation(name, None, error))?,
        StageConfig::Branch(_) => {
            return Err(QuiltError::usage(format!(
                "run --show-plan cannot inspect dynamic branch stage '{name}'"
            )))
        }
    };
    visiting.remove(name);
    cache.insert(name.to_string(), frame.clone());
    Ok(frame)
}
