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
use crate::controllers::pipeline::{Pipeline, PipelineState};
use crate::controllers::resources::ExecutionResources;
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
    selected_stage: Option<&str>,
) -> Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> {
    let started = std::time::Instant::now();
    let mut diagnostics = DiagnosticPolicy::default();
    let mut finalizer_results = Vec::new();
    let result: Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> = (|| {
        let config_path_buf = absolute_path(Path::new(config_path_str))?;
        let config_path = config_path_buf.as_path();
        let cli_input_files = cli_input_files
            .as_ref()
            .map(|files| {
                files
                    .iter()
                    .map(|path| absolute_path(path))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
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
        let prepared = prepare_document(&raw_config_content, config_path, run_vars)?;
        diagnostics = prepared.diagnostics;
        let parameters = prepared.parameters;
        let run_document = prepared.document;
        document::schema_version(&run_document).map_err(|error| {
            diagnostics
                .sanitize_step_error(error, diagnostics.sensitive_locations.contains("version"))
        })?;
        document::preflight(&run_document, &parameters, config_path)
            .map_err(|error| diagnostics.sanitize_preflight_error(error))?;
        let mut plan = planner::build(&run_document)?;
        if let Some(stage) = selected_stage {
            planner::select(&mut plan, stage)?;
        }
        diagnostics.propagate(&plan.order, &plan.stages);
        let execution_order = plan.order;
        let stage_configs = plan.stages;
        LogController::info(&format!(
            "Executing run document with {} stage entries",
            run_document.stages.len()
        ));
        let mut stage_results: HashMap<String, LazyFrame> = HashMap::new();
        let mut resources_by_stage = HashMap::<String, ExecutionResources>::new();
        let mut source_inventories =
            HashMap::<String, crate::controllers::pipeline::SourceInventory>::new();
        if output_path_str == Some("-") {
            return Err(QuiltError::usage(
                "run --output requires a file path; use a show step for stdout",
            ));
        }
        let cli_output_path = output_path_str
            .map(|path| absolute_path(Path::new(path)))
            .transpose()?;
        let output_stage = if cli_output_path.is_some() {
            Some(output_stage(
                &execution_order,
                &stage_configs,
                selected_stage,
            )?)
        } else {
            selected_stage.map(str::to_string)
        };
        let dump_count = output_stage
            .as_ref()
            .and_then(|name| stage_configs.get(name))
            .map(stage_dump_count)
            .unwrap_or(0);
        if cli_output_path.is_some() && dump_count > 1 {
            return Err(QuiltError::usage(
                "run --output requires one dump in the output stage; remove extra dumps",
            ));
        }
        let extra_output_consumer = cli_output_path.is_some() && dump_count == 0;
        let materialization = materialization::MaterializationPlan::for_run(
            &execution_order,
            &stage_configs,
            extra_output_consumer,
            Some(&parameters),
            config_path.parent().unwrap_or_else(|| Path::new(".")),
        )?;
        if check_only {
            return Ok(Vec::new());
        }
        let selected = execution_order
            .iter()
            .map(|name| diagnostics.sanitize_stage_name(name))
            .collect::<Vec<_>>()
            .join(", ");
        let disk = execution_order
            .iter()
            .filter(|name| materialization.should_materialize(name))
            .map(|name| diagnostics.sanitize_stage_name(name))
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("run: stages [{selected}]; disk intermediates [{disk}]");
        let mut last_processed_df: Option<(LazyFrame, ExecutionResources)> = None;
        let mut last_source_inventory = None;
        let mut remaining_consumers = HashMap::<String, usize>::new();
        for stage in stage_configs.values() {
            for dependency in stage.dependencies() {
                *remaining_consumers.entry(dependency).or_default() += 1;
            }
        }
        let mut executed = 0usize;
        let mut skipped_stages = HashSet::new();
        let mut dump_overridden = false;

        for stage_name in &execution_order {
            let stage_name = stage_name.clone();
            if skipped_stages.contains(&stage_name) {
                release_dependencies(
                    &stage_configs[&stage_name],
                    &mut remaining_consumers,
                    &mut stage_results,
                    &mut resources_by_stage,
                    &mut source_inventories,
                );
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
                release_dependencies(
                    stage_config,
                    &mut remaining_consumers,
                    &mut stage_results,
                    &mut resources_by_stage,
                    &mut source_inventories,
                );
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
                resources: controller.resources().fork(
                    stage_config
                        .dependencies()
                        .iter()
                        .filter_map(|dependency| resources_by_stage.get(dependency).cloned())
                        .collect(),
                ),
                source_inventory: stage_config
                    .dependencies()
                    .first()
                    .and_then(|source| source_inventories.get(source))
                    .cloned(),
            };

            let mut stage_output_df = match stage_config {
                StageConfig::Process(process) => {
                    let mut overridden_steps;
                    let steps = if let Some(path) = cli_output_path.as_ref() {
                        if output_stage.as_deref() == Some(stage_name.as_str()) && dump_count == 1 {
                            overridden_steps = process.steps.clone();
                            apply_cli_dump_override(&mut overridden_steps, &path.to_string_lossy());
                            dump_overridden = true;
                            &overridden_steps
                        } else {
                            &process.steps
                        }
                    } else {
                        &process.steps
                    };
                    executor::execute_process(executor::ProcessRequest {
                        stage_name: &stage_name,
                        steps,
                        input: current_stage_input_df.clone(),
                        context: &mut process_step_context,
                        materialize: materialization.should_materialize(&stage_name),
                    })?
                }
                StageConfig::Join(join) => {
                    executor::execute_join_stage(&stage_name, join, &stage_results)
                        .map(Some)
                        .map_err(|error| {
                            diagnostics.sanitize_step_error(
                                QuiltError::automation(&stage_name, None, error),
                                diagnostics.frame_is_sensitive(&stage_name),
                            )
                        })?
                }
                StageConfig::Concat(concat) => {
                    executor::execute_concat_stage(&stage_name, concat, &stage_results)
                        .map(Some)
                        .map_err(|error| {
                            diagnostics.sanitize_step_error(
                                QuiltError::automation(&stage_name, None, error),
                                diagnostics.frame_is_sensitive(&stage_name),
                            )
                        })?
                }
                StageConfig::Branch(branch) => {
                    // A branch node is routing metadata, not a user-visible
                    // output. Prevent an input frame from becoming an
                    // unrelated fallback --output result when both routes
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
                                    diagnostics.sanitize_step_error(
                                        QuiltError::automation(
                                            &stage_name,
                                            None,
                                            error.to_string(),
                                        ),
                                        diagnostics.frame_is_sensitive(&stage_name),
                                    )
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
                        if diagnostics.frame_is_sensitive(&stage_name) {
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

            if matches!(
                stage_config,
                StageConfig::Process(_) | StageConfig::Branch(_)
            ) {
                if let Some(inventory) = process_step_context.source_inventory.take() {
                    source_inventories.insert(stage_name.clone(), inventory);
                }
            }
            let mut stage_resources = process_step_context.resources.clone();
            drop(process_step_context);

            if materialization.should_materialize(&stage_name)
                && !matches!(stage_config, StageConfig::Process(_))
            {
                if let Some(frame) = stage_output_df.take() {
                    let (frame, resources) = materialization::materialize_frame(
                        frame,
                        &stage_resources,
                    )
                    .map_err(|error| {
                        diagnostics
                            .sanitize_step_error(error, diagnostics.frame_is_sensitive(&stage_name))
                    })?;
                    stage_output_df = Some(frame);
                    stage_resources = resources;
                }
            }
            if let Some(df_to_store) = &stage_output_df {
                stage_results.insert(stage_name.clone(), df_to_store.clone());
                resources_by_stage.insert(stage_name.clone(), stage_resources.clone());
                if !matches!(stage_config, StageConfig::Branch(_))
                    && output_stage.as_ref().is_none_or(|name| name == &stage_name)
                {
                    last_processed_df = Some((df_to_store.clone(), stage_resources.clone()));
                    last_source_inventory = source_inventories.get(&stage_name).cloned();
                }
                LogController::debug("Finished processing stage; result stored");
            } else {
                LogController::warn("Stage did not produce a DataFrame");
            }
            executed += 1;
            release_dependencies(
                stage_config,
                &mut remaining_consumers,
                &mut stage_results,
                &mut resources_by_stage,
                &mut source_inventories,
            );
            if remaining_consumers
                .get(&stage_name)
                .copied()
                .unwrap_or_default()
                == 0
            {
                stage_results.remove(&stage_name);
                resources_by_stage.remove(&stage_name);
                source_inventories.remove(&stage_name);
            }
        }

        LogController::info("Run document execution processing finished");
        if let Some(absolute_path) = cli_output_path.filter(|_| !dump_overridden) {
            if let Some((final_df_to_dump, resources)) = last_processed_df {
                LogController::info("Saving final run output");
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
                let mut output_executor =
                    CommandExecutor::from_frame_with_resources(final_df_to_dump, resources);
                output_executor
                    .execute(&TypedCommand::Dump(DumpArgs {
                        output: Some(absolute_path.to_string_lossy().into_owned()),
                        separator: ',',
                    }))
                    .map_err(|error| {
                        diagnostics.sanitize_step_error(
                            QuiltError::automation_with_source("run", Some("output".into()), error),
                            output_stage
                                .as_ref()
                                .is_some_and(|name| diagnostics.frame_is_sensitive(name)),
                        )
                    })?;
                finalizer_results.extend(output_executor.finalizer_results().iter().cloned());
            } else {
                LogController::warn(
                    "No final DataFrame from run execution to save for --output CLI option.",
                );
            }
        } else if output_path_str.is_none() {
            if let Some((final_df_state, resources)) = last_processed_df {
                *controller = PipelineState::Loaded(Box::new(
                    Pipeline::new(final_df_state, resources)
                        .with_source_inventory(last_source_inventory),
                ));
            }
            LogController::debug(
            "Run document finished. Output handled by YAML finalizer steps or by main CLI flow if no explicit output/show in YAML.",
        );
        }
        eprintln!(
            "run: completed {executed} stages in {:.3}s; {} results",
            started.elapsed().as_secs_f64(),
            finalizer_results.len()
        );
        report_completed_files(&finalizer_results, &diagnostics);
        Ok(finalizer_results.clone())
    })();
    result.map_err(|error| {
        report_completed_files(&finalizer_results, &diagnostics);
        diagnostics::redact(error, &diagnostics)
    })
}

fn release_dependencies(
    stage: &StageConfig,
    remaining: &mut HashMap<String, usize>,
    frames: &mut HashMap<String, LazyFrame>,
    resources: &mut HashMap<String, ExecutionResources>,
    inventories: &mut HashMap<String, crate::controllers::pipeline::SourceInventory>,
) {
    for dependency in stage.dependencies() {
        if let Some(count) = remaining.get_mut(&dependency) {
            *count -= 1;
            if *count == 0 {
                frames.remove(&dependency);
                resources.remove(&dependency);
                inventories.remove(&dependency);
            }
        }
    }
}

fn step_command_name(step: &Value) -> Option<&str> {
    step.as_mapping()?.keys().next()?.as_str()
}

pub(super) fn is_finalizer(step: &Value) -> bool {
    step_command_name(step)
        .and_then(|name| {
            crate::controllers::command_model::command_specs()
                .iter()
                .find(|spec| spec.name == name)
        })
        .is_some_and(|spec| {
            spec.category == crate::controllers::command_model::CommandCategory::Finalizer
        })
}

fn stage_dump_count(stage: &StageConfig) -> usize {
    match stage {
        StageConfig::Process(process) => process
            .steps
            .iter()
            .filter(|step| step_command_name(step) == Some("dump"))
            .count(),
        _ => 0,
    }
}

fn output_stage(
    order: &[String],
    stages: &HashMap<String, StageConfig>,
    selected: Option<&str>,
) -> Result<String, QuiltError> {
    if let Some(selected) = selected {
        return Ok(selected.to_string());
    }
    let consumed = stages
        .values()
        .flat_map(StageConfig::dependencies)
        .collect::<HashSet<_>>();
    let sinks = order
        .iter()
        .filter(|name| {
            !consumed.contains(*name) && !matches!(stages[*name], StageConfig::Branch(_))
        })
        .collect::<Vec<_>>();
    if sinks.len() != 1 {
        return Err(QuiltError::usage(
            "run --output requires one sink stage; select a result with --stage NAME",
        ));
    }
    Ok(sinks[0].clone())
}

fn absolute_path(path: &Path) -> Result<PathBuf, QuiltError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|error| QuiltError::io("run current directory", None::<String>, error.to_string()))
}

fn prepare_document(
    raw: &str,
    path: &Path,
    vars: &[String],
) -> Result<document::PreparedRunDocument, QuiltError> {
    match document::prepare(document::DocumentInput {
        raw,
        path,
        overrides: vars,
    }) {
        Ok(prepared) => Ok(prepared),
        Err(parameter_error) => {
            if let Ok(raw_document) = serde_yml::from_str::<RunDocument>(raw) {
                if let Err(stage_error) = document::preflight(&raw_document, &HashMap::new(), path)
                {
                    let mut diagnostics = match parameter_error {
                        QuiltError::Validation { diagnostics } => diagnostics,
                        other => vec![other.to_string()],
                    };
                    match stage_error {
                        QuiltError::Validation {
                            diagnostics: stage_diagnostics,
                        } => diagnostics.extend(stage_diagnostics),
                        other => diagnostics.push(other.to_string()),
                    }
                    return Err(QuiltError::validation(diagnostics));
                }
            }
            Err(parameter_error)
        }
    }
}

fn report_completed_files(
    results: &[crate::operations::finalizers::FinalizerResult],
    diagnostics: &DiagnosticPolicy,
) {
    use crate::operations::finalizers::FinalizerResult;
    for result in results {
        let paths = match result {
            FinalizerResult::File(path) => std::slice::from_ref(path),
            FinalizerResult::Files(paths) => paths.as_slice(),
            _ => continue,
        };
        for path in paths {
            eprintln!(
                "run: completed file {}",
                if diagnostics.has_sensitive_output() {
                    "<redacted>".to_string()
                } else {
                    diagnostics.sanitize(path.display().to_string())
                }
            );
        }
    }
}

fn stage_io_summary(stage: &StageConfig, name: &str, diagnostics: &DiagnosticPolicy) -> String {
    if diagnostics.frame_is_sensitive(name) {
        return "I/O=<redacted>".into();
    }
    let StageConfig::Process(process) = stage else {
        return "I/O=[]".into();
    };
    process
        .steps
        .iter()
        .filter_map(|step| {
            let command = step_command_name(step)?;
            if !matches!(command, "load" | "dump" | "dumpcache" | "partition") {
                return None;
            }
            let value = step.as_mapping()?.values().next()?;
            Some(format!(
                "{command}={}",
                serde_yml::to_string(value).unwrap_or_default().trim()
            ))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn apply_cli_dump_override(steps: &mut [Value], path: &str) {
    for step in steps.iter_mut().rev() {
        let Some(mapping) = step.as_mapping_mut() else {
            continue;
        };
        let key = Value::String("dump".into());
        let Some(args) = mapping.get_mut(&key) else {
            continue;
        };
        let mut output = match args {
            Value::Mapping(existing) => existing.clone(),
            _ => serde_yml::Mapping::new(),
        };
        output.insert(
            Value::String("output".into()),
            Value::String(path.to_string()),
        );
        *args = Value::Mapping(output);
        return;
    }
}

pub fn run(
    controller: &mut PipelineState,
    config_path: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    output_path: Option<&str>,
    variables: &[String],
    check_only: bool,
    selected_stage: Option<&str>,
) -> Result<Vec<crate::operations::finalizers::FinalizerResult>, QuiltError> {
    run_impl(
        controller,
        config_path,
        cli_input_files,
        output_path,
        variables,
        check_only,
        selected_stage,
    )
}

pub fn run_show_plan(
    config_path: &str,
    stage_name: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    variables: &[String],
) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
    run_show_plan_impl(config_path, stage_name, cli_input_files, variables)
}

pub(super) fn run_show_plan_impl(
    config_path_str: &str,
    stage_name: &str,
    cli_input_files: Option<Vec<PathBuf>>,
    variables: &[String],
) -> Result<crate::operations::finalizers::FinalizerResult, QuiltError> {
    let mut diagnostics = DiagnosticPolicy::default();
    let result = (|| {
        let config_path = absolute_path(Path::new(config_path_str))?;
        let raw = fs::read_to_string(&config_path)
            .map_err(|error| QuiltError::automation("run", None, error.to_string()))?;
        diagnostics = diagnostics::discover_secrets(&raw, variables);
        let prepared = prepare_document(&raw, &config_path, variables)?;
        diagnostics = prepared.diagnostics;
        document::schema_version(&prepared.document).map_err(|error| {
            diagnostics
                .sanitize_step_error(error, diagnostics.sensitive_locations.contains("version"))
        })?;
        document::preflight(&prepared.document, &prepared.parameters, &config_path)
            .map_err(|error| diagnostics.sanitize_preflight_error(error))?;
        let mut plan = planner::build(&prepared.document)?;
        planner::select(&mut plan, stage_name)?;
        diagnostics.propagate(&plan.order, &plan.stages);
        let cli_input_files = cli_input_files
            .as_ref()
            .map(|files| {
                files
                    .iter()
                    .map(|path| absolute_path(path))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        let mut context = executor::ExecuteStepContext {
            config_path: &config_path,
            cli_input_files: cli_input_files.as_ref(),
            results: &mut Vec::new(),
            diagnostics: &diagnostics,
            resources: crate::controllers::resources::ExecutionResources::new_plan(),
            source_inventory: None,
        };
        let frame = build_plan_stage(
            stage_name,
            &plan.stages,
            &mut HashMap::new(),
            &mut HashSet::new(),
            &mut context,
        )?;
        let text =
            match crate::operations::finalizers::showquery::showquery(&frame).map_err(|error| {
                diagnostics.sanitize_step_error(error, diagnostics.frame_is_sensitive(stage_name))
            })? {
                crate::operations::finalizers::FinalizerResult::PlanTable(text) => text,
                _ => {
                    return Err(QuiltError::operation(
                        "run show-plan",
                        "expected a query plan",
                    ))
                }
            };
        let decisions = materialization::materialization_decisions_with_output(
            &plan.order,
            &plan.stages,
            false,
            Some(&prepared.parameters),
            config_path.parent().unwrap_or_else(|| Path::new(".")),
        )
        .map_err(|error| QuiltError::automation("run", None, error))?;
        let decision = plan
            .order
            .iter()
            .filter_map(|name| decisions.get(name).map(|decision| (name, decision)))
            .map(|(name, decision)| {
                format!(
                    "{}: materialize={} ({}); dependencies=[{}]; {}",
                    diagnostics.sanitize_stage_name(name),
                    if decision.materialize { "disk" } else { "lazy" },
                    decision.reason,
                    plan.stages[name]
                        .dependencies()
                        .iter()
                        .map(|dependency| diagnostics.sanitize_stage_name(dependency))
                        .collect::<Vec<_>>()
                        .join(", "),
                    stage_io_summary(&plan.stages[name], name, &diagnostics)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = diagnostics.sanitize_plan(text, stage_name);
        Ok(crate::operations::finalizers::FinalizerResult::PlanTable(
            format!("Materialization decisions:\n{decision}\n\n{text}"),
        ))
    })();
    result.map_err(|error| diagnostics::redact(error, &diagnostics))
}

pub(super) fn build_plan_stage(
    name: &str,
    stages: &HashMap<String, StageConfig>,
    cache: &mut HashMap<String, LazyFrame>,
    visiting: &mut HashSet<String>,
    context: &mut executor::ExecuteStepContext<'_>,
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
            build_plan_stage(dependency, stages, cache, visiting, context)?,
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
            executor::execute_steps(
                name,
                &Value::Sequence(filtered),
                dependencies
                    .first()
                    .and_then(|dependency| inputs.get(dependency).cloned()),
                context,
                false,
            )?
            .ok_or_else(|| {
                QuiltError::usage(format!("run stage '{name}' did not produce a frame"))
            })?
        }
        StageConfig::Join(join) => {
            executor::execute_join_stage(name, join, &inputs).map_err(|error| {
                context.diagnostics.sanitize_step_error(
                    QuiltError::automation(name, None, error),
                    context.diagnostics.frame_is_sensitive(name),
                )
            })?
        }
        StageConfig::Concat(concat) => executor::execute_concat_stage(name, concat, &inputs)
            .map_err(|error| {
                context.diagnostics.sanitize_step_error(
                    QuiltError::automation(name, None, error),
                    context.diagnostics.frame_is_sensitive(name),
                )
            })?,
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
