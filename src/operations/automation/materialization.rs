//! Materialization policy boundary. Policy calculation is kept separate from
//! stage execution so planning remains inspectable and side-effect free.

use crate::controllers::resources::ExecutionResources;
use crate::error::QuiltError;
use crate::operations::automation::model::{
    BranchPredicate, MaterializePolicy, ResolvedParameter, StageConfig,
};
use polars::prelude::{LazyFrame, ScanArgsParquet};
use serde_yml::Value;
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

pub(super) fn materialize_frame(
    frame: LazyFrame,
    resources: &ExecutionResources,
) -> Result<LazyFrame, QuiltError> {
    let mut reservation = resources
        .reserve_temp_file("qlt-stage-materialized", "parquet")
        .map_err(|error| {
            QuiltError::io(
                "create stage materialization",
                None::<String>,
                error.to_string(),
            )
        })?;
    let path = reservation.path().to_path_buf();
    reservation.close_file();
    let sink = frame
        .sink_parquet(
            polars::prelude::SinkTarget::Path(std::sync::Arc::new(path.clone())),
            polars::prelude::ParquetWriteOptions {
                compression: polars::prelude::ParquetCompression::Snappy,
                ..Default::default()
            },
            None,
            polars::prelude::SinkOptions::default(),
        )
        .map_err(|error| QuiltError::operation("materialize stage", error.to_string()))?;
    polars::prelude::collect_all([sink])
        .map_err(|error| QuiltError::operation("materialize stage", error.to_string()))?;
    resources.retain_temp_file(reservation).map_err(|error| {
        QuiltError::io(
            "retain stage materialization",
            Some(path.display().to_string()),
            error.to_string(),
        )
    })?;
    LazyFrame::scan_parquet(&path, ScanArgsParquet::default()).map_err(|error| {
        QuiltError::io(
            "scan stage materialization",
            Some(path.display().to_string()),
            error.to_string(),
        )
    })
}

fn stage_materialize_policy(stage: &StageConfig) -> MaterializePolicy {
    match stage {
        StageConfig::Process(stage) => stage.materialize,
        StageConfig::Join(stage) => stage.materialize,
        StageConfig::Concat(stage) => stage.materialize,
        StageConfig::Branch(stage) => stage.materialize,
    }
}

fn stage_has_global_barrier(stage: &StageConfig) -> bool {
    let StageConfig::Process(process) = stage else {
        return false;
    };
    process.steps.iter().any(|step| {
        step.as_mapping()
            .and_then(|mapping| mapping.keys().next())
            .and_then(Value::as_str)
            .is_some_and(|name| matches!(name, "sort" | "uniq" | "count" | "stats" | "tail"))
    })
}

#[cfg(test)]
pub(super) fn materialization_decisions(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
) -> HashMap<String, MaterializationDecision> {
    materialization_decisions_with_output(stage_order, stage_configs, false, None, Path::new("."))
        .unwrap()
}

pub(super) fn materialization_decisions_with_output(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
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
                    .filter_map(|step| step.as_mapping()?.keys().next()?.as_str())
                    .filter(|name| {
                        matches!(
                            *name,
                            "show"
                                | "showtable"
                                | "stats"
                                | "dump"
                                | "dumpcache"
                                | "partition"
                                | "calc"
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
            let auto_reason = if has_reuse && stage_has_global_barrier(stage) {
                "auto:global-barrier"
            } else if downstream_count > 1 || (downstream_count > 0 && finalizer_count > 0) {
                "auto:fan-out"
            } else if finalizer_count > 1 {
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
        stages: &HashMap<String, StageConfig>,
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
    use serde_yml::Value;
    use std::collections::HashMap;
    use std::path::Path;

    #[test]
    fn never_policy_remains_non_materialized() {
        let mut stages = HashMap::new();
        stages.insert(
            "input".into(),
            StageConfig::Process(ProcessStage {
                name: "input".into(),
                source: None,
                materialize: MaterializePolicy::Never,
                steps: vec![Value::Mapping(Default::default())],
            }),
        );
        let plan =
            MaterializationPlan::for_run(&["input".into()], &stages, false, None, Path::new("."))
                .unwrap();
        assert!(!plan.should_materialize("input"));
    }
}
