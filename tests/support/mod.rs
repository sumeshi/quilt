use polars::prelude::*;
use quilt::controllers::command_model::parse_typed_commands;
use quilt::controllers::executor::CommandExecutor;
use quilt::operations::finalizers::{write_stdout, FinalizerResult};
use quilt::QuiltError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub type Row = BTreeMap<String, String>;

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn execute(args: &[&str]) -> Result<CommandExecutor, QuiltError> {
    let commands = parse_typed_commands(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())?;
    let mut executor = CommandExecutor::new();
    executor.execute_plan(&commands)?;
    Ok(executor)
}

pub fn output(results: &[FinalizerResult]) -> String {
    let mut bytes = Vec::new();
    for result in results {
        if !matches!(result, FinalizerResult::Stderr(_)) {
            write_stdout(result, &mut bytes).unwrap();
        }
    }
    String::from_utf8(bytes).unwrap()
}

pub fn text(args: &[&str]) -> String {
    output(execute(args).unwrap().finalizer_results())
}

pub fn rows(text: &str) -> Vec<Row> {
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let headers = reader.headers().unwrap().clone();
    reader
        .records()
        .map(|record| {
            headers
                .iter()
                .zip(record.unwrap().iter())
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .collect()
}

pub fn frame(args: &[&str]) -> DataFrame {
    // execute() adds an implicit show; callers that need a frame avoid the CSV
    // boundary by executing typed commands individually.
    let commands =
        parse_typed_commands(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
    let mut executor = CommandExecutor::new();
    for command in &commands {
        executor.execute(command).unwrap();
    }
    executor
        .into_pipeline()
        .unwrap()
        .into_parts()
        .0
        .collect()
        .unwrap()
}

pub fn write(root: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = root.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

pub fn document(root: &Path, name: &str, value: serde_json::Value) -> PathBuf {
    write(root, name, serde_json::to_vec(&value).unwrap())
}

pub fn failure(args: &[&str]) -> String {
    match execute(args) {
        Ok(_) => panic!("expected failure: {args:?}"),
        Err(error) => error.to_string(),
    }
}
