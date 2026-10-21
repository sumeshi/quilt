use crate::support::*;
use flate2::{write::GzEncoder, Compression};
use polars::prelude::*;
use quilt::controllers::{
    command_model::parse_typed_commands, executor::CommandExecutor, resources::ExecutionResources,
};
use quilt::operations::initializers::load::load_with_ndjson_inference_with_resources;
use serde_json::{json, Value};
use std::io::Write;

#[test]
fn existing_bracketed_paths_remain_literal_across_readers() {
    let root = tempfile::tempdir().unwrap();
    let literal = write(root.path(), "Security[1].jsonl", "{\"id\":1}\n");
    let sibling = write(root.path(), "Security1.jsonl", "{\"id\":2}\n");
    let loaded = rows(&text(&["load", literal.to_str().unwrap()]));
    assert_eq!(loaded[0]["id"], "1");
    let pattern = root.path().join("Security*.jsonl");
    assert_eq!(frame(&["load", pattern.to_str().unwrap()]).height(), 2);
    for source in [&literal, &sibling] {
        let csv = source.with_extension("csv");
        let parquet = source.with_extension("parquet");
        execute(&[
            "load",
            source.to_str().unwrap(),
            "-",
            "dump",
            "-o",
            csv.to_str().unwrap(),
            "-",
            "dumpcache",
            "-o",
            parquet.to_str().unwrap(),
        ])
        .unwrap();
    }
    for extension in ["csv", "parquet"] {
        let path = literal.with_extension(extension);
        let loaded = rows(&text(&["load", path.to_str().unwrap()]));
        assert_eq!(loaded[0]["id"], "1");
    }
}

#[test]
fn schema_union_discovers_late_fields_without_rounding_unsigned_ids() {
    let root = tempfile::tempdir().unwrap();
    let late = write(
        root.path(),
        "late.jsonl",
        format!(
            "{}{{\"id\":18446744073709551615,\"nested\":{{\"late\":true}}}}\n",
            "{\"id\":1}\n".repeat(1001)
        ),
    );
    let sibling = write(
        root.path(),
        "sibling.ndjson",
        "\n{\"id\":9007199254740993,\"nested\":{\"other\":true}}\n\n",
    );
    let result = frame(&[
        "load",
        fixture("jsonl-quality-empty.jsonl").to_str().unwrap(),
        late.to_str().unwrap(),
        sibling.to_str().unwrap(),
        "-",
        "flatten",
    ]);
    assert_eq!(result.height(), 1003);
    assert_eq!(result.column("id").unwrap().dtype(), &DataType::UInt64);
    assert_eq!(
        result.column("id").unwrap().u64().unwrap().get(1001),
        Some(u64::MAX)
    );
    assert_eq!(
        result.column("id").unwrap().u64().unwrap().get(1002),
        Some(9007199254740993)
    );
    assert_eq!(result.column("nested.late").unwrap().null_count(), 1002);
    let error = failure(&[
        "load",
        late.to_str().unwrap(),
        "--infer-schema-length",
        "1000",
    ]);
    assert!(
        error.contains("line 1002") && error.contains("full"),
        "{error}"
    );
    let negative = write(
        root.path(),
        "signed.jsonl",
        "{\"id\":-9223372036854775808}\n{\"id\":9007199254740993}\n",
    );
    let signed = frame(&["load", negative.to_str().unwrap()]);
    assert_eq!(
        signed.column("id").unwrap().i64().unwrap().get(0),
        Some(i64::MIN)
    );
    for data in ["{\"v\":null}\n{\"v\":1}\n", "{\"v\":[]}\n{\"v\":[1]}\n"] {
        let source = write(root.path(), "bounded.jsonl", data);
        assert!(failure(&[
            "load",
            source.to_str().unwrap(),
            "--infer-schema-length",
            "1"
        ])
        .contains("full"));
    }
}

#[test]
fn unrepresentable_or_invalid_records_fail_before_producing_output() {
    let root = tempfile::tempdir().unwrap();
    // Distinct failure modes, not a matrix of synonymous command spellings.
    let cases: &[(&[u8], &str)] = &[
        (b"{\"id\":9007199254740993}\n{\"id\":1.5}\n", "incompatible"),
        (b"{\"id\":18446744073709551616}\n", "range"),
        (
            b"{\"id\":-1}\n{\"id\":18446744073709551615}\n",
            "signed/unsigned",
        ),
        (b"{\"v\":[1,\"a\"]}\n", "incompatible"),
        (b"{\"v\":{}}\n{\"v\":null}\n", "nullable empty"),
        (b"{\"v\":null}\n{\"v\":{}}\n", "nullable empty"),
        (b"{\"v\":[{},null]}\n", "nullable empty"),
        (b"{\"v\":{}}\n{}\n", "nullable empty"),
        (b"{\"v\":\"\xff\"}\n", "UTF-8"),
        (b"[]\n", "must be objects"),
    ];
    for (input, message) in cases {
        let path = write(root.path(), "invalid.jsonl", input);
        let error = failure(&["load", path.to_str().unwrap()]);
        assert!(error.contains(message), "{error}");
        assert!(error.contains("invalid.jsonl"));
    }
    assert!(failure(&[
        "load",
        fixture("jsonl-quality-invalid.jsonl").to_str().unwrap()
    ])
    .contains("line 2"));
    assert!(failure(&[
        "load",
        fixture("jsonl-quality-empty.jsonl").to_str().unwrap()
    ])
    .contains("empty inputs"));
    let first = write(root.path(), "first.jsonl", "{\"nested\":{\"id\":1}}\n");
    let second = write(
        root.path(),
        "second.jsonl",
        "{\"nested\":{\"id\":\"one\"}}\n",
    );
    let error = failure(&["load", first.to_str().unwrap(), second.to_str().unwrap()]);
    assert!(error.contains("nested.id") && error.contains("second.jsonl"));
}

#[test]
fn duplicate_keys_fail_with_record_and_nested_path() {
    let root = tempfile::tempdir().unwrap();
    for (record, key_path) in [
        (r#"{"id":1,"id":2}"#, r#"$["id"]"#),
        (r#"{"event":{"id":1,"id":2}}"#, r#"$["event"]["id"]"#),
        (
            r#"{"items":[{"id":1,"i\u0064":2}]}"#,
            r#"$["items"][0]["id"]"#,
        ),
    ] {
        let path = write(root.path(), "duplicate.jsonl", format!("\n{record}\n"));
        let error = failure(&["load", path.to_str().unwrap()]);
        assert!(
            error.contains("duplicate.jsonl") && error.contains("line 2"),
            "{error}"
        );
        assert!(
            error.contains("duplicate object key") && error.contains(key_path),
            "{error}"
        );
    }
    // Repeated names in independent objects and JSON text inside a string are valid.
    let valid = write(
        root.path(),
        "valid.jsonl",
        "{\"items\":[{\"id\":1},{\"id\":2}],\"text\":\"{\\\"id\\\":1,\\\"id\\\":2}\"}\n",
    );
    assert_eq!(frame(&["load", valid.to_str().unwrap()]).height(), 1);
}

#[test]
fn wildcard_and_regex_shaped_schema_names_remain_literal() {
    let root = tempfile::tempdir().unwrap();
    let path = write(
        root.path(),
        "literal.jsonl",
        "{\"*\":11,\"^x$\":22,\"x\":33,\"__qlt_grep_0\":44,\"__qlt_grep_1\":55,\"event\":{\"*\":66,\"^y$\":77,\"y\":88}}\n",
    );
    let shown = rows(&text(&["load", path.to_str().unwrap(), "-", "flatten"]));
    assert_eq!(shown[0]["*"], "11");
    assert_eq!(shown[0]["^x$"], "22");
    assert_eq!(shown[0]["event.*"], "66");
    assert_eq!(shown[0]["event.^y$"], "77");
    let selected = frame(&["load", path.to_str().unwrap(), "-", "select", "*"]);
    assert_eq!(selected.width(), 1);
    assert_eq!(
        selected.column("*").unwrap().i64().unwrap().get(0),
        Some(11)
    );
    let raw = rows(&text(&["load", path.to_str().unwrap()]));
    assert_eq!(
        serde_json::from_str::<Value>(&raw[0]["event"]).unwrap()["*"],
        66
    );
    let preview = text(&["load", path.to_str().unwrap(), "-", "showtable"]);
    assert!(preview.contains("event.^y$") && preview.contains("77"));
    let filtered = frame(&["load", path.to_str().unwrap(), "-", "grep", "^22$"]);
    assert_eq!(filtered.height(), 1);
    assert_eq!(filtered.width(), 6);
    assert_eq!(
        filtered
            .column("__qlt_grep_0")
            .unwrap()
            .i64()
            .unwrap()
            .get(0),
        Some(44)
    );
    assert_eq!(filtered.column("^x$").unwrap().dtype(), &DataType::Int64);
    let grouped = rows(&text(&[
        "load",
        path.to_str().unwrap(),
        "-",
        "count",
        "*",
        "-",
        "sort",
        "*",
        "-",
        "show",
    ]));
    assert_eq!(grouped[0]["*"], "11");
    assert_eq!(grouped[0]["count"], "1");
    let edited = rows(&text(&[
        "load",
        path.to_str().unwrap(),
        "-",
        "sed",
        "11",
        "matched",
        "--column",
        "*",
        "-",
        "isin",
        "*",
        "matched",
        "-",
        "renamecol",
        "*",
        "^renamed$",
        "-",
        "select",
        "^renamed$",
    ]));
    assert_eq!(edited[0]["^renamed$"], "matched");
}

#[test]
fn nested_csv_and_parquet_roundtrip_preserve_values_and_integer_types() {
    let root = tempfile::tempdir().unwrap();
    let original = json!([
        {"id":18446744073709551615u64,"event":{"deep":{"user":"日本語😀"}},"tags":["comma,quote\"\nline","\u{1b}[31m"],"items":[{"n":9007199254740993u64,"values":[1,null,3]}]},
        {"id":1,"event":null,"tags":[],"items":[]},
        {"id":null,"tags":null}
    ]);
    let source = write(
        root.path(),
        "events.jsonl",
        original
            .as_array()
            .unwrap()
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>(),
    );
    let csv_path = root.path().join("events.csv");
    let cache = root.path().join("events.parquet");
    let shown = text(&["load", source.to_str().unwrap()]);
    let records = rows(&shown);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["id"], u64::MAX.to_string());
    for key in ["event", "tags", "items"] {
        assert_eq!(
            serde_json::from_str::<Value>(&records[0][key]).unwrap(),
            original[0][key]
        );
    }
    assert_eq!(records[1]["tags"], "[]");
    assert_eq!(records[2]["tags"], "");
    execute(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "dump",
        "-o",
        csv_path.to_str().unwrap(),
    ])
    .unwrap();
    assert_eq!(std::fs::read_to_string(csv_path).unwrap(), shown);
    execute(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "dumpcache",
        "-o",
        cache.to_str().unwrap(),
    ])
    .unwrap();
    assert_eq!(text(&["load", cache.to_str().unwrap()]), shown);
    assert_eq!(
        frame(&["load", cache.to_str().unwrap()])
            .column("id")
            .unwrap()
            .dtype(),
        &DataType::UInt64
    );
    let flattened = rows(&text(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "flatten",
        "-",
        "show",
    ]));
    assert_eq!(flattened[0]["event.deep.user"], "日本語😀");
    assert_eq!(flattened[0]["items"], records[0]["items"]);
}

#[test]
fn flatten_and_table_preview_are_bounded_and_never_hide_path_collisions() {
    let root = tempfile::tempdir().unwrap();
    let record = json!({"event":{"deep":{"name":"日本語😀\n\t\u{1b}[31m\u{202e}"}},"tags":["host"],"huge":"漢😀".repeat(100_000),"empty":{},"line\nkey":1,"b":2,"c":3,"d":4,"e":5});
    let source = write(
        root.path(),
        "preview.jsonl",
        format!("{record}\n").repeat(10),
    );
    let preview = text(&["load", source.to_str().unwrap(), "-", "showtable"]);
    assert!(
        preview.contains("event.deep.name")
            && preview.contains("columns omitted")
            && preview.contains("8+")
    );
    assert!(preview.contains("…") && preview.contains("⋮") && preview.contains("\\n\\t"));
    assert!(preview.contains("line\\nkey"));
    assert!(
        !preview.contains('\u{1b}') && !preview.contains('\u{202e}') && !preview.contains('\t')
    );
    assert!(preview.len() < 20_000);
    let selected = text(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "flatten",
        "-",
        "select",
        "event.deep.name,tags,empty",
        "-",
        "showtable",
    ]);
    assert!(selected.contains("[\"host\"]") && selected.contains("{}"));
    for record in [
        json!({"a.b":1,"a":{"b":2}}),
        json!({"a.b":{"c":1},"a":{"b.c":2}}),
    ] {
        let path = write(root.path(), "collision.jsonl", format!("{record}\n"));
        assert!(failure(&["load", path.to_str().unwrap(), "-", "flatten"]).contains("collid"));
    }
    // Parquet cannot encode Struct[0]; failure must not publish partial output.
    let target = root.path().join("unsupported.parquet");
    assert!(failure(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "dumpcache",
        "-o",
        target.to_str().unwrap()
    ])
    .contains("no child field"));
    assert!(!target.exists());
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|p| p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("qlt-stage")));
}

#[test]
fn nested_search_and_literal_keys_preserve_the_typed_frame() {
    let path = fixture("jsonl-quality.jsonl");
    let source = path.to_str().unwrap();
    let result = frame(&["load", source, "-", "contains", "items", "alpha"]);
    assert_eq!(result.height(), 1);
    assert!(matches!(
        result.column("items").unwrap().dtype(),
        DataType::List(_)
    ));
    let searched = frame(&["load", source, "-", "grep", "日本語"]);
    assert_eq!(result, searched);
    let selected = text(&[
        "load",
        source,
        "-",
        "select",
        "xmlns:auto-ns3,literal.dot,\"a,b\",a-b,space key",
        "-",
        "show",
    ]);
    assert_eq!(rows(&selected)[0]["a,b"], "comma");
    assert!(failure(&["load", source, "-", "isin", "items", "alpha"]).contains("nested"));
    assert!(
        failure(&["load", source, "-", "sed", "alpha", "beta", "--column", "items"])
            .contains("nested")
    );
    let replaced = frame(&["load", source, "-", "sed", "alpha", "beta"]);
    assert_eq!(
        replaced.column("items").unwrap(),
        frame(&["load", source]).column("items").unwrap()
    );
}

#[test]
fn gzip_members_integrity_and_execution_owned_spools() {
    let root = tempfile::tempdir().unwrap();
    let mut bytes = Vec::new();
    for data in ["{\"id\":1}\n", "{\"id\":2,\"late\":true}\n"] {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data.as_bytes()).unwrap();
        bytes.extend(encoder.finish().unwrap());
    }
    let source = write(root.path(), "members.jsonl.gz", &bytes);
    let spool_dir = root.path().join("spools");
    std::fs::create_dir(&spool_dir).unwrap();
    let resources = ExecutionResources::new_in(spool_dir.clone());
    let commands = parse_typed_commands(&[
        "load".into(),
        source.to_string_lossy().into_owned(),
        "--source-column".into(),
        "source".into(),
    ])
    .unwrap();
    let mut executor = CommandExecutor::new_with_resources(resources.clone());
    executor.execute_plan(&commands).unwrap();
    let records = rows(&output(executor.finalizer_results()));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["source"], source.to_string_lossy());
    let paths = std::fs::read_dir(&spool_dir)
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect::<Vec<_>>();
    assert!(!paths.is_empty() && paths.iter().all(|p| p.exists()));
    drop(executor);
    drop(resources);
    assert!(paths.iter().all(|p| !p.exists()));
    std::fs::write(&source, &bytes[..bytes.len() - 5]).unwrap();
    let resources = ExecutionResources::new_in(spool_dir.clone());
    assert!(load_with_ndjson_inference_with_resources(
        &[source],
        ",",
        false,
        false,
        None,
        None,
        None,
        &resources
    )
    .err()
    .unwrap()
    .to_string()
    .contains("decompress gzip"));
    assert_eq!(std::fs::read_dir(spool_dir).unwrap().count(), 0);
}
