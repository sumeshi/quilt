use super::{
    Action, Event, Outcome, Preview, PreviewMode, Request, DEFAULT_CACHE_LIMIT, PREVIEW_ROWS,
    SAMPLE_ROWS,
};
use crate::controllers::arguments::LoadArgs;
use crate::controllers::command_model::{CommandCategory, TypedCommand};
use crate::controllers::executor::CommandExecutor;
use crate::controllers::pipeline::Pipeline;
use crate::controllers::resources::ExecutionResources;
use crate::QuiltError;
use polars::prelude::*;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

const ACTION_QUEUE_CAPACITY: usize = 16;
const MAX_COLUMNS: usize = 128;
const MAX_CELL_CHARS: usize = 512;
const SAMPLE_BYTES: usize = 32 * 1024 * 1024;
const SAMPLE_BATCH_ROWS: usize = 256;

struct Schedule {
    actions: VecDeque<Request>,
    preview: Option<Request>,
    closed: bool,
}

pub(super) struct Worker {
    schedule: Arc<(Mutex<Schedule>, Condvar)>,
    events: mpsc::Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(super) fn new() -> Self {
        let schedule = Arc::new((
            Mutex::new(Schedule {
                actions: VecDeque::new(),
                preview: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker_schedule = Arc::clone(&schedule);
        let (event_tx, event_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("quilt-tui-worker".into())
            .spawn(move || worker_loop(worker_schedule, event_tx))
            .expect("failed to start TUI worker");
        Self {
            schedule,
            events: event_rx,
            thread: Some(thread),
        }
    }

    pub(super) fn submit(&self, request: Request) -> Result<(), QuiltError> {
        let (lock, ready) = &*self.schedule;
        let mut schedule = lock
            .lock()
            .map_err(|_| QuiltError::operation("TUI worker", "scheduler stopped"))?;
        enqueue(&mut schedule, request)?;
        ready.notify_one();
        Ok(())
    }

    pub(super) fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

fn enqueue(schedule: &mut Schedule, request: Request) -> Result<(), QuiltError> {
    if schedule.closed {
        return Err(QuiltError::operation("TUI worker", "worker has stopped"));
    }
    if matches!(&request.action, Action::Preview(_)) {
        schedule.preview = Some(request);
    } else {
        if schedule.actions.len() >= ACTION_QUEUE_CAPACITY {
            return Err(QuiltError::usage("TUI worker is busy; try again shortly"));
        }
        schedule.actions.push_back(request);
    }
    Ok(())
}

impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, ready) = &*self.schedule;
        if let Ok(mut schedule) = lock.lock() {
            schedule.closed = true;
            schedule.preview = None;
            schedule.actions.clear();
            ready.notify_one();
        }
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Clone)]
struct LoadedSample {
    pipeline: Pipeline,
    rows: usize,
}

struct LoadedSource {
    args: LoadArgs,
    fingerprint: String,
    full: Pipeline,
    sample: Option<LoadedSample>,
}

struct FramePreview {
    columns: Vec<(String, String)>,
    rows: Vec<Vec<String>>,
    truncated: bool,
}

struct Engine {
    source: Option<LoadedSource>,
    cache: super::cache::SessionCache,
    sample_budget: usize,
}

impl Engine {
    fn new() -> Self {
        Self::with_sample_budget(SAMPLE_BYTES)
    }

    fn with_sample_budget(sample_budget: usize) -> Self {
        Self {
            source: None,
            cache: super::cache::SessionCache::new(DEFAULT_CACHE_LIMIT),
            sample_budget,
        }
    }

    fn process(&mut self, request: Request) -> Result<Outcome, QuiltError> {
        match request.action {
            Action::Cache => {
                let pipeline = self.full_pipeline(&request.commands)?;
                let info = self.cache.store(request.commands, pipeline)?;
                Ok(Outcome::Cached(info))
            }
            Action::ClearCache => {
                self.cache.clear();
                Ok(Outcome::CacheCleared)
            }
            Action::ExportYaml(path) => {
                let yaml = crate::controllers::yaml_export::workflow_yaml(&request.commands)?;
                write_new(&path, yaml.as_bytes())?;
                Ok(Outcome::Saved(path))
            }
            Action::SaveCache(path) => {
                let pipeline = self.full_pipeline(&request.commands)?;
                let requested = path.to_string_lossy();
                let result = pipeline.dumpcache_result(Some(&requested))?;
                match result {
                    crate::operations::finalizers::FinalizerResult::File(saved_path) => {
                        Ok(Outcome::Saved(saved_path))
                    }
                    _ => Err(QuiltError::operation(
                        "write TUI cache",
                        "dumpcache did not return a file path",
                    )),
                }
            }
            Action::Preview(mode) => {
                let (pipeline, sample_rows) = if mode == PreviewMode::Sample {
                    let pipeline = self.sample_pipeline(&request.commands)?;
                    let rows = self
                        .source
                        .as_ref()
                        .and_then(|source| source.sample.as_ref().map(|sample| sample.rows));
                    (pipeline, rows)
                } else {
                    (self.full_pipeline(&request.commands)?, None)
                };
                let preview = preview_frame(pipeline.frame())?;
                Ok(Outcome::Preview(Preview {
                    columns: preview.columns,
                    rows: preview.rows,
                    truncated: preview.truncated,
                    mode,
                    cache: self.cache.info(),
                    sample_rows,
                }))
            }
        }
    }

    fn validate(commands: &[TypedCommand]) -> Result<&LoadArgs, QuiltError> {
        if commands
            .iter()
            .any(|c| c.category() == CommandCategory::Automation)
        {
            return Err(QuiltError::usage("run cannot be used in a TUI preview"));
        }
        if commands
            .iter()
            .any(|c| c.category() == CommandCategory::Finalizer)
        {
            return Err(QuiltError::usage(
                "finalizer commands cannot run in a TUI preview",
            ));
        }
        let loads: Vec<_> = commands
            .iter()
            .enumerate()
            .filter(|(_, c)| c.category() == CommandCategory::Initializer)
            .collect();
        if loads.len() != 1 || loads[0].0 != 0 {
            return Err(QuiltError::usage(
                "load must be the first and only initializer",
            ));
        }
        let TypedCommand::Load(args) = &commands[0] else {
            unreachable!()
        };
        if args.stdin_format.is_some() || args.rejects.is_some() {
            return Err(QuiltError::usage(
                "TUI previews do not support load stdin or rejects",
            ));
        }
        Ok(args)
    }

    fn ensure_source(
        &mut self,
        args: &LoadArgs,
        need_sample: bool,
    ) -> Result<Pipeline, QuiltError> {
        let fingerprint = source_fingerprint(args)?;
        let current = self
            .source
            .as_ref()
            .is_some_and(|source| source.args == *args && source.fingerprint == fingerprint);
        if !current {
            self.cache.clear();
            let pipeline = Pipeline::load_args(args, ExecutionResources::new())?;
            self.source = Some(LoadedSource {
                args: args.clone(),
                fingerprint,
                full: pipeline,
                sample: None,
            });
        }
        let pipeline = self
            .source
            .as_ref()
            .expect("source just loaded")
            .full
            .clone();
        if need_sample
            && self
                .source
                .as_ref()
                .is_some_and(|source| source.sample.is_none())
        {
            let (sample, sample_rows) =
                collect_bounded_sample(pipeline.frame().clone(), self.sample_budget)?;
            let sample_pipeline = Pipeline::new(sample.lazy(), pipeline.resources())
                .with_source_inventory(pipeline.source_inventory());
            self.source.as_mut().expect("source just loaded").sample = Some(LoadedSample {
                pipeline: sample_pipeline,
                rows: sample_rows,
            });
        }
        Ok(pipeline)
    }

    fn sample_pipeline(&mut self, commands: &[TypedCommand]) -> Result<Pipeline, QuiltError> {
        let args = Self::validate(commands)?;
        let _ = self.ensure_source(args, true)?;
        // A cached checkpoint is only valid while its typed prefix remains
        // unchanged. Sample previews never use its full-data frame, but still
        // validate the checkpoint so edits discard stale state promptly.
        let _ = self.cache.restore(commands);
        let source = self
            .source
            .as_ref()
            .and_then(|source| source.sample.as_ref())
            .expect("sample just loaded")
            .clone();
        let mut executor = CommandExecutor::from_pipeline(source.pipeline);
        for command in &commands[1..] {
            executor.execute(command)?;
        }
        executor
            .into_pipeline()
            .ok_or_else(|| QuiltError::usage("load data before previewing"))
    }

    fn full_pipeline(&mut self, commands: &[TypedCommand]) -> Result<Pipeline, QuiltError> {
        let args = Self::validate(commands)?;
        let _ = self.ensure_source(args, false)?;
        if let Some((prefix, pipeline)) = self.cache.restore(commands) {
            let mut executor = CommandExecutor::from_pipeline(pipeline);
            for command in &commands[prefix..] {
                executor.execute(command)?;
            }
            return executor
                .into_pipeline()
                .ok_or_else(|| QuiltError::usage("load data before previewing"));
        }
        let source = self
            .source
            .as_ref()
            .expect("source just loaded")
            .full
            .clone();
        let mut executor = CommandExecutor::from_pipeline(source);
        for command in &commands[1..] {
            executor.execute(command)?;
        }
        executor
            .into_pipeline()
            .ok_or_else(|| QuiltError::usage("load data before previewing"))
    }
}

fn worker_loop(schedule: Arc<(Mutex<Schedule>, Condvar)>, events: mpsc::Sender<Event>) {
    let mut engine = Engine::new();
    loop {
        let request = {
            let (lock, ready) = &*schedule;
            let mut state = match lock.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            while state.actions.is_empty() && state.preview.is_none() && !state.closed {
                state = match ready.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
            if state.closed {
                return;
            }
            let action_id = state.actions.front().map(|r| r.id);
            let preview_id = state.preview.as_ref().map(|r| r.id);
            if preview_id.is_some() && (action_id.is_none() || preview_id < action_id) {
                state.preview.take()
            } else {
                state.actions.pop_front()
            }
        };
        let Some(request) = request else { continue };
        let id = request.id;
        let result = catch_unwind(AssertUnwindSafe(|| engine.process(request)))
            .unwrap_or_else(|_| {
                Err(QuiltError::operation(
                    "TUI worker",
                    "background operation panicked",
                ))
            })
            .map_err(bound_error);
        if events.send(Event { id, result }).is_err() {
            return;
        }
    }
}

fn bound_error(error: QuiltError) -> QuiltError {
    let message = bound_cell(error.to_string());
    QuiltError::operation("TUI operation", message)
}

fn source_fingerprint(args: &LoadArgs) -> Result<String, QuiltError> {
    let paths = crate::operations::initializers::load::expand_input_paths(&args.paths)?;
    let mut records = Vec::with_capacity(paths.len());
    for path in paths {
        let canonical = std::fs::canonicalize(&path).map_err(|error| {
            QuiltError::io(
                "resolve TUI input",
                Some(path.display().to_string()),
                error.to_string(),
            )
        })?;
        let metadata = std::fs::metadata(&path).map_err(|error| {
            QuiltError::io(
                "inspect TUI input",
                Some(path.display().to_string()),
                error.to_string(),
            )
        })?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        records.push((
            path.to_string_lossy().into_owned(),
            canonical.to_string_lossy().into_owned(),
            metadata.len(),
            modified,
            metadata.is_file(),
        ));
    }
    let configured = args
        .paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    Ok(format!("{configured:?}|{records:?}"))
}

fn collect_bounded_sample(
    frame: LazyFrame,
    byte_limit: usize,
) -> Result<(DataFrame, usize), QuiltError> {
    use std::num::NonZeroUsize;
    use std::sync::{Arc, Mutex};

    struct Sample {
        batches: Vec<DataFrame>,
        rows: usize,
        bytes: usize,
    }
    let sample = Arc::new(Mutex::new(Sample {
        batches: Vec::new(),
        rows: 0,
        bytes: 0,
    }));
    let callback_sample = Arc::clone(&sample);
    let sink = frame.clone().sink_batches(
        PlanCallback::new(move |batch: DataFrame| {
            let mut sample = callback_sample.lock().map_err(|_| {
                polars::error::PolarsError::ComputeError("TUI sample lock poisoned".into())
            })?;
            if batch.height() == 0 || sample.rows >= SAMPLE_ROWS || sample.bytes >= byte_limit { return Ok(true); }
            let batch_bytes = batch.estimated_size();
            let estimated_row_bytes = batch_bytes.div_ceil(batch.height()).max(1);
            let row_budget = (byte_limit - sample.bytes) / estimated_row_bytes;
            if row_budget == 0 {
                if sample.rows == 0 {
                    return Err(polars::error::PolarsError::ComputeError(
                        format!("first TUI sample row exceeds the {byte_limit} byte estimate limit").into(),
                    ));
                }
                return Ok(true);
            }
            let take = batch.height().min(SAMPLE_ROWS - sample.rows).min(row_budget);
            let rows = (0..take).map(|row| row as IdxSize).collect();
            // `take` preserves schema and values, but Arrow may share backing
            // buffers with the streamed batch. This is a visible-size estimate,
            // not a hard process-RAM ceiling.
            let retained = batch.take(&IdxCa::from_vec(PlSmallStr::EMPTY, rows)).map_err(|error| {
                polars::error::PolarsError::ComputeError(format!("failed to retain TUI sample rows: {error}").into())
            })?;
            let retained_bytes = retained.estimated_size();
            if retained_bytes > byte_limit - sample.bytes {
                if sample.rows == 0 {
                    return Err(polars::error::PolarsError::ComputeError(
                        format!("first TUI sample batch exceeds the {byte_limit} byte estimate limit").into(),
                    ));
                }
                return Ok(true);
            }
            sample.rows += take;
            sample.bytes += retained_bytes;
            sample.batches.push(retained);
            Ok(sample.rows >= SAMPLE_ROWS || sample.bytes >= byte_limit)
        }),
        true,
        NonZeroUsize::new(SAMPLE_BATCH_ROWS),
    ).map_err(|error| QuiltError::operation("load TUI sample", error.to_string()))?;
    sink.collect()
        .map_err(|error| QuiltError::operation("load TUI sample", error.to_string()))?;
    let mut sample = sample
        .lock()
        .map_err(|_| QuiltError::operation("load TUI sample", "sample lock poisoned"))?;
    let rows = sample.rows;
    let mut batches = std::mem::take(&mut sample.batches).into_iter();
    let Some(mut output) = batches.next() else {
        let empty = frame
            .limit(0)
            .collect()
            .map_err(|error| QuiltError::operation("load TUI sample", error.to_string()))?;
        return Ok((empty, 0));
    };
    for batch in batches {
        output
            .vstack_mut(&batch)
            .map_err(|error| QuiltError::operation("load TUI sample", error.to_string()))?;
    }
    Ok((output, rows))
}

fn preview_frame(frame: &LazyFrame) -> Result<FramePreview, QuiltError> {
    let schema = frame
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::operation("preview schema", e.to_string()))?;
    let columns: Vec<(String, String)> = schema
        .iter()
        .take(MAX_COLUMNS)
        .map(|(name, dtype)| (bound_cell(name.to_string()), bound_cell(dtype.to_string())))
        .collect();
    let display_exprs: Vec<Expr> = schema
        .iter()
        .take(MAX_COLUMNS)
        .map(|(name, dtype)| {
            let column = crate::controllers::nested::literal_col(name.as_str());
            if matches!(dtype, DataType::Struct(_) | DataType::List(_)) {
                crate::controllers::nested::json_expr(column).alias(name.clone())
            } else {
                column
            }
        })
        .collect();
    let df = frame
        .clone()
        .select(display_exprs)
        .limit((PREVIEW_ROWS + 1) as IdxSize)
        .collect()
        .map_err(|e| QuiltError::operation("TUI preview", e.to_string()))?;
    let truncated = df.height() > PREVIEW_ROWS || schema.len() > MAX_COLUMNS;
    let rows = (0..df.height().min(PREVIEW_ROWS))
        .map(|row| {
            df.columns()
                .iter()
                .take(MAX_COLUMNS)
                .map(|column| {
                    bound_cell(column.get(row).map(|v| v.to_string()).unwrap_or_default())
                })
                .collect()
        })
        .collect();
    Ok(FramePreview {
        columns,
        rows,
        truncated,
    })
}

fn bound_cell(value: String) -> String {
    let escaped: String = value
        .chars()
        .map(|ch| {
            if ch.is_control()
                || matches!(ch as u32, 0x2028..=0x202e | 0x2066..=0x2069 | 0x200e | 0x200f)
            {
                ch.escape_default().to_string()
            } else {
                ch.to_string()
            }
        })
        .collect();
    let mut chars = escaped.chars();
    let bounded: String = chars.by_ref().take(MAX_CELL_CHARS).collect();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

fn write_new(path: &std::path::Path, contents: &[u8]) -> Result<(), QuiltError> {
    use std::io::Write;
    crate::operations::finalizers::atomic_path(path, "write TUI export", |temporary| {
        let mut file = std::fs::File::create(temporary)
            .map_err(|e| QuiltError::operation("write TUI export", e.to_string()))?;
        file.write_all(contents)
            .map_err(|e| QuiltError::operation("write TUI export", e.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::command_model::{CountArgs, DumpCacheArgs};

    fn load(path: &std::path::Path) -> TypedCommand {
        TypedCommand::Load(LoadArgs {
            paths: vec![path.to_owned()],
            separator: ',',
            low_memory: false,
            no_headers: false,
            chunk_size: None,
            infer_schema_length: None,
            source_column: None,
            all_strings: false,
            schema: Vec::new(),
            stdin_format: None,
            rejects: None,
        })
    }

    fn preview_request(id: u64) -> Request {
        Request {
            id,
            commands: Vec::new(),
            action: Action::Preview(PreviewMode::Sample),
        }
    }

    #[test]
    fn queued_preview_replaces_stale_snapshot_without_dropping_explicit_jobs() {
        let mut schedule = Schedule {
            actions: VecDeque::new(),
            preview: None,
            closed: false,
        };
        enqueue(&mut schedule, preview_request(1)).unwrap();
        enqueue(
            &mut schedule,
            Request {
                id: 2,
                commands: Vec::new(),
                action: Action::Cache,
            },
        )
        .unwrap();
        enqueue(&mut schedule, preview_request(3)).unwrap();
        assert_eq!(schedule.preview.as_ref().unwrap().id, 3);
        assert_eq!(schedule.actions.front().unwrap().id, 2);
    }

    fn commands(path: &std::path::Path) -> Vec<TypedCommand> {
        vec![
            load(path),
            TypedCommand::Count(CountArgs {
                columns: vec!["group".into()],
            }),
        ]
    }

    #[test]
    fn sample_cap_precedes_count_while_full_counts_all_groups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.csv");
        let mut csv = String::from("group\n");
        for _ in 0..SAMPLE_ROWS {
            csv.push_str("A\n");
        }
        csv.push_str("B\n");
        std::fs::write(&path, csv).unwrap();
        let cmds = commands(&path);
        let mut engine = Engine::new();
        assert_eq!(
            engine
                .sample_pipeline(&cmds)
                .unwrap()
                .frame()
                .clone()
                .collect()
                .unwrap()
                .height(),
            1
        );
        assert_eq!(
            engine
                .full_pipeline(&cmds)
                .unwrap()
                .frame()
                .clone()
                .collect()
                .unwrap()
                .height(),
            2
        );
    }

    #[test]
    fn oversized_sample_row_does_not_block_full_preview_or_explicit_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large-cell.csv");
        let value = "x".repeat(128);
        std::fs::write(&path, format!("value\n{value}\n")).unwrap();
        let cmds = vec![load(&path)];
        let mut engine = Engine::with_sample_budget(8);
        assert!(engine.sample_pipeline(&cmds).is_err());
        assert!(matches!(
            engine
                .process(Request {
                    id: 1,
                    commands: cmds.clone(),
                    action: Action::Preview(PreviewMode::Full)
                })
                .unwrap(),
            Outcome::Preview(_)
        ));
        assert!(matches!(
            engine
                .process(Request {
                    id: 2,
                    commands: cmds.clone(),
                    action: Action::Cache
                })
                .unwrap(),
            Outcome::Cached(_)
        ));
        let requested = dir.path().join("persisted");
        let saved = engine
            .process(Request {
                id: 3,
                commands: cmds,
                action: Action::SaveCache(requested.clone()),
            })
            .unwrap();
        let actual = requested.with_extension("parquet");
        assert!(matches!(saved, Outcome::Saved(path) if path == actual));
        assert!(actual.exists());
    }

    #[test]
    fn explicit_cache_materializes_and_can_restore_full_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.csv");
        std::fs::write(&path, "group\nA\nB\n").unwrap();
        let cmds = commands(&path);
        let mut engine = Engine::new();
        let result = engine
            .process(Request {
                id: 1,
                commands: cmds.clone(),
                action: Action::Cache,
            })
            .unwrap();
        let Outcome::Cached(info) = result else {
            panic!("cache action did not report its checkpoint")
        };
        assert!(info.bytes > 0);
        assert!(engine.cache.restore(&cmds).is_some());
        let edited = vec![load(&path)];
        assert!(engine.sample_pipeline(&edited).is_ok());
        assert!(engine.cache.info().is_none());
    }

    #[test]
    fn previews_reject_finalizers_and_yaml_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.csv");
        let target = dir.path().join("should-not-exist.parquet");
        std::fs::write(&path, "value\n1\n").unwrap();
        let bad = vec![
            load(&path),
            TypedCommand::DumpCache(DumpCacheArgs {
                output: Some(target.to_string_lossy().into_owned()),
            }),
        ];
        let mut engine = Engine::new();
        assert!(engine.sample_pipeline(&bad).is_err());
        assert!(!target.exists());
        std::fs::write(&target, "keep").unwrap();
        assert!(write_new(&target, b"replace").is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn changed_source_metadata_reloads_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.csv");
        std::fs::write(&path, "group\nA\n").unwrap();
        let cmds = commands(&path);
        let mut engine = Engine::new();
        assert_eq!(
            engine
                .full_pipeline(&cmds)
                .unwrap()
                .frame()
                .clone()
                .collect()
                .unwrap()
                .height(),
            1
        );
        assert!(matches!(
            engine
                .process(Request {
                    id: 1,
                    commands: cmds.clone(),
                    action: Action::Cache
                })
                .unwrap(),
            Outcome::Cached(_)
        ));
        std::fs::write(&path, "group\nA\nB\n").unwrap();
        assert_eq!(
            engine
                .full_pipeline(&cmds)
                .unwrap()
                .frame()
                .clone()
                .collect()
                .unwrap()
                .height(),
            2
        );
        assert!(engine.cache.info().is_none());
    }
}
