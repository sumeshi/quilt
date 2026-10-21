use super::diagnostics::location_matches;
use super::*;
use crate::controllers::command_model::parse_automation_step;
use crate::controllers::pipeline::PipelineState;
use crate::controllers::resources::ExecutionResources;
use crate::operations::automation::model::{ParameterOrigin, ResolvedParameter};
use polars::prelude::{IntoLazy, LazyFrame, ScanArgsParquet};
use serde_yml::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

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
fn schema_requires_known_fields_and_sequence_steps() {
    assert!(serde_yml::from_str::<RunDocument>("version: 1\nunknown: true\nstages: []").is_err());
    assert!(serde_yml::from_str::<RunDocument>(
        "version: 1\nstages: [{name: input, steps: [], extra: true}]"
    )
    .is_err());
    assert!(serde_yml::from_str::<RunDocument>(
        "version: 1\nstages: [{name: input, steps: {load: {paths: [x]}}}]"
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

    let invalid =
        document("version: 1\nstages: [{name: input, steps: [{load: {paths: [x], typo: true}}]}]");
    let error = document::preflight(&invalid, &HashMap::new(), Path::new("run.yaml")).unwrap_err();
    let rendered = error.to_string();
    assert!(
        rendered.contains("stages[0].steps[0].load") && rendered.contains("typo"),
        "{rendered}"
    );

    let misplaced = document(
            "version: 1\nstages: [{name: input, steps: [{load: {paths: [x]}}, {show: {}}, {head: {number: 1}}]}]",
        );
    let error =
        document::preflight(&misplaced, &HashMap::new(), Path::new("run.yaml")).unwrap_err();
    assert!(error
        .to_string()
        .contains("record step cannot follow a finalizer"));
}

#[test]
fn public_run_rejects_static_shapes_before_nonexistent_input_io() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
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
            None,
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
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
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
        ExecutionResources::new_in(dir.to_path_buf()).with_evaluation_probe(probe.clone());
    let mut controller = PipelineState::empty(resources);
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        None,
        &[],
        false,
        None,
    )
    .unwrap();
    assert_eq!(probe.load(Ordering::SeqCst), 1);
    assert!(left_output.exists());
    assert!(right_output.exists());
    assert!(std::fs::metadata(&right_output).unwrap().len() > 0);
    std::fs::write(
            &config,
            format!(
                "version: 1\nstages:\n  - name: input\n    materialize: never\n    steps: [{{load: {{paths: [{}]}}}}]\n  - name: left\n    from: input\n    steps: [{{select: {{columns: [id]}}}}, {{dump: {{output: {}}}}}]\n  - name: right\n    from: input\n    steps: [{{select: {{columns: [value]}}}}, {{dump: {{output: {}}}}}]\n",
                input.display(), dir.join("left-never.csv").display(), dir.join("right-never.csv").display()
            ),
        )
        .unwrap();
    probe.store(0, Ordering::SeqCst);
    let resources =
        ExecutionResources::new_in(dir.to_path_buf()).with_evaluation_probe(probe.clone());
    let mut controller = PipelineState::empty(resources);
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        None,
        &[],
        false,
        None,
    )
    .unwrap();
    assert_eq!(probe.load(Ordering::SeqCst), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn always_has_one_stage_artifact_and_serial_auto_has_none() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
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
    let always_resources = ExecutionResources::new_in(dir.to_path_buf());
    let always_probe = always_resources.clone();
    let mut controller = PipelineState::empty(always_resources);
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        None,
        &[],
        false,
        None,
    )
    .unwrap();
    assert_eq!(always_probe.tracked_count(), 0);
    assert_eq!(controller.resources().tracked_count(), 1);
    write_config("auto");
    let auto_resources = ExecutionResources::new_in(dir.to_path_buf());
    let auto_probe = auto_resources.clone();
    let mut controller = PipelineState::empty(auto_resources);
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        None,
        &[],
        false,
        None,
    )
    .unwrap();
    assert_eq!(auto_probe.tracked_count(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn row_count_branch_reuses_one_materialized_source_evaluation() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
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
        ExecutionResources::new_in(dir.to_path_buf()).with_evaluation_probe(probe.clone());
    let mut controller = PipelineState::empty(resources);
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        Some(output.to_str().unwrap()),
        &[],
        false,
        Some("selected"),
    )
    .unwrap();
    assert_eq!(probe.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "id\n1\n2\n");
    assert!(!std::fs::read_dir(dir).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("qlt-stage-materialized")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn completed_materialization_releases_its_input_owner() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let base = ExecutionResources::new_in(dir.to_path_buf());
    let (first, first_resources) =
        materialization::materialize_frame(polars::df!("id" => &[1i64, 2]).unwrap().lazy(), &base)
            .unwrap();
    let first_path = first_resources.tracked_paths()[0].clone();
    let (second, second_resources) =
        materialization::materialize_frame(first, &first_resources).unwrap();
    drop(first_resources);
    assert!(!first_path.exists());
    assert_eq!(second.collect().unwrap().height(), 2);
    let second_path = second_resources.tracked_paths()[0].clone();
    assert!(second_path.exists());
    drop(second_resources);
    assert!(!second_path.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn materialization_failure_does_not_leave_stage_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let resources = ExecutionResources::new_in(dir.to_path_buf());
    let bad =
        LazyFrame::scan_parquet(dir.join("missing.parquet"), ScanArgsParquet::default()).unwrap();
    assert!(materialization::materialize_frame(bad, &resources).is_err());
    assert_eq!(resources.tracked_count(), 0);
    assert!(std::fs::read_dir(dir).unwrap().next().is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn cli_output_overrides_yaml_dump_path() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let input = dir.join("input.csv");
    let config = dir.join("run.yaml");
    let yaml_output = dir.join("yaml.csv");
    let cli_output = dir.join("cli.csv");
    std::fs::write(&input, "id\n1\n").unwrap();
    std::fs::write(
            &config,
            format!(
                "version: 1\nstages: [{{name: input, steps: [{{load: {{paths: [{}]}}}}, {{dump: {{output: {}}}}}]}}]\n",
                input.display(),
                yaml_output.display()
            ),
        )
        .unwrap();
    let mut controller = PipelineState::empty(ExecutionResources::new_in(dir.to_path_buf()));
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        Some(cli_output.to_str().unwrap()),
        &[],
        false,
        None,
    )
    .unwrap();
    assert!(cli_output.exists());
    assert!(!yaml_output.exists());
    assert_eq!(std::fs::read_to_string(cli_output).unwrap(), "id\n1\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn yaml_show_plus_cli_output_reuses_one_source_evaluation() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
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
        ExecutionResources::new_in(dir.to_path_buf()).with_evaluation_probe(probe.clone());
    let mut controller = PipelineState::empty(resources);
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        Some(output.to_str().unwrap()),
        &[],
        false,
        None,
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
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
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
    let mut controller = PipelineState::empty(ExecutionResources::new_in(dir.to_path_buf()));
    run(
        &mut controller,
        config.to_str().unwrap(),
        None,
        None,
        &[],
        true,
        None,
    )
    .unwrap();
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 2);
    run_show_plan(config.to_str().unwrap(), "input", None, &[]).unwrap();
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 2);
    let _ = std::fs::remove_dir_all(dir);
}
