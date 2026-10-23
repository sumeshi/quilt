use crate::support::*;
use flate2::{write::GzEncoder, Compression};
use quilt::controllers::command_model::parse_typed_commands;
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

fn qlt_in_temp(directory: &Path, temp: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_qlt"))
        .args(args)
        .current_dir(directory)
        .env("RUST_LOG", "debug")
        .env("TMPDIR", temp)
        .env("TMP", temp)
        .env("TEMP", temp)
        .output()
        .unwrap()
}

fn qlt_with_stdin_in_temp(directory: &Path, temp: &Path, args: &[&str], input: &[u8]) -> Output {
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
fn lazy_requires_terminal_before_loading_and_keeps_help_available() {
    let root = tempfile::tempdir().unwrap();
    let temp = root.path().join("isolated-temp");
    std::fs::create_dir(&temp).unwrap();
    write(root.path(), "invalid.csv", b"value\n\xff\n");
    let help = qlt(root.path(), &["--lazy", "--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("Ctrl+A"));
    assert!(help.contains(":export PATH"));
    let result = qlt_in_temp(
        root.path(),
        &temp,
        &["--lazy", "--rejects", "rejects.jsonl", "invalid.csv"],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8(result.stderr)
        .unwrap()
        .contains("interactive terminal"));
    assert!(!root.path().join("rejects.jsonl").exists());
    assert_temp_directory_empty(&temp);
}

#[test]
fn unicode_short_looking_options_return_unknown_option_errors() {
    let args = ["load", "-🙂"];
    let parsed = parse_typed_commands(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
    assert!(matches!(
        parsed,
        Err(error) if error.to_string().contains("Unknown option '-🙂'")
    ));

    let root = tempfile::tempdir().unwrap();
    let result = qlt(root.path(), &args);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Unknown option '-🙂'"));
}

#[test]
fn cli_errors_drop_execution_owned_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let temp = root.path().join("isolated-temp");
    std::fs::create_dir(&temp).unwrap();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(b"id,message\n1,keep\n").unwrap();
    let compressed = gzip.finish().unwrap();
    write(root.path(), "input.csv.gz", compressed);

    let conversion = qlt_in_temp(
        root.path(),
        &temp,
        &[
            "load",
            "input.csv.gz",
            "-",
            "cast",
            "message",
            "int",
            "-",
            "show",
        ],
    );
    assert!(!conversion.status.success());
    assert!(conversion.stdout.is_empty());
    assert_temp_directory_empty(&temp);

    let stdin_args = [
        "load",
        "--stdin-format",
        "jsonl",
        "--source-column",
        "source",
        "-",
        "show",
        "--format",
        "jsonl",
    ];
    let stdin_data = b"{\"id\":9007199254740993,\"nested\":{\"value\":\"kept\"}}\n";
    let stdin_success = qlt_with_stdin_in_temp(root.path(), &temp, &stdin_args, stdin_data);
    assert!(
        stdin_success.status.success(),
        "{}",
        String::from_utf8_lossy(&stdin_success.stderr)
    );
    let output = String::from_utf8(stdin_success.stdout).unwrap();
    assert!(output.contains("\"source\":\"<stdin>\""), "{output}");
    assert!(output.contains("\"id\":9007199254740993"), "{output}");
    assert!(
        output.contains("\"nested\":{\"value\":\"kept\"}"),
        "{output}"
    );
    assert_temp_directory_empty(&temp);

    let stdin_error =
        qlt_with_stdin_in_temp(root.path(), &temp, &stdin_args, b"{\"id\":1,\"nested\":\n");
    assert!(!stdin_error.status.success());
    assert!(stdin_error.stdout.is_empty());
    assert_temp_directory_empty(&temp);

    #[cfg(target_os = "linux")]
    {
        let full = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_qlt"))
            .args(["load", "input.csv.gz", "-", "show"])
            .current_dir(root.path())
            .env("RUST_LOG", "debug")
            .env("TMPDIR", &temp)
            .env("TMP", &temp)
            .env("TEMP", &temp)
            .stdout(Stdio::from(full))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_temp_directory_empty(&temp);
    }
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
        json!({"version":2,"stages":[{"name":"input","steps":[{"load":{"paths":["input.csv"]}},{"dump":{"output":"completed.csv"}},{"dump":{"output":"existing.csv"}}]}]}),
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
        json!({"version":2,"parameters":{"limit":{"type":"int","required":true}},"stages":[{"name":"input","steps":[{"load":{}},{"head":{"number":{"$param":"limit"}}},{"dump":{"output":"unused.csv"}}]}]}),
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
        json!({"version":2,"parameters":{"replacement":{"type":"string","required":true,"secret":true}},"stages":[{"name":"input","steps":[{"load":{"paths":["input.csv"]}},{"sed":{"column":"value","pattern":"x","replacement":{"$param":"replacement"}}},{"showquery":{}}]}]}),
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
        json!({"version":2,"parameters":{"stage":{"type":"string","secret":true,"default":"private-stage-marker"},"flag":{"type":"bool","secret":true,"default":true}},"stages":[{"name":{"$param":"stage"},"steps":[{"load":{"paths":["input.csv"]}},{"contains":{"column":"missing","pattern":"x","ignore-case":{"$param":"flag"}}}]}]}),
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
        json!({"version":2,"parameters":{"length":{"type":"int","secret":true,"default":0}},"stages":[{"name":"input","steps":[{"load":{"paths":["input.csv"],"infer-schema-length":{"$param":"length"}}}]},{"name":"invalid","steps":[{"head":{"number":"nope"}}]}]}),
    );
    let result = qlt(root.path(), &["run", doc.to_str().unwrap(), "--check"]);
    assert!(!result.status.success());
    let diagnostic = String::from_utf8(result.stderr).unwrap();
    assert!(diagnostic.contains("<redacted diagnostic>") && diagnostic.contains("valid number"));
    assert!(!diagnostic.contains("inference length"));
}
