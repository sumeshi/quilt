use crate::support::*;
use polars::prelude::*;
use quilt::controllers::command_model::parse_typed_commands;
use serde_json::json;

#[test]
fn json_string_pointers_normalize_selected_nodes_through_cli_and_yaml() {
    let root = tempfile::tempdir().unwrap();
    let raw_first = concat!(
        "  ",
        r#"{"kind":"ok","a/b":[{"~c":18446744073709551616},{"~c":null}],"array":[1,2],"obj":{"key":1},"huge":18446744073709551615,"untouched":{"$serde_json::private::Number":"literal","$serde_json::private::RawValue":"raw"}}"#
    );
    let raw_rejected = r#" {"kind":7,"a/b":[],"obj":{"key":2},"untouched":{"$serde_json::private::Number":"other","$serde_json::private::RawValue":"still raw"}}"#;
    let source = write(
        root.path(),
        "selected.jsonl",
        format!("{raw_first}\n{raw_rejected}\n"),
    );
    let pointers = ["/a~1b/0/~0c", "/a~1b/1/~0c", "/array", "/obj", "/missing"];
    let rejects = root.path().join("cli-rejects.jsonl");
    let mut cli_args = vec!["load", source.to_str().unwrap()];
    for pointer in pointers {
        cli_args.extend(["--json-string", pointer]);
    }
    cli_args.extend(["--rejects", rejects.to_str().unwrap()]);
    let cli_output = text(&cli_args);

    let yaml_rejects = root.path().join("yaml-rejects.jsonl");
    let config = document(
        root.path(),
        "selected.yaml",
        json!({
            "version": 2,
            "stages": [{"name":"input","steps":[
                {"load": {
                    "paths": [source],
                    "json-string": pointers,
                    "rejects": yaml_rejects
                }},
                {"show": {}}
            ]}]
        }),
    );
    let yaml_output = text(&["run", config.to_str().unwrap()]);
    assert_eq!(rows(&cli_output), rows(&yaml_output));

    let frame_rejects = root.path().join("frame-rejects.jsonl");
    let loaded = frame(&[
        "load",
        source.to_str().unwrap(),
        "--json-string",
        pointers[0],
        "--json-string",
        pointers[1],
        "--json-string",
        pointers[2],
        "--json-string",
        pointers[3],
        "--json-string",
        pointers[4],
        "--rejects",
        frame_rejects.to_str().unwrap(),
    ]);
    assert_eq!(loaded.height(), 1);
    let array = loaded.column("a/b").unwrap().list().unwrap();
    let first_item = array.get_as_series(0).unwrap();
    assert_eq!(
        first_item
            .struct_()
            .unwrap()
            .field_by_name("~c")
            .unwrap()
            .str()
            .unwrap()
            .get(0),
        Some("18446744073709551616")
    );
    assert_eq!(
        first_item
            .struct_()
            .unwrap()
            .field_by_name("~c")
            .unwrap()
            .null_count(),
        1
    );
    assert_eq!(loaded.column("array").unwrap().dtype(), &DataType::String);
    assert_eq!(
        loaded.column("array").unwrap().str().unwrap().get(0),
        Some("[1,2]")
    );
    assert_eq!(loaded.column("obj").unwrap().dtype(), &DataType::String);
    let untouched = loaded.column("untouched").unwrap();
    assert!(matches!(untouched.dtype(), DataType::Struct(_)));
    let fields = untouched.struct_().unwrap();
    assert_eq!(
        fields
            .field_by_name("$serde_json::private::Number")
            .unwrap()
            .str()
            .unwrap()
            .get(0),
        Some("literal")
    );
    assert_eq!(
        fields
            .field_by_name("$serde_json::private::RawValue")
            .unwrap()
            .str()
            .unwrap()
            .get(0),
        Some("raw")
    );

    let cli_report = std::fs::read_to_string(rejects)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let yaml_report = std::fs::read_to_string(yaml_rejects)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(cli_report, yaml_report);
    assert_eq!(cli_report.len(), 1);
    assert_eq!(cli_report[0]["raw_text"].as_str(), Some(raw_rejected));

    // Quarantine without normalization must pass accepted source bytes through
    // the accepted-record spool and keep both reserved-name objects and wide
    // unsigned integers intact.
    let plain_source = write(
        root.path(),
        "plain-selected.jsonl",
        format!(
            "{}\n{}\n",
            raw_first.replace("18446744073709551616", "18446744073709551615"),
            raw_rejected
        ),
    );
    let unnormalized_rejects = root.path().join("unnormalized-rejects.jsonl");
    let unnormalized = frame(&[
        "load",
        plain_source.to_str().unwrap(),
        "--rejects",
        unnormalized_rejects.to_str().unwrap(),
    ]);
    assert_eq!(unnormalized.height(), 1);
    assert_eq!(
        unnormalized.column("huge").unwrap().u64().unwrap().get(0),
        Some(u64::MAX)
    );
    assert!(matches!(
        unnormalized.column("untouched").unwrap().dtype(),
        DataType::Struct(_)
    ));
}

#[test]
fn cache_input_keeps_a_typed_snapshot_reusable_after_source_changes() {
    let root = tempfile::tempdir().unwrap();
    let source = write(
        root.path(),
        "snapshot.jsonl",
        r#"{"id":9007199254740993,"nested":{"name":"before"}}"#,
    );
    let args = parse_typed_commands(&[
        "load".into(),
        source.to_string_lossy().into_owned(),
        "--cache-input".into(),
        "-".into(),
        "select".into(),
        "id,nested".into(),
    ])
    .unwrap();
    let mut executor = quilt::controllers::executor::CommandExecutor::new();
    executor.execute(&args[0]).unwrap();
    write(
        root.path(),
        "snapshot.jsonl",
        r#"{"id":"changed","nested":{"name":"after"}}"#,
    );
    executor.execute(&args[1]).unwrap();
    let frame = executor
        .into_pipeline()
        .unwrap()
        .frame()
        .clone()
        .collect()
        .unwrap();
    assert_eq!(frame.column("id").unwrap().dtype(), &DataType::Int64);
    assert_eq!(
        frame.column("id").unwrap().i64().unwrap().get(0),
        Some(9_007_199_254_740_993)
    );
    assert_eq!(
        frame
            .column("nested")
            .unwrap()
            .struct_()
            .unwrap()
            .field_by_name("name")
            .unwrap()
            .str()
            .unwrap()
            .get(0),
        Some("before")
    );
}
