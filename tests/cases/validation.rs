use crate::support::*;
use flate2::{write::GzEncoder, Compression};
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn qlt(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qlt"))
        .args(args)
        .current_dir(directory)
        .env("RUST_LOG", "debug")
        .output()
        .unwrap()
}

fn assert_temp_directory_empty(directory: &Path) {
    let artifacts = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        artifacts.is_empty(),
        "left temporary artifacts: {artifacts:?}"
    );
}

#[test]
fn contains_searches_literal_metacharacters_in_nested_strings() {
    let root = tempfile::tempdir().unwrap();
    let source = write(
        root.path(),
        "nested.jsonl",
        b"{\"event\":{\"message\":\"a.b [inside]\"}}\n{\"event\":{\"message\":\"plain text\"}}\n",
    );

    for pattern in [".", "["] {
        let result = frame(&[
            "load",
            source.to_str().unwrap(),
            "-",
            "contains",
            "event",
            pattern,
        ]);
        assert_eq!(result.height(), 1, "pattern {pattern:?}");
        assert_eq!(
            result
                .column("event")
                .unwrap()
                .struct_()
                .unwrap()
                .field_by_name("message")
                .unwrap()
                .str()
                .unwrap()
                .get(0),
            Some("a.b [inside]")
        );
    }
}

#[test]
fn invalid_search_and_static_commands_preflight_before_any_dump() {
    let root = tempfile::tempdir().unwrap();
    let input = write(root.path(), "input.csv", "value,date\n1,2024-01-01\n");
    let invalid_steps = [
        (json!({"grep": {"pattern": "["}}), "Invalid grep regex"),
        (json!({"filter": {"expression": "value >"}}), "filter"),
        (
            json!({"cast": {"column": "date", "type": "datetime", "input-format": "%Q"}}),
            "invalid datetime format",
        ),
    ];

    for (index, (invalid, expected_diagnostic)) in invalid_steps.into_iter().enumerate() {
        let earlier = format!("earlier-{index}.csv");
        let config = document(
            root.path(),
            &format!("invalid-{index}.yaml"),
            json!({
                "version": 2,
                "stages": [{"name": "input", "steps": [
                    {"load": {"paths": [input]}},
                    {"dump": {"output": earlier}},
                    invalid,
                    {"show": {}}
                ]}]
            }),
        );
        for options in [vec!["--check"], vec![]] {
            let mut args = vec!["run", config.to_str().unwrap()];
            args.extend(options);
            let result = qlt(root.path(), &args);
            assert!(!result.status.success(), "case {index} unexpectedly passed");
            assert!(result.stdout.is_empty(), "case {index} wrote stdout");
            let error = String::from_utf8_lossy(&result.stderr);
            assert!(
                error.contains(expected_diagnostic),
                "case {index} diagnostic: {error}"
            );
            assert!(
                !root.path().join(&earlier).exists(),
                "case {index} created a dump before preflight rejected the workflow"
            );
        }
    }

    // The same malformed regular expression is rejected by the CLI adapter,
    // including when inverted, instead of being deferred until row execution.
    let result = qlt(root.path(), &["grep", "--invert-match", "["]);
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("Invalid grep regex"), "{error}");
}

#[test]
fn managed_temp_limit_is_reported_and_failed_spills_are_cleaned_up() {
    let root = tempfile::tempdir().unwrap();
    let temp = root.path().join("isolated-temp");
    std::fs::create_dir(&temp).unwrap();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(b"id,message\n1,large-enough-to-spill\n")
        .unwrap();
    write(root.path(), "input.csv.gz", gzip.finish().unwrap());

    let gzip_failure = Command::new(env!("CARGO_BIN_EXE_qlt"))
        .args(["load", "input.csv.gz", "-", "show"])
        .current_dir(root.path())
        .env("RUST_LOG", "debug")
        .env("TMPDIR", &temp)
        .env("TMP", &temp)
        .env("TEMP", &temp)
        .env("QLT_TEMP_LIMIT", "1")
        .output()
        .unwrap();
    assert!(!gzip_failure.status.success());
    assert!(gzip_failure.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&gzip_failure.stderr).contains("QLT_TEMP_LIMIT (1 bytes)"),
        "{}",
        String::from_utf8_lossy(&gzip_failure.stderr)
    );
    assert_temp_directory_empty(&temp);

    let mut child = Command::new(env!("CARGO_BIN_EXE_qlt"))
        .args(["load", "--stdin-format", "jsonl", "-", "show"])
        .current_dir(root.path())
        .env("RUST_LOG", "debug")
        .env("TMPDIR", &temp)
        .env("TMP", &temp)
        .env("TEMP", &temp)
        .env("QLT_TEMP_LIMIT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"id\":1,\"message\":\"large-enough-to-spill\"}\n")
        .unwrap();
    let stdin_failure = child.wait_with_output().unwrap();
    assert!(!stdin_failure.status.success());
    assert!(stdin_failure.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&stdin_failure.stderr).contains("QLT_TEMP_LIMIT (1 bytes)"),
        "{}",
        String::from_utf8_lossy(&stdin_failure.stderr)
    );
    assert_temp_directory_empty(&temp);
}
