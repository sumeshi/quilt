use crate::support::*;
use serde_json::json;

#[test]
fn version_two_diagnostics_identify_fields_before_opening_inputs() {
    let root = tempfile::tempdir().unwrap();
    let old = document(
        root.path(),
        "old.yaml",
        json!({
            "version": 1,
            "parameters": {"input": {"type": "path", "required": true}},
            "stages": [{"name": "input", "steps": [{"load": {"paths": ["missing.csv"]}}]}]
        }),
    );
    for options in [vec![], vec!["--check"], vec!["--show-plan", "input"]] {
        let mut args = vec!["run", old.to_str().unwrap()];
        args.extend(options);
        let error = failure(&args);
        assert!(
            error.contains("version 1 is no longer supported"),
            "{error}"
        );
        assert!(error.contains("version: 2"), "{error}");
        assert!(!error.contains("required parameter"), "{error}");
        assert!(!error.contains("Error reading"), "{error}");
    }
    let config = write(root.path(), "bad.yaml", "version: 2\nstages:\n  - name: input\n    steps:\n      - load: {paths: [missing.csv]}\n      - bucket:\n          column: when\n          interval: 1d\n          strict: wrong\n");
    let error = failure(&["run", config.to_str().unwrap(), "--check"]);
    assert!(
        error.contains("stages[0].steps[1].bucket.strict"),
        "{error}"
    );
    assert!(
        error.contains(&format!("{}:9:19:", config.display())),
        "{error}"
    );
    assert!(!error.contains("bucket.interval"), "{error}");
    for stage in [
        json!({"name": "input", "stepz": []}),
        json!({"name": "input", "steps": [], "join": null}),
    ] {
        let config = document(
            root.path(),
            "bad-stage.yaml",
            json!({"version": 2, "stages": [stage]}),
        );
        let error = failure(&["run", config.to_str().unwrap(), "--check"]);
        assert!(error.contains("stages[0]"), "{error}");
        assert!(!error.contains("stepz.stepz"), "{error}");
        assert!(!error.contains("untagged enum"), "{error}");
    }
}

#[test]
fn version_two_literal_columns_and_structured_metrics_keep_exact_values() {
    let root = tempfile::tempdir().unwrap();
    let input = write(root.path(), "events.jsonl", "{\"a,b\":2,\"host\":\"a\",\"amount:raw\":9007199254740993}\n{\"a,b\":1,\"host\":\"a\",\"amount:raw\":1}\n{\"a,b\":2,\"host\":\"b\",\"amount:raw\":18446744073709551615}\n");
    let config = document(
        root.path(),
        "literal.yaml",
        json!({"version": 2, "stages": [{
            "name": "result", "steps": [
                {"load": {}}, {"select": {"columns": "a,b"}},
                {"sort": {"columns": "a,b"}}, {"count": {"columns": "a,b"}}, {"show": {}}
            ]
        }]}),
    );
    let output = rows(&text(&[
        "run",
        config.to_str().unwrap(),
        input.to_str().unwrap(),
    ]));
    assert_eq!(output.len(), 2);
    assert_eq!(output[0]["a,b"], "2");
    assert_eq!(output[0]["count"], "2");
    let config = document(
        root.path(),
        "metrics.yaml",
        json!({"version": 2, "stages": [{
            "name": "result", "steps": [
                {"load": {}},
                {"aggregate": {"by": "host", "metrics": [
                    {"mode": "sum", "column": "amount:raw", "output": "total:raw"},
                    {"mode": "count", "column": "a,b", "output": "present"}
                ]}}, {"show": {"format": "jsonl"}}
            ]
        }]}),
    );
    let output = text(&["run", config.to_str().unwrap(), input.to_str().unwrap()]);
    let values = output
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values[0]["total:raw"], json!(9007199254740994u64));
    assert_eq!(values[1]["total:raw"], json!(u64::MAX));
    assert_eq!(values[0]["present"], json!(2));
    let side_effect = root.path().join("must-not-exist.csv");
    let invalid = document(
        root.path(),
        "bad-metrics.yaml",
        json!({"version": 2, "stages": [
            {"name": "earlier", "steps": [{"load": {}}, {"dump": {"output": side_effect}}]},
            {"name": "invalid", "from": "earlier", "steps": [{"aggregate": {"by": "host", "metrics": [{"mode": "count", "output": "host"}]}}]}
        ]}),
    );
    assert!(
        failure(&["run", invalid.to_str().unwrap(), input.to_str().unwrap()]).contains("conflicts")
    );
    assert!(!side_effect.exists());
}

#[test]
fn cross_join_preserves_columns_that_resemble_internal_working_names() {
    let root = tempfile::tempdir().unwrap();
    let key = "__qlt_run_cross_join_key";
    let left = write(root.path(), "left.jsonl", format!(
        "{{\"{key}\":\"left\",\"{key}_1\":\"keep-left\",\"id\":1}}\n{{\"{key}\":\"left\",\"{key}_1\":\"keep-left\",\"id\":2}}\n"
    ));
    let right = write(root.path(), "right.jsonl", format!(
        "{{\"{key}\":\"right\",\"{key}_2\":\"keep-right\",\"tag\":\"a\"}}\n{{\"{key}\":\"right\",\"{key}_2\":\"keep-right\",\"tag\":\"b\"}}\n"
    ));
    let config = document(
        root.path(),
        "cross.yaml",
        json!({"version":2,"stages":[
            {"name":"left","steps":[{"load":{"paths":[left]}}]},
            {"name":"right","steps":[{"load":{"paths":[right]}}]},
            {"name":"joined","join":{"inputs":["left","right"],"how":"cross"}},
            {"name":"output","from":"joined","steps":[{"show":{}}]}
        ]}),
    );
    let result = rows(&text(&["run", config.to_str().unwrap()]));
    assert_eq!(result.len(), 4);
    for row in &result {
        assert_eq!(row.len(), 6);
        assert_eq!(row[key], "left");
        assert_eq!(row[&format!("{key}_right")], "right");
        assert_eq!(row[&format!("{key}_1")], "keep-left");
        assert_eq!(row[&format!("{key}_2")], "keep-right");
    }
    assert_eq!(
        result
            .iter()
            .map(|row| (row["id"].as_str(), row["tag"].as_str()))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([("1", "a"), ("1", "b"), ("2", "a"), ("2", "b")])
    );
}

#[test]
fn typed_steps_preserve_repeated_filters_inherited_inputs_and_dump_options() {
    let root = tempfile::tempdir().unwrap();
    let source = write(
        root.path(),
        "input.csv",
        "value,kind\nalpha beta,one\nalpha,two\nbeta,three\n",
    );
    let mut workflow = json!({"version":2,"stages":[
        {"name":"input","steps":[{"load":{}},{"grep":{"pattern":"alpha"}},{"grep":{"pattern":"beta"}}]},
        {"name":"output","from":"input","steps":[{"load":{}},{"dump":{"output":"yaml.csv","separator":";"}}]}
    ]});
    let config = document(root.path(), "typed.yaml", workflow.clone());
    let target = root.path().join("cli.csv");
    execute(&[
        "run",
        config.to_str().unwrap(),
        source.to_str().unwrap(),
        "--output",
        target.to_str().unwrap(),
    ])
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "value;kind\nalpha beta;one\n"
    );
    assert!(!root.path().join("yaml.csv").exists());

    // Explicit empty paths must fail rather than inherit the upstream frame.
    workflow["stages"][1]["steps"][0]["load"] = json!({"paths":[]});
    let config = document(root.path(), "explicit.yaml", workflow);
    failure(&["run", config.to_str().unwrap(), source.to_str().unwrap()]);
    assert!(!root.path().join("yaml.csv").exists());
}

#[test]
fn dag_forward_references_join_and_fanout_concat_keep_values_and_order() {
    let root = tempfile::tempdir().unwrap();
    let left = write(root.path(), "left.csv", "*\n1\n2\n");
    let right = write(root.path(), "right.csv", "^key$,label\n1,match\n");
    let config = document(
        root.path(),
        "graph.yaml",
        json!({"version":2,"stages":[
            {"name":"output","from":"combined","steps":[{"show":{}}]},
            {"name":"combined","concat":{"inputs":["first","second"],"how":"vertical"}},
            {"name":"first","from":"joined","steps":[{"select":{"columns":["*","label"]}}]},
            {"name":"second","from":"joined","steps":[{"select":{"columns":["*","label"]}}]},
            {"name":"joined","join":{"inputs":["left","right"],"how":"left","left-on":["*"],"right-on":["^key$"],"coalesce":true}},
            {"name":"left","steps":[{"load":{"paths":[left]}}]},
            {"name":"right","steps":[{"load":{"paths":[right]}}]}
        ]}),
    );
    let result = rows(&text(&["run", config.to_str().unwrap()]));
    assert_eq!(
        result
            .iter()
            .map(|r| (r["*"].as_str(), r["label"].as_str()))
            .collect::<Vec<_>>(),
        [("1", "match"), ("2", ""), ("1", "match"), ("2", "")]
    );
}

#[test]
fn concat_schema_mismatch_reaches_existing_inputs_and_fails_cleanly() {
    let root = tempfile::tempdir().unwrap();
    let a = write(root.path(), "a.csv", "id\n1\n");
    let b = write(root.path(), "b.csv", "other\n2\n");
    let config = document(
        root.path(),
        "mismatch.yaml",
        json!({"version":2,"stages":[
            {"name":"a","steps":[{"load":{"paths":[a]}}]},
            {"name":"b","steps":[{"load":{"paths":[b]}}]},
            {"name":"combined","concat":{"inputs":["a","b"],"how":"vertical"}},
            {"name":"result","from":"combined","steps":[{"show":{}}]}
        ]}),
    );
    let error = failure(&["run", config.to_str().unwrap()]);
    assert!(
        error.contains("mismatch") && error.contains("id") && error.contains("other"),
        "{error}"
    );
    assert!(!error.contains("No such file"));
}

#[test]
fn typed_parameters_and_relative_paths_execute_the_documented_recipe() {
    let root = tempfile::tempdir().unwrap();
    let source = write(root.path(), "events.csv", "timestamp,message,host\n2026-01-01T12:00:00Z,before,a\n2026-01-02T12:00:00Z,inside,b\n2026-01-03T12:00:00Z,after,c\n");
    let target = root.path().join("timeline.csv");
    let recipe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/timeline.yaml");
    execute(&[
        "run",
        recipe.to_str().unwrap(),
        "--var",
        &format!("input={}", source.display()),
        "--var",
        "time-column=timestamp",
        "--var",
        "start=2026-01-02T00:00:00Z",
        "--var",
        "end=2026-01-02T23:59:59Z",
        "--var",
        &format!("output={}", target.display()),
    ])
    .unwrap();
    let result = rows(&std::fs::read_to_string(target).unwrap());
    assert_eq!(
        (
            result.len(),
            result[0]["message"].as_str(),
            result[0]["host"].as_str()
        ),
        (1, "inside", "b")
    );
    let config = document(
        root.path(),
        "relative.yaml",
        json!({"version":2,"parameters":{"input":{"type":"path","default":"events.csv"},"limit":{"type":"int","default":1}},"stages":[{"name":"input","steps":[{"load":{"paths":[{"$param":"input"}]}},{"head":{"number":{"$param":"limit"}}},{"show":{}}]}]}),
    );
    assert_eq!(rows(&text(&["run", config.to_str().unwrap()])).len(), 1);
    assert_eq!(
        rows(&text(&[
            "run",
            config.to_str().unwrap(),
            "--var",
            "limit=2"
        ]))
        .len(),
        2
    );
    assert!(
        failure(&["run", config.to_str().unwrap(), "--var", "limit=not-an-int"]).contains("int")
    );
}

#[test]
fn output_conflicts_are_preflight_errors_and_later_failures_report_completed_files() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "events.csv", "id\n1\n2\n");
    let config = document(
        root.path(),
        "outputs.yaml",
        json!({"version":2,"stages":[{"name":"events","steps":[
            {"load":{"paths":["events.csv"]}}, {"head":{"number":1}}, {"headers":{"plain":true}}, {"show":{}}
        ]}]}),
    );
    assert!(failure(&["run", config.to_str().unwrap()]).contains("multiple stdout"));
    write(root.path(), "existing.csv", "keep\n");
    let config = document(
        root.path(),
        "failure.yaml",
        json!({"version":2,"stages":[{"name":"events","steps":[
            {"load":{"paths":["events.csv"]}},{"dump":{"output":"completed.csv"}},{"dump":{"output":"existing.csv"}}
        ]}]}),
    );
    let error = failure(&["run", config.to_str().unwrap()]);
    assert!(error.contains("refusing to overwrite"), "{error}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("existing.csv")).unwrap(),
        "keep\n"
    );
    assert_eq!(
        rows(&std::fs::read_to_string(root.path().join("completed.csv")).unwrap()).len(),
        2
    );

    let config = document(
        root.path(),
        "static-error.yaml",
        json!({"version":2,"stages":[
            {"name":"earlier","steps":[
                {"load":{"paths":["events.csv"]}},
                {"dump":{"output":"before-static-error.csv"}}
            ]},
            {"name":"invalid","steps":[
                {"load":{"paths":["events.csv"]}},
                {"cast":{"column":"id","type":"int","input-format":"%F"}}
            ]}
        ]}),
    );
    for check in [false, true] {
        let mut args = vec!["run", config.to_str().unwrap()];
        if check {
            args.push("--check");
        }
        assert!(failure(&args).contains("datetime parsing options apply only"));
        assert!(!root.path().join("before-static-error.csv").exists());
    }
}

#[test]
fn nested_yaml_and_cli_share_literal_column_and_filter_contracts() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture("jsonl-quality.jsonl");
    let config = document(
        root.path(),
        "nested.yaml",
        json!({"version":2,"stages":[{"name":"events","steps":[
            {"load":{"paths":[source]}},{"flatten":{}},{"contains":{"column":"event.deep.user.name","pattern":"日本語"}},
            {"select":{"columns":["event.deep.user.name","a,b","xmlns:auto-ns3","items"]}},{"show":{}}
        ]}]}),
    );
    let yaml = text(&["run", config.to_str().unwrap()]);
    let cli = text(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "flatten",
        "-",
        "contains",
        "event.deep.user.name",
        "日本語",
        "-",
        "select",
        "event.deep.user.name,\"a,b\",xmlns:auto-ns3,items",
        "-",
        "show",
    ]);
    assert_eq!(yaml, cli);
    assert_eq!(rows(&yaml)[0]["a,b"], "comma");
}
