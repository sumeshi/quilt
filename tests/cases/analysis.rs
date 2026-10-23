use crate::support::*;
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn log_fixture(root: &std::path::Path) -> std::path::PathBuf {
    write(
        root,
        "logs.jsonl",
        concat!(
            "{\"id\":9007199254740993,\"host\":\"edge-a\",\"time\":\"2025-01-01T00:00:00Z\",\"bytes\":100,\"latency\":10.0,\"tags\":[\"alpha\",\"blue\"],\"details\":{\"process\":\"worker\"}}\n",
            "{\"id\":18446744073709551615,\"host\":\"edge-a\",\"time\":\"2025-01-01T00:01:00Z\",\"bytes\":150,\"latency\":20.0,\"tags\":[\"alpha\"],\"details\":{\"process\":\"worker\"}}\n",
            "{\"id\":9007199254740995,\"host\":\"edge-b\",\"time\":\"2025-01-01T00:02:00Z\",\"bytes\":80,\"latency\":5.0,\"tags\":[\"alpha\",\"green\"],\"details\":{\"process\":\"api\"}}\n",
            "{\"id\":9007199254740997,\"host\":\"edge-b\",\"time\":\"2025-01-01T00:03:00Z\",\"bytes\":120,\"latency\":25.0,\"tags\":[\"alphabet\",\"red\"],\"details\":{\"process\":\"api\"}}\n",
            "{\"id\":9007199254740999,\"host\":\"edge-c\",\"time\":\"2025-01-01T00:04:00Z\",\"bytes\":20,\"latency\":2.0,\"tags\":[\"alpha\"],\"details\":{\"process\":\"batch\"}}\n",
        ),
    )
}

fn jsonl_rows(output: &str) -> Vec<Value> {
    output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn qlt_with_input(
    directory: &std::path::Path,
    temp: &std::path::Path,
    args: &[&str],
    input: &[u8],
) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_qlt"))
        .args(args)
        .current_dir(directory)
        .env("RUST_LOG", "debug")
        .env("TMPDIR", temp)
        .env("TMP", temp)
        .env("TEMP", temp)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn filtered_array_log_workflow_and_grouped_delta_match_yaml_and_cli() {
    let root = tempfile::tempdir().unwrap();
    let source = log_fixture(root.path());
    let expression = "bytes >= 80 && latency < 30";
    let config = document(
        root.path(),
        "filtered.yaml",
        json!({"version":2,"stages":[{"name":"logs","steps":[
            {"load":{"paths":[source]}},
            {"filter":{"expression":expression}},
            {"array-contains":{"column":"tags","value":"alpha"}},
            {"array-get":{"column":"tags","index":1,"output":"second_tag"}},
            {"delta":{"column":"bytes","by":["host"]}},
            {"explode":{"columns":["tags"]}},
            {"show":{"format":"jsonl"}}
        ]}]}),
    );
    let yaml = text(&["run", config.to_str().unwrap()]);
    let cli = text(&[
        "load",
        source.to_str().unwrap(),
        "-",
        "filter",
        expression,
        "-",
        "array-contains",
        "tags",
        "alpha",
        "-",
        "array-get",
        "tags",
        "1",
        "--output",
        "second_tag",
        "-",
        "delta",
        "bytes",
        "--by",
        "host",
        "-",
        "explode",
        "tags",
        "-",
        "show",
        "--format",
        "jsonl",
    ]);
    assert_eq!(yaml, cli);

    let rows = jsonl_rows(&cli);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0]["id"].as_u64(), Some(9_007_199_254_740_993));
    assert_eq!(rows[1]["id"].as_u64(), Some(9_007_199_254_740_993));
    assert_eq!(rows[2]["id"].as_u64(), Some(u64::MAX));
    assert_eq!(rows[3]["id"].as_u64(), Some(9_007_199_254_740_995));
    assert_eq!(rows[4]["id"].as_u64(), Some(9_007_199_254_740_995));
    assert_eq!(rows[0]["details"]["process"], "worker");
    assert_eq!(rows[3]["details"]["process"], "api");
    assert_eq!(rows[4]["details"]["process"], "api");
    assert_eq!(rows[0]["tags"], "alpha");
    assert_eq!(rows[1]["tags"], "blue");
    assert_eq!(rows[2]["tags"], "alpha");
    assert_eq!(rows[3]["tags"], "alpha");
    assert_eq!(rows[4]["tags"], "green");
    assert_eq!(rows[0]["second_tag"], "blue");
    assert_eq!(rows[1]["second_tag"], "blue");
    assert!(rows[2]["second_tag"].is_null());
    assert_eq!(rows[3]["second_tag"], "green");
    assert_eq!(rows[4]["second_tag"], "green");
    assert!(rows[0]["bytes_delta"].is_null());
    assert!(rows[1]["bytes_delta"].is_null());
    assert_eq!(rows[2]["bytes_delta"].as_i64(), Some(50));
    assert!(rows[3]["bytes_delta"].is_null());
    assert!(rows[4]["bytes_delta"].is_null());
    assert!(!cli.contains("alphabet") && !cli.contains("9007199254740997"));
}

#[test]
fn grouped_aggregate_and_derive_match_yaml_and_cli_with_exact_ids() {
    let root = tempfile::tempdir().unwrap();
    let source = log_fixture(root.path());
    let metrics = json!([
        {"mode":"count","output":"rows"},
        {"mode":"sum","column":"bytes","output":"total"},
        {"mode":"avg","column":"latency","output":"mean"},
        {"mode":"p95","column":"latency","output":"p95"}
    ]);
    let cli_metrics = [
        "count:rows",
        "sum:bytes:total",
        "avg:latency:mean",
        "p95:latency:p95",
    ];
    let config = document(
        root.path(),
        "aggregate.yaml",
        json!({"version":2,"stages":[{"name":"logs","steps":[
            {"load":{"paths":[source]}},
            {"aggregate":{"metrics":metrics,"by":["host"]}},
            {"derive":{"output":"total_plus_one","expression":"total + 1"}},
            {"show":{"format":"jsonl"}}
        ]}]}),
    );
    let yaml = text(&["run", config.to_str().unwrap()]);
    let mut args = vec!["load", source.to_str().unwrap(), "-", "aggregate"];
    args.extend(cli_metrics);
    args.extend([
        "--by",
        "host",
        "-",
        "derive",
        "total_plus_one",
        "total + 1",
        "-",
        "show",
        "--format",
        "jsonl",
    ]);
    let cli = text(&args);
    assert_eq!(yaml, cli);

    let rows = jsonl_rows(&cli);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["host"], "edge-a");
    assert_eq!(rows[0]["rows"].as_u64(), Some(2));
    assert_eq!(rows[0]["total"].as_u64(), Some(250));
    assert_eq!(rows[0]["total_plus_one"].as_u64(), Some(251));
    assert_eq!(rows[0]["mean"].as_f64(), Some(15.0));
    assert_eq!(rows[0]["p95"].as_f64(), Some(19.5));
    assert_eq!(rows[1]["host"], "edge-b");
    assert_eq!(rows[1]["rows"].as_u64(), Some(2));
    assert_eq!(rows[1]["total"].as_u64(), Some(200));
    assert_eq!(rows[2]["host"], "edge-c");
    assert_eq!(rows[2]["rows"].as_u64(), Some(1));
    assert_eq!(rows[2]["total"].as_u64(), Some(20));
}

#[test]
fn stdin_quarantine_cli_and_yaml_keep_the_same_records_and_report() {
    let root = tempfile::tempdir().unwrap();
    let cli_temp = root.path().join("cli-temp");
    let yaml_temp = root.path().join("yaml-temp");
    std::fs::create_dir(&cli_temp).unwrap();
    std::fs::create_dir(&yaml_temp).unwrap();
    let input = concat!(
        "{\"id\":9007199254740993,\"host\":\"edge-a\",\"details\":{\"process\":\"worker\"}}\n",
        "{\"id\":9007199254740995,\"host\":\"edge-b\",\"details\":{\"process\":\"api\"}}\n",
        "not-json\n",
        "{\"id\":\"bad\",\"host\":\"edge-c\",\"details\":{\"process\":\"batch\"}}\n",
    );
    let cli = qlt_with_input(
        root.path(),
        &cli_temp,
        &[
            "load",
            "--stdin-format",
            "jsonl",
            "--source-column",
            "source",
            "--rejects",
            "cli-rejects.jsonl",
            "-",
            "show",
            "--format",
            "jsonl",
        ],
        input.as_bytes(),
    );
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );

    let config = document(
        root.path(),
        "stdin.yaml",
        json!({"version":2,"stages":[{"name":"logs","steps":[
            {"load":{"stdin-format":"jsonl","source-column":"source","rejects":"yaml-rejects.jsonl"}},
            {"show":{"format":"jsonl"}}
        ]}]}),
    );
    let yaml = qlt_with_input(
        root.path(),
        &yaml_temp,
        &["run", config.to_str().unwrap()],
        input.as_bytes(),
    );
    assert!(
        yaml.status.success(),
        "{}",
        String::from_utf8_lossy(&yaml.stderr)
    );
    assert_eq!(cli.stdout, yaml.stdout);

    let accepted = jsonl_rows(std::str::from_utf8(&cli.stdout).unwrap());
    assert_eq!(accepted.len(), 2);
    assert_eq!(accepted[0]["id"].as_u64(), Some(9_007_199_254_740_993));
    assert_eq!(accepted[0]["source"], "<stdin>");
    assert_eq!(accepted[0]["details"]["process"], "worker");
    assert_eq!(accepted[1]["source"], "<stdin>");

    let cli_report = std::fs::read_to_string(root.path().join("cli-rejects.jsonl")).unwrap();
    let yaml_report = std::fs::read_to_string(root.path().join("yaml-rejects.jsonl")).unwrap();
    assert_eq!(cli_report, yaml_report);
    let rejected = jsonl_rows(&cli_report);
    assert_eq!(rejected.len(), 2);
    assert_eq!(rejected[0]["source"], "<stdin>");
    assert_eq!(rejected[0]["line"], 3);
    assert_eq!(rejected[0]["raw_text"], "not-json");
    assert_eq!(rejected[1]["source"], "<stdin>");
    assert_eq!(rejected[1]["line"], 4);
    assert!(rejected[1]["reason"]
        .as_str()
        .unwrap()
        .contains("incompatible"));
    assert!(std::fs::read_dir(&cli_temp).unwrap().next().is_none());
    assert!(std::fs::read_dir(&yaml_temp).unwrap().next().is_none());
}
