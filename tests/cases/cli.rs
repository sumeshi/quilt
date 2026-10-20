use crate::support::*;
use serde_json::json;
use std::path::Path;
use std::process::{Command, Output};

fn qlt(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qlt"))
        .args(args)
        .current_dir(directory)
        .env("RUST_LOG", "debug")
        .output()
        .unwrap()
}

#[test]
fn process_boundary_separates_machine_output_diagnostics_and_exit_status() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "input.csv", "id,message\n1,keep\n2,drop\n");
    let result = qlt(
        root.path(),
        &["load", "input.csv", "-", "contains", "message", "keep"],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, b"id,message\n1,keep\n");
    assert!(!result.stderr.is_empty());
    let result = qlt(
        root.path(),
        &[
            "load",
            "input.csv",
            "-",
            "cast",
            "message",
            "int",
            "-",
            "show",
        ],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("conversion"));
    write(root.path(), "existing.csv", "keep\n");
    let doc = document(
        root.path(),
        "partial.yaml",
        json!({"version":1,"stages":[{"name":"input","steps":[{"load":{"paths":["input.csv"]}},{"dump":{"output":"completed.csv"}},{"dump":{"output":"existing.csv"}}]}]}),
    );
    let result = qlt(root.path(), &["run", doc.to_str().unwrap()]);
    assert!(!result.status.success() && result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("completed file"));
    assert!(root.path().join("completed.csv").exists());
    assert_eq!(
        std::fs::read(root.path().join("existing.csv")).unwrap(),
        b"keep\n"
    );
}

#[test]
fn invocation_paths_and_output_override_are_relative_to_the_caller() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("rules")).unwrap();
    write(root.path(), "events.csv", "id\n1\n2\n");
    document(
        &root.path().join("rules"),
        "recipe.yaml",
        json!({"version":1,"parameters":{"limit":{"type":"int","required":true}},"stages":[{"name":"input","steps":[{"load":{}},{"head":{"number":{"$param":"limit"}}},{"dump":{"output":"unused.csv"}}]}]}),
    );
    let args = [
        "run",
        "rules/recipe.yaml",
        "events.csv",
        "--var",
        "limit=1",
        "--output",
        "result.csv",
    ];
    let result = qlt(root.path(), &args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert_eq!(
        std::fs::read(root.path().join("result.csv")).unwrap(),
        b"id\n1\n"
    );
    assert!(!root.path().join("rules/unused.csv").exists());
    let repeated = qlt(root.path(), &args);
    assert!(!repeated.status.success());
    assert_eq!(
        std::fs::read(root.path().join("result.csv")).unwrap(),
        b"id\n1\n"
    );
}

#[test]
fn secret_provenance_redacts_debug_plans_and_runtime_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "input.csv", "value\nx\n");
    let doc = document(
        root.path(),
        "plan.yaml",
        json!({"version":1,"parameters":{"replacement":{"type":"string","required":true,"secret":true}},"stages":[{"name":"input","steps":[{"load":{"paths":["input.csv"]}},{"sed":{"column":"value","pattern":"x","replacement":{"$param":"replacement"}}},{"showquery":{}}]}]}),
    );
    // A one-character secret cannot safely be redacted by a substring filter:
    // plan suppression must follow parameter provenance.
    for extra in [vec![], vec!["--show-plan", "input"]] {
        let mut args = vec!["run", doc.to_str().unwrap(), "--var", "replacement=s"];
        args.extend(extra);
        let result = qlt(root.path(), &args);
        assert!(result.status.success());
        let stdout = String::from_utf8(result.stdout).unwrap();
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert!(stdout.contains("<redacted query plan>"));
        assert!(!stdout.contains("\"s\"") && !stderr.contains("replacement=s"));
    }
    let doc = document(
        root.path(),
        "failure.yaml",
        json!({"version":1,"parameters":{"stage":{"type":"string","secret":true,"default":"private-stage-marker"},"flag":{"type":"bool","secret":true,"default":true}},"stages":[{"name":{"$param":"stage"},"steps":[{"load":{"paths":["input.csv"]}},{"contains":{"column":"missing","pattern":"x","ignore-case":{"$param":"flag"}}}]}]}),
    );
    let result = qlt(root.path(), &["run", doc.to_str().unwrap()]);
    assert!(!result.status.success() && result.stdout.is_empty());
    let diagnostic = String::from_utf8(result.stderr).unwrap();
    assert!(diagnostic.contains("<redacted diagnostic>"), "{diagnostic}");
    assert!(
        !diagnostic.contains("private-stage-marker") && !diagnostic.contains("column not found")
    );
    let doc = document(
        root.path(),
        "preflight.yaml",
        json!({"version":1,"parameters":{"length":{"type":"int","secret":true,"default":0}},"stages":[{"name":"input","steps":[{"load":{"paths":["input.csv"],"infer-schema-length":{"$param":"length"}}}]},{"name":"invalid","steps":[{"head":{"number":"nope"}}]}]}),
    );
    let result = qlt(root.path(), &["run", doc.to_str().unwrap(), "--check"]);
    assert!(!result.status.success());
    let diagnostic = String::from_utf8(result.stderr).unwrap();
    assert!(diagnostic.contains("<redacted diagnostic>") && diagnostic.contains("valid number"));
    assert!(!diagnostic.contains("inference length"));
}
