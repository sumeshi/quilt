//! Typed DAG planning boundary for run documents.

use crate::error::QuiltError;
use crate::operations::automation::model::{RunDocument, StageConfig};
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub(super) struct ExecutionPlan {
    pub order: Vec<String>,
    pub stages: HashMap<String, StageConfig>,
}

pub(super) fn build(document: &RunDocument) -> Result<ExecutionPlan, QuiltError> {
    let (stage_names, stages) = collect_stage_configs(&document.stages)
        .map_err(|error| QuiltError::automation("run", None, error))?;
    let order = resolve_stage_execution_order(&stage_names, &stages).map_err(|error| {
        QuiltError::automation(
            "run",
            None,
            format!("Error validating run document stage dependencies: {error}"),
        )
    })?;
    Ok(ExecutionPlan { order, stages })
}

pub(super) fn collect_stage_configs(
    stages: &[StageConfig],
) -> Result<(Vec<String>, HashMap<String, StageConfig>), String> {
    let mut stage_order = Vec::with_capacity(stages.len());
    let mut stage_configs = HashMap::with_capacity(stages.len());

    for stage_config in stages {
        let stage_name = stage_config.name().to_string();
        if stage_configs.contains_key(&stage_name) {
            return Err(format!("Duplicate stage name '{stage_name}'."));
        }
        stage_order.push(stage_name.clone());
        stage_configs.insert(stage_name, stage_config.clone());
    }

    Ok((stage_order, stage_configs))
}

fn get_stage_dependencies(stage_config: &StageConfig) -> Vec<String> {
    stage_config.dependencies()
}

/// Build the dependency graph used for ordering and cycle checks.
///
/// A branch's `input` is a data dependency of the branch itself, while each
/// target is a control-flow successor. The latter therefore becomes an
/// incoming dependency of the target (rather than a dependency of the branch)
/// so that a target cannot run before its branch has selected a route.
fn stage_order_dependencies(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
) -> Result<HashMap<String, Vec<String>>, String> {
    let mut dependencies = stage_configs
        .iter()
        .map(|(name, config)| (name.clone(), get_stage_dependencies(config)))
        .collect::<HashMap<_, _>>();

    for (branch_name, config) in stage_configs {
        let StageConfig::Branch(branch) = config else {
            continue;
        };
        for target in branch
            .branch
            .then
            .iter()
            .chain(branch.branch.r#else.iter().flatten())
        {
            let target_dependencies = dependencies
                .get_mut(target)
                .ok_or_else(|| format!("Branch target '{target}' not found"))?;
            if !target_dependencies.contains(branch_name) {
                target_dependencies.push(branch_name.clone());
            }
        }
    }

    // Keep deterministic validation for all stages, including an empty list.
    for stage_name in stage_order {
        if !dependencies.contains_key(stage_name) {
            return Err(format!(
                "Stage '{stage_name}' not found during dependency resolution."
            ));
        }
    }
    Ok(dependencies)
}

fn visit_stage(
    stage_name: &str,
    stage_order: &[String],
    stage_dependencies: &HashMap<String, Vec<String>>,
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

    let dependencies = stage_dependencies
        .get(stage_name)
        .ok_or_else(|| format!("Stage '{stage_name}' not found during dependency resolution."))?;

    for dep in dependencies {
        if !stage_dependencies.contains_key(dep) {
            return Err(format!(
                "Stage '{stage_name}' depends on missing stage '{dep}'."
            ));
        }
        visit_stage(
            dep,
            stage_order,
            stage_dependencies,
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

pub(super) fn resolve_stage_execution_order(
    stage_order: &[String],
    stage_configs: &HashMap<String, StageConfig>,
) -> Result<Vec<String>, String> {
    let stage_dependencies = stage_order_dependencies(stage_order, stage_configs)?;
    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut resolved = Vec::with_capacity(stage_order.len());

    for stage_name in stage_order {
        visit_stage(
            stage_name,
            stage_order,
            &stage_dependencies,
            &mut visiting,
            &mut visited,
            &mut resolved,
        )?;
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::build;
    use crate::operations::automation::model::RunDocument;

    #[test]
    fn planner_rejects_duplicate_stage_names() {
        let document: RunDocument = serde_yml::from_str(
            "version: 1\nstages: [{name: same, steps: []}, {name: same, steps: []}]",
        )
        .unwrap();
        assert!(build(&document).is_err());
    }
}
