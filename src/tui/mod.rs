//! Interactive pipeline editing using the same typed commands as the CLI.

mod app;
mod cache;
mod columns;
mod detail;
mod editor;
mod input;
mod output;
mod task;
mod worker;

use crate::controllers::arguments::OutputFormat;
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

#[derive(Clone, Debug)]
pub(super) struct Preview {
    columns: Vec<(String, String)>,
    total_columns: usize,
    input_columns: Vec<(String, String)>,
    rows: Vec<Vec<String>>,
    has_more_rows: bool,
    mode: PreviewMode,
    sample_rows: Option<usize>,
    draft_issue: Option<String>,
    cache: Option<CacheInfo>,
    temp_usage: Option<(u64, Option<u64>)>,
    commands: Vec<TypedCommand>,
    source_version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TaskPhase {
    Loading,
    Computing,
    Inspecting,
    Saving,
    Caching,
}

impl TaskPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Loading => "Reading and validating input",
            Self::Computing => "Computing preview",
            Self::Inspecting => "Reading details",
            Self::Saving => "Saving output",
            Self::Caching => "Creating checkpoint",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Action {
    Preview(PreviewMode),
    DraftPreview {
        mode: PreviewMode,
        context: Vec<TypedCommand>,
        issue: Option<String>,
    },
    Cache,
    ClearCache,
    SaveCache(PathBuf),
    ExportYaml(PathBuf),
    ExportData {
        path: PathBuf,
        format: OutputFormat,
    },
    Inspect {
        mode: PreviewMode,
        row: usize,
        column: Option<usize>,
        source_version: String,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Request {
    id: u64,
    commands: Vec<TypedCommand>,
    action: Action,
}

#[derive(Debug)]
pub(super) enum Outcome {
    Working(TaskPhase),
    Preview(Preview),
    Detail {
        title: String,
        text: String,
        truncated: bool,
    },
    Cancelled,
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
    if args.iter().any(|argument| argument == "--recipe") {
        let mut recipe = None;
        let mut files = Vec::new();
        let mut vars = Vec::new();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--recipe" => {
                    index += 1;
                    let path = args
                        .get(index)
                        .ok_or_else(|| QuiltError::usage("--recipe requires a YAML file"))?;
                    if recipe.replace(PathBuf::from(path)).is_some() {
                        return Err(QuiltError::usage("--recipe may be specified once"));
                    }
                }
                "--var" => {
                    index += 1;
                    let value = args
                        .get(index)
                        .ok_or_else(|| QuiltError::usage("--var requires name=value"))?;
                    if !value.contains('=') {
                        return Err(QuiltError::usage("--var requires name=value"));
                    }
                    vars.push(value.clone());
                }
                value if value.starts_with('-') => {
                    return Err(QuiltError::usage(format!(
                        "unexpected lazy recipe option '{value}'"
                    )))
                }
                value => files.push(PathBuf::from(value)),
            }
            index += 1;
        }
        let recipe = recipe.ok_or_else(|| QuiltError::usage("--recipe requires a YAML file"))?;
        let commands =
            crate::operations::automation::run::load_lazy_recipe(&recipe, &files, &vars)?;
        return app::run_recipe(commands);
    }
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
