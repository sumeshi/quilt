pub use super::model::{
    BranchNode, BranchPredicate, BranchStage, ConcatNode, ConcatStage, JoinNode, JoinStage,
    MaterializePolicy, ParameterPredicate, ProcessStage, RowCountPredicate, RunDocument, StageConfig,
};
#[cfg(test)]
use crate::controllers::command_model::CommandCategory;
#[cfg(test)]
use crate::controllers::pipeline::PipelineState;
#[cfg(test)]
use crate::error::QuiltError;
#[cfg(test)]
use polars::prelude::LazyFrame;
#[cfg(test)]
use serde_yml::Value;
#[cfg(test)]
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::path::Path;

#[path = "diagnostics.rs"]
mod diagnostics;
#[path = "document.rs"]
mod document;
#[path = "executor.rs"]
mod executor;
#[path = "materialization.rs"]
mod materialization;
#[path = "orchestrator.rs"]
mod orchestrator;
#[path = "planner.rs"]
mod planner;

#[cfg(test)]
use super::model::ParameterOrigin;
#[cfg(test)]
use super::model::ResolvedParameter;
pub use super::model::{ParameterDeclaration, ParameterLiteral, ParameterType, ParameterValue};
#[cfg(test)]
use crate::controllers::command_model::parse_automation_step;
#[cfg(test)]
pub(crate) use executor::step_error;
pub use orchestrator::{run, run_show_plan};
#[cfg(test)]
use polars::prelude::ScanArgsParquet;

#[cfg(test)]
use diagnostics::location_matches;

#[cfg(test)]
fn validate_step_sequence(stage_name: &str, steps: &[Value]) -> Result<(), QuiltError> {
    let parsed = document::parse_steps(&Value::Sequence(steps.to_vec()))?;
    let mut finalizer_seen = false;
    for (index, command_name, args) in parsed {
        let command = parse_automation_step(&command_name, &args)
            .map_err(|error| executor::step_error(stage_name, index, &command_name, error))?;
        if finalizer_seen && !matches!(command.category(), CommandCategory::Finalizer) {
            return Err(executor::step_error(
                stage_name,
                index,
                &command_name,
                QuiltError::usage(format!(
                    "Record step '{command_name}' cannot follow a finalizer in stage '{stage_name}'."
                )),
            ));
        }
        finalizer_seen |= matches!(command.category(), CommandCategory::Finalizer);
    }
    Ok(())
}

#[cfg(test)]
fn validate_run_document(
    document: &RunDocument,
    parameters: Option<&HashMap<String, ResolvedParameter>>,
) -> Result<(), QuiltError> {
    for stage in &document.stages {
        if stage.name().trim().is_empty() {
            return Err(QuiltError::usage("Stage names must be non-empty."));
        }
        let dependencies = stage.dependencies();
        if dependencies
            .iter()
            .any(|dependency| dependency.trim().is_empty())
        {
            return Err(QuiltError::usage(format!(
                "Stage '{}' contains an empty dependency.",
                stage.name()
            )));
        }
        let mut unique_dependencies = HashSet::new();
        if dependencies
            .iter()
            .any(|dependency| !unique_dependencies.insert(dependency))
        {
            return Err(QuiltError::usage(format!(
                "Stage '{}' contains duplicate dependencies.",
                stage.name()
            )));
        }
        match stage {
            StageConfig::Process(process) => validate_step_sequence(&process.name, &process.steps)?,
            StageConfig::Branch(branch) => {
                match &branch.branch.when {
                    BranchPredicate::RowCount { .. } => {
                        branch.branch.when.evaluate(0).map_err(QuiltError::usage)?;
                    }
                    BranchPredicate::Parameter { .. } => branch
                        .branch
                        .when
                        .validate_parameter(
                            parameters.ok_or_else(|| {
                                QuiltError::usage(
                                    "parameter predicate requires resolved parameters",
                                )
                            })?,
                            Path::new("."),
                        )
                        .map_err(QuiltError::usage)?,
                }
                let mut targets = HashSet::new();
                for target in branch
                    .branch
                    .then
                    .iter()
                    .chain(branch.branch.r#else.iter().flatten())
                {
                    if target.trim().is_empty() {
                        return Err(QuiltError::usage(format!(
                            "Branch stage '{}' has an empty target",
                            branch.name
                        )));
                    }
                    if !targets.insert(target) {
                        return Err(QuiltError::usage(format!(
                            "Branch stage '{}' contains duplicate target '{}'.",
                            branch.name, target
                        )));
                    }
                }
            }
            StageConfig::Join(join) => {
                let node = &join.join;
                if node.inputs.len() < 2 {
                    return Err(QuiltError::usage(format!(
                        "Join stage '{}' must have at least two inputs.",
                        join.name
                    )));
                }
                let how = node.how.as_deref().unwrap_or("inner").to_ascii_lowercase();
                if !matches!(how.as_str(), "inner" | "left" | "full" | "outer" | "cross") {
                    return Err(QuiltError::usage(format!(
                        "Unsupported join type '{how}' in stage '{}'.",
                        join.name
                    )));
                }
                let has_on = node.on.is_some();
                let has_left = node.left_on.is_some();
                let has_right = node.right_on.is_some();
                if has_on && (has_left || has_right) || has_left != has_right {
                    return Err(QuiltError::usage(format!(
                        "Join stage '{}' must specify exactly one key mode.",
                        join.name
                    )));
                }
                if let Some(keys) = node.on.as_ref() {
                    if keys.is_empty() || keys.iter().any(|key| key.trim().is_empty()) {
                        return Err(QuiltError::usage(format!(
                            "Join stage '{}' has empty join keys.",
                            join.name
                        )));
                    }
                }
                if let (Some(left), Some(right)) = (&node.left_on, &node.right_on) {
                    if left.is_empty()
                        || left.len() != right.len()
                        || left.iter().chain(right).any(|key| key.trim().is_empty())
                    {
                        return Err(QuiltError::usage(format!(
                            "Join stage '{}' has invalid asymmetric join keys.",
                            join.name
                        )));
                    }
                }
                if how == "cross" && (has_on || has_left) {
                    return Err(QuiltError::usage(format!(
                        "Cross join stage '{}' cannot specify join keys.",
                        join.name
                    )));
                }
                if how != "cross" && !has_on && !has_left {
                    return Err(QuiltError::usage(format!(
                        "Join stage '{}' requires join keys unless how is cross.",
                        join.name
                    )));
                }
            }
            StageConfig::Concat(concat) => {
                if concat.concat.inputs.len() < 2 {
                    return Err(QuiltError::usage(format!(
                        "Concat stage '{}' must have at least two inputs.",
                        concat.name
                    )));
                }
                let how = concat
                    .concat
                    .how
                    .as_deref()
                    .unwrap_or("vertical")
                    .to_ascii_lowercase();
                if !matches!(how.as_str(), "vertical" | "v") {
                    return Err(QuiltError::usage(format!(
                        "Unsupported concat type '{how}' in stage '{}'.",
                        concat.name
                    )));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::controllers::resources::ExecutionResources;
    use std::path::PathBuf;
    use std::sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    };

    static TEST_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_temp_dir(label: &str) -> PathBuf {
        for _ in 0..128 {
            let nonce = TEST_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("qlt-run-{label}-{}-{nonce}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create test directory {path:?}: {error}"),
            }
        }
        panic!("could not reserve unique test directory for {label}");
    }
    #[test]
    fn sensitive_preflight_location_matching_respects_step_boundaries() {
        assert!(location_matches(
            "run.yaml: stages[0].steps[1].head: error",
            "stages[0].steps[1]"
        ));
        assert!(!location_matches(
            "run.yaml: stages[0].steps[10].head: error",
            "stages[0].steps[1]"
        ));
    }

    #[test]
    fn sensitive_location_parser_accepts_nested_parameter_paths() {
        assert_eq!(
            diagnostics::parse_step_location("run.stages[0].steps[1].grep.pattern"),
            Some((0, 1))
        );
        assert_eq!(
            diagnostics::parse_step_location("run.stages[2].steps[0].load.paths[0]"),
            Some((2, 0))
        );
        assert_eq!(
            diagnostics::parse_stage_location("run.stages[3].branch.when"),
            Some(3)
        );
        assert_eq!(
            diagnostics::sensitive_location("run.stages[4].name"),
            Some("stages[4]".to_string())
        );
    }

    #[test]
    fn typed_parameter_declarations_and_literals_are_strict() {
        let yaml = "version: 1\nparameters: {count: {type: int, default: 3}, enabled: {type: bool, default: true}}\nstages: [{name: input, steps: []}]";
        let prepared = document::prepare(document::DocumentInput {
            raw: yaml,
            path: Path::new("rules/run.yaml"),
            overrides: &[],
        })
        .unwrap();
        assert_eq!(prepared.parameters["count"].value, ParameterValue::Int(3));
        assert_eq!(
            prepared.parameters["enabled"].value,
            ParameterValue::Bool(true)
        );
        assert!(document::prepare(document::DocumentInput {
            raw: "version: 1\nparameters: {count: {type: int, default: nope}}\nstages: []",
            path: Path::new("run.yaml"),
            overrides: &[],
        })
        .is_err());
    }

    #[test]
    fn preflight_collects_yaml_path_diagnostics() {
        let document = document(
            "version: 1\nstages: [{name: one, from: missing, steps: [{head: {number: nope}}]}]",
        );
        let parameters = HashMap::new();
        let error = document::preflight(&document, &parameters, Path::new("run.yaml")).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("run.yaml: stages[0].dependencies[0]"));
        assert!(rendered.contains("run.yaml: stages[0].steps[0].head"));
    }
    use super::*;

    fn document(yaml: &str) -> RunDocument {
        serde_yml::from_str(yaml).expect("canonical run document should deserialize")
    }

    #[test]
    fn canonical_v1_supports_all_stage_kinds_and_repeated_steps() {
        let doc = document(
            r#"
version: 1
stages:
  - name: input
    steps:
      - load: {paths: [input.csv]}
      - grep: {pattern: ERROR}
      - grep: {pattern: WARN}
  - name: left
    from: input
    steps: [{select: {columns: [id]}}]
  - name: right
    from: input
    steps: [{select: {columns: [id, value]}}]
  - name: joined
    join: {inputs: [left, right], how: inner, on: [id], coalesce: true}
  - name: combined
    concat: {inputs: [left, right], how: vertical}
  - name: selected
    branch:
      input: joined
      when: {row-count: {greater-than: 0}}
      then: [left]
      else: [right]
"#,
        );
        assert_eq!(doc.version, 1);
        assert_eq!(doc.stages.len(), 6);
        assert_eq!(doc.stages[0].name(), "input");
        assert!(matches!(doc.stages[3], StageConfig::Join(_)));
        assert!(matches!(doc.stages[4], StageConfig::Concat(_)));
        assert!(matches!(doc.stages[5], StageConfig::Branch(_)));
        let ProcessStage { steps, .. } = match &doc.stages[0] {
            StageConfig::Process(stage) => stage,
            _ => panic!("expected process stage"),
        };
        assert_eq!(
            document::parse_steps(&Value::Sequence(steps.clone()))
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn dependency_resolution_is_deterministic_and_handles_shared_intermediates() {
        let doc = document(
            r#"
version: 1
stages:
  - name: output
    from: shared
    steps: [{show: {}}]
  - name: shared
    from: input
    steps: [{head: {number: 1}}]
  - name: other
    from: input
    steps: [{tail: {number: 1}}]
  - name: input
    steps: [{load: {paths: [input.csv]}}]
"#,
        );
        let (order, configs) = planner::collect_stage_configs(&doc.stages).unwrap();
        assert_eq!(
            planner::resolve_stage_execution_order(&order, &configs).unwrap(),
            ["input", "shared", "output", "other"]
        );
    }

    #[test]
    fn schema_rejects_unknown_keys_legacy_shapes_and_mapping_steps() {
        assert!(
            serde_yml::from_str::<RunDocument>("version: 1\nunknown: true\nstages: []").is_err()
        );
        assert!(serde_yml::from_str::<RunDocument>(
            "version: 1\nstages: [{name: input, steps: [], extra: true}]"
        )
        .is_err());
        assert!(serde_yml::from_str::<RunDocument>(
            "version: 1\nstages: [{name: input, steps: {load: {paths: [x]}}}]"
        )
        .is_err());
        assert!(serde_yml::from_str::<RunDocument>("version: '1.0.0'\nstages: {}").is_err());
        assert!(serde_yml::from_str::<RunDocument>(
            "version: 1\nstages: [{name: input, type: process, steps: []}]"
        )
        .is_err());
    }

    #[test]
    fn duplicate_names_cycles_and_invalid_steps_are_rejected_before_execution() {
        let duplicate =
            document("version: 1\nstages: [{name: same, steps: []}, {name: same, steps: []}]");
        assert!(planner::collect_stage_configs(&duplicate.stages).is_err());

        let cycle = document(
            "version: 1\nstages: [{name: a, from: b, steps: []}, {name: b, from: a, steps: []}]",
        );
        let (order, configs) = planner::collect_stage_configs(&cycle.stages).unwrap();
        assert!(planner::resolve_stage_execution_order(&order, &configs).is_err());

        let invalid = document(
            "version: 1\nstages: [{name: input, steps: [{load: {paths: [x], typo: true}}]}]",
        );
        assert!(validate_run_document(&invalid, None).is_err());

        let misplaced = document(
            "version: 1\nstages: [{name: input, steps: [{load: {paths: [x]}}, {show: {}}, {head: {number: 1}}]}]",
        );
        assert!(validate_run_document(&misplaced, None).is_err());
    }

    #[test]
    fn public_run_rejects_static_shapes_before_nonexistent_input_io() {
        let root = test_temp_dir("static");
        let input = root.join("does-not-exist.csv");
        let cases = [
            (
                "join",
                "join:\n      inputs: [a]\n      how: inner\n      'on': [id]",
                "at least two inputs",
            ),
            (
                "join-how",
                "join:\n      inputs: [a, b]\n      how: sideways\n      'on': [id]",
                "Unsupported join type",
            ),
            (
                "join-key",
                "join:\n      inputs: [a, b]\n      left-on: [id]",
                "exactly one key mode",
            ),
            (
                "concat",
                "concat:\n      inputs: [a]\n      how: vertical",
                "at least two inputs",
            ),
            (
                "concat-how",
                "concat:\n      inputs: [a, b]\n      how: horizontal",
                "Unsupported concat type",
            ),
            (
                "branch-lhs",
                "branch:\n      input: input\n      when: {row-count: {greater-than: 1, less-than: 2}}\n      then: []",
                "exactly one comparison",
            ),
            (
                "branch-op",
                "branch:\n      input: input\n      when: {row-count: {}}\n      then: []",
                "exactly one comparison",
            ),
            ("step-key", "steps: [{true: {}}]", "single-entry mapping"),
            (
                "step-category",
                "steps: [{run: {}}]",
                "single-entry mapping",
            ),
        ];
        for (name, shape, expected) in cases {
            let load = format!("- load:\n          paths: ['{}']", input.display());
            let yaml = if shape.starts_with("steps:") {
                format!(
                    "version: 1\nstages:\n  - name: input\n    steps:\n      {load}\n      - {}\n",
                    shape.strip_prefix("steps: ").unwrap()
                )
            } else {
                format!("version: 1\nstages:\n  - name: input\n    steps:\n      {load}\n  - name: a\n    from: input\n    steps: []\n  - name: b\n    from: input\n    steps: []\n  - name: bad\n    {shape}\n")
            };
            let path = root.join(format!("{name}.yaml"));
            std::fs::write(&path, yaml).unwrap();
            let mut controller = PipelineState::empty(ExecutionResources::new());
            let error = run(
                &mut controller,
                path.to_str().unwrap(),
                None,
                None,
                &[],
                false,
            )
            .unwrap_err();
            let rendered = error.to_string();
            assert!(!rendered.contains("File not found"), "{name}: {rendered}");
            assert!(
                rendered.contains(expected),
                "{name}: expected '{expected}', got {rendered}"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn automation_steps_reject_wrong_yaml_scalar_types() {
        for (name, yaml) in [
            ("select", "columns: [1]"),
            ("load", "paths: [false]"),
            ("grep", "pattern: 42"),
            ("show", "debug: yes"),
        ] {
            assert!(
                parse_automation_step(name, &serde_yml::from_str(yaml).unwrap()).is_err(),
                "accepted wrong type for {name}"
            );
        }
    }

    #[test]
    fn auto_materialization_keeps_serial_barriers_lazy() {
        let doc = document(
            "version: 1\nstages: [{name: input, steps: [{load: {paths: [input.csv]}}, {sort: {columns: [id]}}]}]",
        );
        let (order, configs) = planner::collect_stage_configs(&doc.stages).unwrap();
        let decisions = materialization::materialization_decisions(&order, &configs);
        let decision = decisions.get("input").unwrap();
        assert!(!decision.materialize);
        assert_eq!(decision.reason, "auto:serial-lazy");
    }

    #[test]
    fn parameter_only_branch_does_not_force_input_materialization() {
        let doc = document(
            "version: 1\nstages: [{name: input, steps: []}, {name: chosen, branch: {input: input, when: {parameter: {name: enabled, equal: true}}, then: [input]}}]",
        );
        let (order, configs) = planner::collect_stage_configs(&doc.stages).unwrap();
        let decisions = materialization::materialization_decisions(&order, &configs);
        assert!(!decisions.get("input").unwrap().materialize);
    }

    #[test]
    fn only_row_evaluating_finalizers_trigger_reuse_materialization() {
        let one = document(
            "version: 1\nstages: [{name: one, steps: [{load: {paths: [input.csv]}}, {show: {}}, {headers: {}}]}]",
        );
        let (order, configs) = planner::collect_stage_configs(&one.stages).unwrap();
        let decision = materialization::materialization_decisions(&order, &configs)
            .remove("one")
            .unwrap();
        assert!(!decision.materialize);

        let two = document(
            "version: 1\nstages: [{name: two, steps: [{load: {paths: [input.csv]}}, {show: {}}, {show: {}}]}]",
        );
        let (order, configs) = planner::collect_stage_configs(&two.stages).unwrap();
        let decision = materialization::materialization_decisions(&order, &configs)
            .remove("two")
            .unwrap();
        assert!(decision.materialize);
        assert_eq!(decision.reason, "auto:multiple-finalizers");
    }

    #[test]
    fn run_fanout_materializes_one_probed_source_evaluation() {
        let dir = test_temp_dir("probe");
        let input = dir.join("input.csv");
        let left_output = dir.join("left.csv");
        let right_output = dir.join("right.parquet");
        std::fs::write(&input, "id,value\n1,a\n2,b\n").unwrap();
        let config = dir.join("run.yaml");
        std::fs::write(
            &config,
            format!(
                "version: 1\nstages:\n  - name: input\n    steps: [{{load: {{paths: [{}]}}}}]\n  - name: left\n    from: input\n    steps: [{{select: {{columns: [id]}}}}, {{dump: {{output: {}}}}}]\n  - name: right\n    from: input\n    steps: [{{select: {{columns: [value]}}}}, {{dumpcache: {{output: {}}}}}]\n",
                input.display(), left_output.display(), right_output.display()
            ),
        )
        .unwrap();
        let probe = Arc::new(AtomicUsize::new(0));
        let resources =
            ExecutionResources::new_in(dir.clone()).with_evaluation_probe(probe.clone());
        let mut controller = PipelineState::empty(resources);
        run(
            &mut controller,
            config.to_str().unwrap(),
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(probe.load(Ordering::SeqCst), 1);
        assert!(left_output.exists());
        assert!(right_output.exists());
        assert!(std::fs::metadata(&right_output).unwrap().len() > 0);
        std::fs::write(
            &config,
            format!(
                "version: 1\nstages:\n  - name: input\n    materialize: never\n    steps: [{{load: {{paths: [{}]}}}}]\n  - name: left\n    from: input\n    steps: [{{select: {{columns: [id]}}}}, {{show: {{}}}}]\n  - name: right\n    from: input\n    steps: [{{select: {{columns: [value]}}}}, {{show: {{}}}}]\n",
                input.display()
            ),
        )
        .unwrap();
        probe.store(0, Ordering::SeqCst);
        let resources =
            ExecutionResources::new_in(dir.clone()).with_evaluation_probe(probe.clone());
        let mut controller = PipelineState::empty(resources);
        run(
            &mut controller,
            config.to_str().unwrap(),
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(probe.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn always_has_one_stage_artifact_and_serial_auto_has_none() {
        let dir = test_temp_dir("artifacts");
        let input = dir.join("input.csv");
        std::fs::write(&input, "id\n1\n").unwrap();
        let config = dir.join("run.yaml");
        let write_config = |policy: &str| {
            std::fs::write(
                &config,
                format!(
                    "version: 1\nstages:\n  - name: input\n    materialize: {policy}\n    steps: [{{load: {{paths: [{}]}}}}]\n",
                    input.display()
                ),
            )
            .unwrap();
        };
        write_config("always");
        let always_resources = ExecutionResources::new_in(dir.clone());
        let always_probe = always_resources.clone();
        let mut controller = PipelineState::empty(always_resources);
        run(
            &mut controller,
            config.to_str().unwrap(),
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(always_probe.tracked_count(), 1);
        write_config("auto");
        let auto_resources = ExecutionResources::new_in(dir.clone());
        let auto_probe = auto_resources.clone();
        let mut controller = PipelineState::empty(auto_resources);
        run(
            &mut controller,
            config.to_str().unwrap(),
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(auto_probe.tracked_count(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn row_count_branch_reuses_one_materialized_source_evaluation() {
        let dir = test_temp_dir("row");
        let input = dir.join("input.csv");
        let config = dir.join("run.yaml");
        let output = dir.join("selected.csv");
        std::fs::write(&input, "id\n1\n2\n").unwrap();
        std::fs::write(
            &config,
            format!(
                "version: 1\nstages:\n  - name: input\n    steps: [{{load: {{paths: [{}]}}}}]\n  - name: route\n    branch:\n      input: input\n      when: {{row-count: {{greater-than: 1}}}}\n      then: [selected]\n      else: [fallback]\n  - name: selected\n    from: input\n    steps: [{{show: {{}}}}]\n  - name: fallback\n    from: input\n    steps: [{{show: {{}}}}]\n",
                input.display()
            ),
        )
        .unwrap();
        let probe = Arc::new(AtomicUsize::new(0));
        let resources =
            ExecutionResources::new_in(dir.clone()).with_evaluation_probe(probe.clone());
        let mut controller = PipelineState::empty(resources);
        run(
            &mut controller,
            config.to_str().unwrap(),
            None,
            Some(output.to_str().unwrap()),
            &[],
            false,
        )
        .unwrap();
        assert_eq!(probe.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_to_string(&output).unwrap(), "id\n1\n2\n");
        let resources = controller.resources();
        assert_eq!(
            resources
                .tracked_paths()
                .iter()
                .filter(|path| path.to_string_lossy().contains("qlt-stage-materialized"))
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn materialization_failure_does_not_leave_stage_artifact() {
        let dir = test_temp_dir("fail");
        let resources = ExecutionResources::new_in(dir.clone());
        let bad = LazyFrame::scan_parquet(dir.join("missing.parquet"), ScanArgsParquet::default())
            .unwrap();
        assert!(materialization::materialize_frame(bad, &resources).is_err());
        assert_eq!(resources.tracked_count(), 0);
        assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn yaml_show_plus_cli_output_reuses_one_source_evaluation() {
        let dir = test_temp_dir("output-reuse");
        let input = dir.join("input.csv");
        let config = dir.join("run.yaml");
        let output = dir.join("output.csv");
        std::fs::write(&input, "id\n1\n").unwrap();
        std::fs::write(
            &config,
            format!(
                "version: 1\nstages: [{{name: input, steps: [{{load: {{paths: [{}]}}}}, {{show: {{}}}}]}}]\n",
                input.display()
            ),
        )
        .unwrap();
        let probe = Arc::new(AtomicUsize::new(0));
        let resources =
            ExecutionResources::new_in(dir.clone()).with_evaluation_probe(probe.clone());
        let mut controller = PipelineState::empty(resources);
        run(
            &mut controller,
            config.to_str().unwrap(),
            None,
            Some(output.to_str().unwrap()),
            &[],
            false,
        )
        .unwrap();
        assert_eq!(probe.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read_to_string(output).unwrap(), "id\n1\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parameter_branch_excludes_unselected_output_route() {
        let doc = document(
            "version: 1\nparameters: {use_left: {type: bool, default: true}}\nstages: [{name: input, steps: []}, {name: route, branch: {input: input, when: {parameter: {name: use_left, equal: true}}, then: [left], else: [right]}}, {name: left, from: input, steps: [{show: {}}]}, {name: right, from: input, steps: [{show: {}}]}]",
        );
        let (order, configs) = planner::collect_stage_configs(&doc.stages).unwrap();
        let values = HashMap::from([(
            "use_left".to_string(),
            ResolvedParameter {
                parameter_type: ParameterType::Bool,
                value: ParameterValue::Bool(true),
                origin: ParameterOrigin::Default,
                secret: false,
            },
        )]);
        let decisions = materialization::materialization_decisions_with_output(
            &order,
            &configs,
            true,
            Some(&values),
            Path::new("."),
        )
        .unwrap();
        assert!(!decisions.get("right").unwrap().materialize);
        assert!(decisions.get("left").unwrap().materialize);
    }

    #[test]
    fn check_and_show_plan_leave_config_directory_artifact_free() {
        let dir = test_temp_dir("plan");
        let input = dir.join("input.csv");
        let config = dir.join("run.yaml");
        std::fs::write(&input, "id\n1\n").unwrap();
        std::fs::write(
            &config,
            format!(
                "version: 1\nstages: [{{name: input, steps: [{{load: {{paths: [{}]}}}}]}}]\n",
                input.display()
            ),
        )
        .unwrap();
        let mut controller = PipelineState::empty(ExecutionResources::new_in(dir.clone()));
        run(
            &mut controller,
            config.to_str().unwrap(),
            None,
            None,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        run_show_plan(config.to_str().unwrap(), "input").unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}
