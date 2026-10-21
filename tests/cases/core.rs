use crate::support::*;
use polars::prelude::*;
use quilt::controllers::resources::ExecutionResources;
use quilt::operations::{chainables, finalizers};
use quilt::Pipeline;

#[test]
fn relational_pipeline_filters_preserves_types_and_groups_rows() {
    let input = df!("id" => [3i64, 1, 2, 2], "message" => ["INFO 日本語", "ERROR", "info", "info"])
        .unwrap();
    let mut pipeline = Pipeline::new(input.lazy(), ExecutionResources::new());
    pipeline
        .grep("info", true, false, None)
        .unwrap()
        .sed(Some("message"), "info", "ok", true)
        .unwrap()
        .isin("id", &["2".into(), "3".into()])
        .unwrap()
        .uniq()
        .unwrap()
        .sort(&["id".into()], false)
        .unwrap();
    let result = pipeline.frame().clone().collect().unwrap();
    assert_eq!(
        result
            .column("id")
            .unwrap()
            .i64()
            .unwrap()
            .into_no_null_iter()
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        result.column("message").unwrap().str().unwrap().get(1),
        Some("ok 日本語")
    );
    pipeline
        .renamecol("message", "text")
        .unwrap()
        .contains("text", "日本語", false)
        .unwrap();
    assert_eq!(pipeline.frame().clone().collect().unwrap().height(), 1);
    let counted = chainables::count::count(&result.lazy(), &["message".into()])
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(
        counted.column("count").unwrap().u32().unwrap().sum(),
        Some(2)
    );

    // Equal sort/count keys keep input order, including first-seen groups
    // whose labels are reverse alphabetical. Global sed edits string columns
    // while leaving similarly rendered scalar values typed and unchanged.
    let tied = df!(
        "group" => ["z", "a", "z", "a"],
        "sort_key" => [0i64, 0, 0, 0],
        "row" => ["z1", "a1", "z2", "a2"],
        "enabled" => [true, false, true, false],
        "number" => [12i64, 12, 12, 12],
        "text" => ["12", "22", "12", "22"]
    )
    .unwrap()
    .lazy();
    let sorted = chainables::sort::sort(&tied, &["sort_key".into()], false)
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(
        sorted
            .column("row")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(Option::unwrap)
            .collect::<Vec<_>>(),
        ["z1", "a1", "z2", "a2"]
    );
    let tied_counts = chainables::count::count(&tied, &["group".into()])
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(
        tied_counts
            .column("group")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(Option::unwrap)
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
    let sed = chainables::sed::sed(&tied, None, "12", "X", false)
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(
        sed.column("enabled")
            .unwrap()
            .bool()
            .unwrap()
            .iter()
            .map(Option::unwrap)
            .collect::<Vec<_>>(),
        [true, false, true, false]
    );
    assert_eq!(
        sed.column("number")
            .unwrap()
            .i64()
            .unwrap()
            .into_no_null_iter()
            .collect::<Vec<_>>(),
        [12, 12, 12, 12]
    );
    assert_eq!(
        sed.column("text")
            .unwrap()
            .str()
            .unwrap()
            .iter()
            .map(Option::unwrap)
            .collect::<Vec<_>>(),
        ["X", "22", "X", "22"]
    );

    // Overflow slicing is our boundary; ordinary head/tail are Polars behavior.
    let input = df!("id" => [1, 2]).unwrap().lazy();
    assert_eq!(
        chainables::tail::tail(&input, usize::MAX)
            .unwrap()
            .collect()
            .unwrap()
            .height(),
        2
    );
}

#[test]
fn select_index_ranges_validate_bounds_before_expansion() {
    let input = df!("first" => [1], "second" => [2], "last" => [3])
        .unwrap()
        .lazy();
    let selected = chainables::select::select(&input, &["1:2".into()])
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(selected.get_column_names(), &["first", "second"]);
    assert!(
        chainables::select::select(&input, &["1:9999999999999999999999999999".into()]).is_err()
    );
    assert!(chainables::select::select(&input, &["3:1".into()]).is_err());
}

#[test]
fn extracted_optional_groups_and_exact_size_conversion() {
    let source = df!("value" => [Some("bob@例え.テスト"), Some("invalid"), None], "size" => [Some("1.5KiB"), Some("2MB"), None]).unwrap().lazy();
    let extracted =
        chainables::extract::extract(&source, "value", r"(?P<user>[^@]+)@(?P<domain>.+)").unwrap();
    let result = chainables::parse_size::parse_size_column(&extracted, "size")
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(
        result.column("domain").unwrap().str().unwrap().get(0),
        Some("例え.テスト")
    );
    assert_eq!(result.column("user").unwrap().null_count(), 2);
    assert_eq!(
        result
            .column("size")
            .unwrap()
            .u64()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [Some(1536), Some(2_000_000), None]
    );
    assert!(chainables::extract::extract(&source, "value", "(unnamed)").is_err());
    assert!(chainables::extract::extract(&source, "value", "(?P<size>.+)").is_err());
    for value in ["-1KB", "0.1B", "1XB", "18446744073709551616B"] {
        let input = df!("size" => [value]).unwrap().lazy();
        assert!(
            chainables::parse_size::parse_size_column(&input, "size")
                .unwrap()
                .collect()
                .is_err(),
            "{value}"
        );
    }
}

#[test]
fn datetime_execution_resolves_dst_and_rejects_redacted_row_errors() {
    let root = tempfile::tempdir().unwrap();
    let source = write(root.path(), "events.csv", "when\n2024-11-03 01:30:00\n");
    let mut times = Vec::new();
    for policy in ["earliest", "latest"] {
        let data = frame(&[
            "load",
            source.to_str().unwrap(),
            "-",
            "changetz",
            "when",
            "--from-tz",
            "America/New_York",
            "--to-tz",
            "UTC",
            "--ambiguous",
            policy,
            "-",
            "cast",
            "when",
            "datetime",
        ]);
        times.push(
            data.column("when")
                .unwrap()
                .datetime()
                .unwrap()
                .physical()
                .get(0)
                .unwrap(),
        );
    }
    assert_eq!(times[1] - times[0], 3_600_000_000);
    std::fs::write(&source, "when\nnot-a-date-secret\n").unwrap();
    let error = failure(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "cast",
        "when",
        "datetime",
        "-",
        "show",
    ]);
    assert!(
        error.contains("row 0") && error.contains("redacted"),
        "{error}"
    );
    assert!(!error.contains("not-a-date-secret"));
    // Use typed extreme values directly; no PyArrow fixture construction.
    let extreme = df!("when" => [i64::MIN])
        .unwrap()
        .lazy()
        .with_column(col("when").cast(DataType::Datetime(TimeUnit::Microseconds, None)));
    assert!(chainables::bucket::bucket(&extreme, "when", "7s", None)
        .unwrap()
        .collect()
        .is_err());
}

#[test]
fn numeric_finalizers_keep_null_and_sample_standard_deviation_semantics() {
    let input = df!("value" => [Some(1i64), Some(2), Some(3), Some(4), None])
        .unwrap()
        .lazy();
    assert_eq!(
        finalizers::calc::calc(&input, "value", "sum").unwrap(),
        finalizers::FinalizerResult::Scalar("10".into())
    );
    let result = finalizers::calc::calc(&input, "value", "std").unwrap();
    let finalizers::FinalizerResult::Scalar(std) = result else {
        panic!("scalar expected")
    };
    assert!((std.parse::<f64>().unwrap() - (5f64 / 3.).sqrt()).abs() < 1e-12);
    let empty = input.clone().limit(0);
    assert_eq!(
        finalizers::calc::calc(&empty, "value", "sum").unwrap(),
        finalizers::FinalizerResult::Scalar("null".into())
    );
    let stats = output(&[finalizers::stats::stats(&input).unwrap()]);
    assert!(
        stats.contains("1.2910") && stats.contains("2.5000"),
        "{stats}"
    );
}

#[test]
fn source_inventory_includes_filtered_files_but_does_not_override_reused_names() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "a.jsonl",
        "{\"when\":\"2026-10-15T12:00:00Z\",\"host\":\"a\"}\n",
    );
    write(root.path(), "empty.jsonl", "");
    let pattern = root.path().join("*.jsonl");
    let args = [
        "load",
        pattern.to_str().unwrap(),
        "--source-column",
        "*",
        "-",
        "cast",
        "when",
        "datetime",
        "-",
        "timeinfo",
        "when",
        "--by",
        "*",
        "-",
        "show",
    ];
    let summary = rows(&text(&args));
    assert_eq!(summary.len(), 2);
    assert_eq!(
        summary
            .iter()
            .map(|r| r["rows"].parse::<usize>().unwrap())
            .sum::<usize>(),
        1
    );
    assert!(summary
        .iter()
        .any(|r| r["*"].ends_with("empty.jsonl") && r["rows"] == "0"));
    let result = rows(&text(&[
        "load",
        pattern.to_str().unwrap(),
        "--source-column",
        "source",
        "-",
        "cast",
        "when",
        "datetime",
        "-",
        "select",
        "when,host",
        "-",
        "renamecol",
        "host",
        "source",
        "-",
        "timeinfo",
        "when",
        "--by",
        "source",
        "-",
        "show",
    ]));
    assert_eq!(
        (
            result.len(),
            result[0]["source"].as_str(),
            result[0]["rows"].as_str()
        ),
        (1, "a", "1")
    );
}

#[test]
fn csv_tsv_and_gzip_feed_the_same_typed_pipeline() {
    use std::io::Write;
    let root = tempfile::tempdir().unwrap();
    let content = "id,message\n1,\"日本語,quoted\"\n2,\"line\nbreak\"\n";
    let source = write(root.path(), "data.csv", content);
    let tab = write(
        root.path(),
        "data.tsv",
        "id\tmessage\n1\t日本語,quoted\n2\t\"line\nbreak\"\n",
    );
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(content.as_bytes()).unwrap();
    let compressed = write(root.path(), "data.csv.gz", encoder.finish().unwrap());
    let csv = frame(&["load", source.to_str().unwrap()]);
    let tsv = frame(&["load", tab.to_str().unwrap(), "--separator", "\t"]);
    let gzip = frame(&["load", compressed.to_str().unwrap()]);
    assert_eq!(csv, tsv);
    assert_eq!(csv, gzip);
}

#[test]
fn delta_retains_small_unsigned_extreme_differences_and_rejects_overflow() {
    let input = df!("id" => [u64::MAX-2, u64::MAX]).unwrap().lazy();
    let result = chainables::delta::delta(&input, "id", None)
        .unwrap()
        .collect()
        .unwrap();
    assert_eq!(
        result
            .column("id_delta")
            .unwrap()
            .i64()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [None, Some(2)]
    );
    let input = df!("id" => [i64::MIN, i64::MAX]).unwrap().lazy();
    assert!(chainables::delta::delta(&input, "id", None)
        .unwrap()
        .collect()
        .is_err());
}
