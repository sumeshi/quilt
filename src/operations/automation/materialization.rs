//! Materialization policy boundary. Policy calculation is kept separate from
//! stage execution so planning remains inspectable and side-effect free.

use crate::controllers::log::LogController;
use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::automation::model::{
    BranchPredicate, MaterializePolicy, PlannedStep, ResolvedParameter, StageConfig,
};
use polars::prelude::LazyFrame;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MaterializationDecision {
    pub(super) materialize: bool,
    pub(super) reason: &'static str,
}

pub(super) struct MaterializationPlan {
    decisions: HashMap<String, MaterializationDecision>,
}

pub(super) fn materialize_pipeline(
    pipeline: crate::controllers::pipeline::Pipeline,
    policy: MaterializePolicy,
) -> Result<crate::controllers::pipeline::Pipeline, QuiltError> {
    let (frame, resources) =
        materialize_frame(pipeline.frame().clone(), &pipeline.resources(), policy)?;
    Ok(
        crate::controllers::pipeline::Pipeline::new(frame, resources)
            .with_source_inventory(pipeline.source_inventory()),
    )
}

pub(super) fn materialize_frame(
    mut frame: LazyFrame,
    resources: &ExecutionResources,
    policy: MaterializePolicy,
) -> Result<(LazyFrame, ExecutionResources), QuiltError> {
    // Parquet cannot represent a struct with no fields. Auto is only an
    // optimization, so leave such frames lazy and let their consumers see the
    // original schema. Always makes an explicit request and reports why it
    // cannot be honored.
    let schema = match frame.collect_schema() {
        Ok(schema) => schema,
        Err(_) if policy == MaterializePolicy::Auto => {
            LogController::debug(
                "Skipping automatic stage materialization because schema inspection failed",
            );
            return Ok((frame, resources.clone()));
        }
        Err(error) => {
            return Err(QuiltError::operation(
                "materialize stage",
                format!("could not inspect the frame schema for Parquet materialization: {error}"),
            ));
        }
    };
    if let Some(column) = schema
        .iter()
        .find_map(|(name, dtype)| contains_empty_struct(dtype).then(|| name.to_string()))
    {
        if policy == MaterializePolicy::Auto {
            LogController::debug(
                "Skipping automatic stage materialization because the schema contains an empty struct",
            );
            return Ok((frame, resources.clone()));
        }
        return Err(QuiltError::operation(
            "materialize stage",
            format!(
                "column '{column}' contains an empty struct, which Parquet cannot represent; use materialize: auto or never"
            ),
        ));
    }

    let (frame, owned, _) = crate::controllers::materialization::materialize_parquet(
        frame,
        resources,
        "qlt-stage-materialized",
        None,
    )?;
    Ok((frame, owned))
}

fn contains_empty_struct(dtype: &polars::prelude::DataType) -> bool {
    use polars::prelude::DataType;
    match dtype {
        DataType::Struct(fields) => {
            fields.is_empty()
                || fields
                    .iter()
                    .any(|field| contains_empty_struct(field.dtype()))
        }
        DataType::List(inner) => contains_empty_struct(inner),
        _ => false,
    }
}

pub(super) fn stage_materialize_policy(stage: &StageConfig<PlannedStep>) -> MaterializePolicy {
    match stage {
        StageConfig::Process(stage) => stage.materialize,
        StageConfig::Join(stage) => stage.materialize,
        StageConfig::Concat(stage) => stage.materialize,
        StageConfig::Branch(stage) => stage.materialize,
    }
}

#[cfg(test)]
pub(super) fn materialization_decisions(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig<PlannedStep>>,
) -> HashMap<String, MaterializationDecision> {
    materialization_decisions_with_output(stage_order, stage_configs, false, None, Path::new("."))
        .unwrap()
}

pub(super) fn materialization_decisions_with_output(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig<PlannedStep>>,
    output_consumer: bool,
    parameters: Option<&HashMap<String, ResolvedParameter>>,
    parameter_base: &Path,
) -> Result<HashMap<String, MaterializationDecision>, String> {
    let mut consumers = HashMap::<String, usize>::new();
    let mut excluded_targets = HashSet::new();
    for stage in stage_configs.values() {
        if let StageConfig::Branch(branch) = stage {
            if matches!(branch.branch.when, BranchPredicate::Parameter { .. }) {
                let selected = match parameters {
                    Some(values) => branch
                        .branch
                        .when
                        .evaluate_parameter(values, parameter_base)
                        .map_err(|error| format!("parameter branch evaluation failed: {error}"))?,
                    None => true,
                };
                let selected_targets = if selected {
                    &branch.branch.then
                } else {
                    branch.branch.r#else.as_deref().unwrap_or(&[])
                };
                for target in branch
                    .branch
                    .then
                    .iter()
                    .chain(branch.branch.r#else.iter().flatten())
                {
                    if !selected_targets.contains(target) {
                        excluded_targets.insert(target.clone());
                    }
                }
            }
        }
    }
    for (name, stage) in stage_configs {
        if excluded_targets.contains(name) {
            continue;
        }
        if let StageConfig::Branch(branch) = stage {
            if matches!(branch.branch.when, BranchPredicate::Parameter { .. }) {
                continue;
            }
        }
        for dependency in stage.dependencies() {
            *consumers.entry(dependency).or_default() += 1;
        }
    }
    let mut output_stages = stage_order
        .iter()
        .rev()
        .filter(|name| {
            !excluded_targets.contains(*name)
                && stage_configs
                    .get(*name)
                    .is_some_and(|stage| !matches!(stage, StageConfig::Branch(_)))
        })
        .take(1)
        .cloned()
        .collect::<HashSet<_>>();
    for stage in stage_configs.values() {
        let StageConfig::Branch(branch) = stage else {
            continue;
        };
        if !matches!(branch.branch.when, BranchPredicate::RowCount { .. }) {
            continue;
        }
        output_stages.extend(branch.branch.then.iter().cloned());
        output_stages.extend(branch.branch.r#else.iter().flatten().cloned());
    }
    Ok(stage_order
        .iter()
        .filter_map(|name| stage_configs.get(name).map(|stage| (name.clone(), stage)))
        .map(|(name, stage)| {
            let policy = stage_materialize_policy(stage);
            let finalizer_count = match stage {
                StageConfig::Process(process) => process
                    .steps
                    .iter()
                    .filter(|step| {
                        step.is_finalizer()
                            && !matches!(
                                step.command,
                                crate::controllers::command_model::TypedCommand::Headers(_)
                                    | crate::controllers::command_model::TypedCommand::ShowQuery(_)
                            )
                    })
                    .count(),
                _ => 0,
            };
            let downstream_count = consumers.get(&name).copied().unwrap_or_default();
            let has_output = output_consumer
                && output_stages.contains(&name)
                && !matches!(stage, StageConfig::Branch(_));
            let effective_consumers = downstream_count + finalizer_count + usize::from(has_output);
            let has_reuse = effective_consumers > 1;
            let auto_reason =
                if downstream_count > 1 || (downstream_count > 0 && finalizer_count > 0) {
                    "auto:fan-out"
                } else if has_reuse {
                    "auto:multiple-finalizers"
                } else {
                    "auto:serial-lazy"
                };
            let (materialize, reason) = match policy {
                MaterializePolicy::Always => (true, "policy=always"),
                MaterializePolicy::Never => (false, "policy=never"),
                MaterializePolicy::Auto => (has_reuse, auto_reason),
            };
            (
                name,
                MaterializationDecision {
                    materialize,
                    reason,
                },
            )
        })
        .collect::<HashMap<_, _>>())
}

impl MaterializationPlan {
    pub(super) fn for_run(
        order: &[String],
        stages: &HashMap<String, StageConfig<PlannedStep>>,
        has_output: bool,
        parameters: Option<&HashMap<String, ResolvedParameter>>,
        base: &Path,
    ) -> Result<Self, QuiltError> {
        Ok(Self {
            decisions: materialization_decisions_with_output(
                order, stages, has_output, parameters, base,
            )
            .map_err(|error| QuiltError::automation("run", None, error))?,
        })
    }

    pub(super) fn should_materialize(&self, stage: &str) -> bool {
        self.decisions
            .get(stage)
            .is_some_and(|decision| decision.materialize)
    }
}

#[cfg(test)]
mod tests {
    use super::MaterializationPlan;
    use crate::operations::automation::model::{MaterializePolicy, ProcessStage, StageConfig};
    use std::collections::HashMap;
    use std::path::Path;

    #[test]
    fn never_policy_remains_non_materialized() {
        let mut stages =
            HashMap::<String, StageConfig<crate::operations::automation::model::PlannedStep>>::new(
            );
        stages.insert(
            "input".into(),
            StageConfig::Process(ProcessStage {
                name: "input".into(),
                source: None,
                materialize: MaterializePolicy::Never,
                steps: Vec::new(),
            }),
        );
        let plan =
            MaterializationPlan::for_run(&["input".into()], &stages, false, None, Path::new("."))
                .unwrap();
        assert!(!plan.should_materialize("input"));
    }
}
