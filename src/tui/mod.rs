//! Interactive pipeline editing using the same typed commands as the CLI.

mod app;
mod cache;
mod input;
mod worker;

use crate::controllers::command_model::{parse_typed_commands, TypedCommand};
use crate::QuiltError;
use std::path::PathBuf;

pub(super) const SAMPLE_ROWS: usize = 10_000;
pub(super) const PREVIEW_ROWS: usize = 100;
pub(super) const DEFAULT_CACHE_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PreviewMode {
    #[default]
    Sample,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CacheInfo {
    bytes: u64,
    steps: usize,
}

#[derive(Debug)]
pub(super) struct Preview {
    columns: Vec<(String, String)>,
    total_columns: usize,
    rows: Vec<Vec<String>>,
    truncated: bool,
    mode: PreviewMode,
    sample_rows: Option<usize>,
    cache: Option<CacheInfo>,
}

#[derive(Clone, Debug)]
pub(super) enum Action {
    Preview(PreviewMode),
    Cache,
    ClearCache,
    SaveCache(PathBuf),
    ExportYaml(PathBuf),
}

#[derive(Clone, Debug)]
pub(super) struct Request {
    id: u64,
    commands: Vec<TypedCommand>,
    action: Action,
}

#[derive(Debug)]
pub(super) enum Outcome {
    Preview(Preview),
    Cached(CacheInfo),
    CacheCleared,
    Saved(PathBuf),
}

#[derive(Debug)]
pub(super) struct Event {
    id: u64,
    result: Result<Outcome, QuiltError>,
}

/// Start a terminal session. Arguments are the usual `load` arguments.
pub fn run(args: &[String]) -> Result<(), QuiltError> {
    let (initial, initial_source) = if args.is_empty() {
        (Vec::new(), None)
    } else {
        let mut argv = vec!["load".to_owned()];
        argv.extend_from_slice(args);
        let source = argv
            .iter()
            .map(|value| input::quote_argument(value))
            .collect::<Vec<_>>()
            .join(" ");
        (parse_typed_commands(&argv)?, Some(source))
    };
    app::run(initial, initial_source)
}
