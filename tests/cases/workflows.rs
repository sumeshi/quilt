use crate::support::*;
use serde_json::json;

#[test]
fn dag_forward_references_join_and_fanout_concat_keep_values_and_order() {
    let root = tempfile::tempdir().unwrap();
    let left = write(root.path(), "left.csv", "*\n1\n2\n");
    let right = write(root.path(), "right.csv", "^key$,label\n1,match\n");
    let config = document(
        root.path(),
        "graph.yaml",
        json!({"version":1,"stages":[
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
        json!({"version":1,"stages":[
            {"name":"a","steps":[{"load":{"paths":[a]}}]},
            {"name":"b","steps":[{"load":{"paths":[b]}}]},
            {"name":"combined","concat":{"inputs":["a","b"],"how":"vertical"}},
            {"name":"result","from":"combined","steps":[{"show":{}}]}
        ]}),
    );
    let error = failure(&["run", config.to_str().unwrap()]);
    assert!(error.contains("field") && error.contains("id"), "{error}");
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
        json!({"version":1,"parameters":{"input":{"type":"path","default":"events.csv"},"limit":{"type":"int","default":1}},"stages":[{"name":"input","steps":[{"load":{"paths":[{"$param":"input"}]}},{"head":{"number":{"$param":"limit"}}},{"show":{}}]}]}),
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
        json!({"version":1,"stages":[{"name":"events","steps":[
            {"load":{"paths":["events.csv"]}}, {"head":{"number":1}}, {"headers":{"plain":true}}, {"show":{}}
        ]}]}),
    );
    assert!(failure(&["run", config.to_str().unwrap()]).contains("multiple stdout"));
    write(root.path(), "existing.csv", "keep\n");
    let config = document(
        root.path(),
        "failure.yaml",
        json!({"version":1,"stages":[{"name":"events","steps":[
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
}

#[test]
fn nested_yaml_and_cli_share_literal_column_and_filter_contracts() {
    let root = tempfile::tempdir().unwrap();
    let source = fixture("jsonl-quality.jsonl");
    let config = document(
        root.path(),
        "nested.yaml",
        json!({"version":1,"stages":[{"name":"events","steps":[
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
