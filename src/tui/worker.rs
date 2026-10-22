use super::task::TaskControl;
use super::{
    Action, Event, Outcome, Preview, PreviewMode, Request, TaskPhase, DEFAULT_CACHE_LIMIT,
    PREVIEW_ROWS, SAMPLE_ROWS,
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
const MAX_DETAIL_BYTES: usize = 256 * 1024;

struct Schedule {
    actions: VecDeque<Request>,
    preview: Option<Request>,
    active: Option<(u64, TaskControl)>,
    closed: bool,
}

pub(super) struct Worker {
    schedule: Arc<(Mutex<Schedule>, Condvar)>,
    events: mpsc::Receiver<Event>,
    event_tx: mpsc::Sender<Event>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub(super) fn new() -> Self {
        let schedule = Arc::new((
            Mutex::new(Schedule {
                actions: VecDeque::new(),
                preview: None,
                active: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let worker_schedule = Arc::clone(&schedule);
        let (event_tx, event_rx) = mpsc::channel();
        let worker_events = event_tx.clone();
        let thread = thread::Builder::new()
            .name("quilt-tui-worker".into())
            .spawn(move || worker_loop(worker_schedule, worker_events))
            .expect("failed to start TUI worker");
        Self {
            schedule,
            events: event_rx,
            event_tx,
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

    /// Cancels the running request when possible and discards queued requests.
    /// Running Polars work may finish before its cancellation is observed.
    pub(super) fn cancel(&self) -> bool {
        let (lock, ready) = &*self.schedule;
        let Ok(mut schedule) = lock.lock() else {
            return false;
        };
        let mut cancelled = false;
        if let Some((_, task)) = &schedule.active {
            task.cancel();
            cancelled = true;
        }
        let mut discarded = schedule.actions.drain(..).collect::<Vec<_>>();
        if let Some(preview) = schedule.preview.take() {
            discarded.push(preview);
        }
        for request in discarded {
            let _ = self.event_tx.send(Event {
                id: request.id,
                result: Ok(Outcome::Cancelled),
            });
            cancelled = true;
        }
        ready.notify_one();
        cancelled
    }
}

fn enqueue(schedule: &mut Schedule, request: Request) -> Result<(), QuiltError> {
    if schedule.closed {
        return Err(QuiltError::operation("TUI worker", "worker has stopped"));
    }
    if matches!(
        &request.action,
        Action::Preview(_) | Action::DraftPreview { .. }
    ) {
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
            if let Some((_, task)) = &schedule.active {
                task.cancel();
            }
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
    total_columns: usize,
    input_columns: Vec<(String, String)>,
    rows: Vec<Vec<String>>,
    has_more_rows: bool,
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

    #[cfg(test)]
    fn process(&mut self, request: Request) -> Result<Outcome, QuiltError> {
        self.process_with_task(request, &TaskControl::new(), |_| {})
    }

    fn process_with_task(
        &mut self,
        request: Request,
        task: &TaskControl,
        mut progress: impl FnMut(TaskPhase),
    ) -> Result<Outcome, QuiltError> {
        task.check()?;
        match request.action {
            Action::Cache => {
                progress(TaskPhase::Loading);
                let pipeline = self.full_pipeline(&request.commands, task)?;
                task.check()?;
                progress(TaskPhase::Caching);
                let info = self.cache.store(request.commands, pipeline)?;
                task.check()?;
                Ok(Outcome::Cached(info))
            }
            Action::ClearCache => {
                task.check()?;
                self.cache.clear();
                Ok(Outcome::CacheCleared)
            }
            Action::ExportYaml(path) => {
                progress(TaskPhase::Saving);
                let yaml = crate::controllers::yaml_export::workflow_yaml(&request.commands)?;
                task.check()?;
                write_new(&path, yaml.as_bytes())?;
                Ok(Outcome::Saved(path))
            }
            Action::SaveCache(path) => {
                progress(TaskPhase::Loading);
                let pipeline = self.full_pipeline(&request.commands, task)?;
                task.check()?;
                progress(TaskPhase::Saving);
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
            Action::ExportData { path, format } => {
                progress(TaskPhase::Loading);
                let pipeline = self.full_pipeline(&request.commands, task)?;
                progress(TaskPhase::Saving);
                super::output::export(&pipeline, &path, format, task)?;
                Ok(Outcome::Saved(path))
            }
            Action::Preview(mode) => {
                progress(TaskPhase::Loading);
                let pipeline = self.pipeline_for(&request.commands, mode, task)?;
                progress(TaskPhase::Computing);
                let preview = preview_frame(pipeline.frame(), task)?;
                let source_version = self.loaded_source_version(&request.commands)?;
                self.verify_source_version(&request.commands, &source_version)?;
                Ok(Outcome::Preview(Preview {
                    columns: preview.columns,
                    total_columns: preview.total_columns,
                    input_columns: preview.input_columns,
                    rows: preview.rows,
                    has_more_rows: preview.has_more_rows,
                    mode,
                    cache: self.cache.info(),
                    sample_rows: self.sample_rows(mode),
                    draft_issue: None,
                    commands: request.commands,
                    source_version,
                }))
            }
            Action::DraftPreview {
                mode,
                context,
                issue,
            } => {
                progress(TaskPhase::Loading);
                let preview = self.draft_preview(
                    &request.commands,
                    &context,
                    issue,
                    mode,
                    task,
                    &mut progress,
                )?;
                Ok(Outcome::Preview(preview))
            }
            Action::Inspect {
                mode,
                row,
                column,
                source_version,
            } => {
                progress(TaskPhase::Inspecting);
                self.inspect(&request.commands, mode, row, column, &source_version, task)
            }
        }
    }

    fn source_version(&self, commands: &[TypedCommand]) -> Result<String, QuiltError> {
        let args = Self::validate(commands)?;
        source_fingerprint(args)
    }

    fn loaded_source_version(&self, commands: &[TypedCommand]) -> Result<String, QuiltError> {
        let args = Self::validate(commands)?;
        self.source
            .as_ref()
            .filter(|source| source.args == *args)
            .map(|source| source.fingerprint.clone())
            .ok_or_else(|| QuiltError::usage("preview source is no longer available"))
    }

    fn verify_source_version(
        &self,
        commands: &[TypedCommand],
        expected: &str,
    ) -> Result<(), QuiltError> {
        if self.source_version(commands)? == expected {
            Ok(())
        } else {
            Err(QuiltError::usage(
                "input changed while reading preview data; refresh before continuing",
            ))
        }
    }

    fn inspect(
        &mut self,
        commands: &[TypedCommand],
        mode: PreviewMode,
        row: usize,
        column: Option<usize>,
        expected_source: &str,
        task: &TaskControl,
    ) -> Result<Outcome, QuiltError> {
        if row >= PREVIEW_ROWS {
            return Err(QuiltError::usage("row is outside the visible preview"));
        }
        let before = self.source_version(commands)?;
        if before != expected_source {
            return Err(QuiltError::usage(
                "input changed since the preview was rendered; refresh before inspecting details",
            ));
        }
        let pipeline = self.pipeline_for(commands, mode, task)?;
        let schema = pipeline
            .frame()
            .clone()
            .collect_schema()
            .map_err(|error| QuiltError::operation("inspect schema", error.to_string()))?;
        if let Some(index) = column {
            let Some((name, dtype)) = schema.get_at_index(index) else {
                return Err(QuiltError::usage("column is outside the preview schema"));
            };
            let expression = detail_expression(name, dtype);
            let frame = pipeline
                .frame()
                .clone()
                .select([expression])
                .slice(row as i64, 1);
            let result = task.collect(frame)?;
            if result.height() == 0 {
                return Err(QuiltError::usage(
                    "row is no longer available in this preview",
                ));
            }
            let (text, truncated) = result
                .columns()
                .first()
                .map(|column| {
                    let series = column.as_materialized_series();
                    if let Ok(values) = series.str() {
                        values
                            .get(0)
                            .map(bound_detail_ref)
                            .unwrap_or_else(|| ("null".to_owned(), false))
                    } else {
                        let value = series
                            .get(0)
                            .map(|value| value.to_string())
                            .unwrap_or_else(|_| "null".to_owned());
                        bound_detail(value)
                    }
                })
                .unwrap_or_else(|| ("null".to_owned(), false));
            task.check()?;
            self.verify_source_version(commands, expected_source)?;
            return Ok(Outcome::Detail {
                title: format!("Row {}, {name}", row + 1),
                text,
                truncated,
            });
        }

        let total_columns = schema.len();
        let expressions = schema
            .iter()
            .take(MAX_COLUMNS)
            .map(|(name, dtype)| detail_expression(name.as_str(), dtype))
            .collect::<Vec<_>>();
        let frame = pipeline
            .frame()
            .clone()
            .select(expressions)
            .slice(row as i64, 1);
        let result = task.collect(frame)?;
        if result.height() == 0 {
            return Err(QuiltError::usage(
                "row is no longer available in this preview",
            ));
        }
        let mut text = String::new();
        let mut content_truncated = false;
        for (index, column) in result.columns().iter().enumerate() {
            let Some((name, _)) = schema.get_at_index(index) else {
                break;
            };
            if !append_series_detail(&mut text, name.as_str(), column.as_materialized_series()) {
                content_truncated = true;
                break;
            }
        }
        let omitted_columns = total_columns > MAX_COLUMNS;
        if !content_truncated && omitted_columns {
            let note = format!("… {} more columns omitted\n", total_columns - MAX_COLUMNS);
            content_truncated = !append_detail(&mut text, &note);
        }
        if content_truncated {
            append_detail_suffix(&mut text);
        }
        let truncated = content_truncated || omitted_columns;
        task.check()?;
        self.verify_source_version(commands, expected_source)?;
        Ok(Outcome::Detail {
            title: format!("Row {}", row + 1),
            text,
            truncated,
        })
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
        task: &TaskControl,
    ) -> Result<Pipeline, QuiltError> {
        task.check()?;
        let fingerprint = source_fingerprint(args)?;
        let current = self
            .source
            .as_ref()
            .is_some_and(|source| source.args == *args && source.fingerprint == fingerprint);
        if !current {
            task.check()?;
            self.cache.clear();
            let pipeline = Pipeline::load_args(args, ExecutionResources::new())?;
            task.check()?;
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
                collect_bounded_sample(pipeline.frame().clone(), self.sample_budget, task)?;
            let sample_pipeline = Pipeline::new(sample.lazy(), pipeline.resources())
                .with_source_inventory(pipeline.source_inventory());
            self.source.as_mut().expect("source just loaded").sample = Some(LoadedSample {
                pipeline: sample_pipeline,
                rows: sample_rows,
            });
        }
        Ok(pipeline)
    }

    fn sample_pipeline(
        &mut self,
        commands: &[TypedCommand],
        task: &TaskControl,
    ) -> Result<Pipeline, QuiltError> {
        let args = Self::validate(commands)?;
        let _ = self.ensure_source(args, true, task)?;
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
            task.check()?;
            executor.execute(command)?;
        }
        task.check()?;
        executor
            .into_pipeline()
            .ok_or_else(|| QuiltError::usage("load data before previewing"))
    }

    fn full_pipeline(
        &mut self,
        commands: &[TypedCommand],
        task: &TaskControl,
    ) -> Result<Pipeline, QuiltError> {
        let args = Self::validate(commands)?;
        let _ = self.ensure_source(args, false, task)?;
        if let Some((prefix, pipeline)) = self.cache.restore(commands) {
            let mut executor = CommandExecutor::from_pipeline(pipeline);
            for command in &commands[prefix..] {
                task.check()?;
                executor.execute(command)?;
            }
            task.check()?;
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
            task.check()?;
            executor.execute(command)?;
        }
        task.check()?;
        executor
            .into_pipeline()
            .ok_or_else(|| QuiltError::usage("load data before previewing"))
    }

    fn pipeline_for(
        &mut self,
        commands: &[TypedCommand],
        mode: PreviewMode,
        task: &TaskControl,
    ) -> Result<Pipeline, QuiltError> {
        match mode {
            PreviewMode::Sample => self.sample_pipeline(commands, task),
            PreviewMode::Full => self.full_pipeline(commands, task),
        }
    }

    fn sample_rows(&self, mode: PreviewMode) -> Option<usize> {
        (mode == PreviewMode::Sample)
            .then(|| {
                self.source
                    .as_ref()
                    .and_then(|source| source.sample.as_ref().map(|sample| sample.rows))
            })
            .flatten()
    }

    fn draft_preview(
        &mut self,
        candidate: &[TypedCommand],
        context: &[TypedCommand],
        issue: Option<String>,
        mode: PreviewMode,
        task: &TaskControl,
        progress: &mut impl FnMut(TaskPhase),
    ) -> Result<Preview, QuiltError> {
        let context_pipeline = if context.is_empty() {
            None
        } else {
            Some(self.pipeline_for(context, mode, task)?)
        };

        let (pipeline, draft_issue, input_columns) = if let Some(issue) = issue {
            let pipeline = context_pipeline
                .ok_or_else(|| QuiltError::usage("no valid pipeline context for draft"))?;
            (pipeline, Some(bound_cell(issue)), None)
        } else {
            let candidate_result = self
                .pipeline_for(candidate, mode, task)
                .and_then(|pipeline| {
                    progress(TaskPhase::Computing);
                    preview_frame(pipeline.frame(), task).map(|preview| (pipeline, preview))
                });
            match candidate_result {
                Ok((pipeline, candidate_preview)) => {
                    let context_schema = if let Some(context_pipeline) = context_pipeline.as_ref() {
                        input_columns(context_pipeline.frame())?
                    } else {
                        candidate_preview.input_columns.clone()
                    };
                    (pipeline, None, Some((candidate_preview, context_schema)))
                }
                Err(candidate_error) => {
                    let Some(pipeline) = context_pipeline else {
                        return Err(candidate_error);
                    };
                    (
                        pipeline,
                        Some(bound_detail_ref(&candidate_error.to_string()).0),
                        None,
                    )
                }
            }
        };

        let (rendered_commands, rendered) = if let Some((preview, input_columns)) = input_columns {
            (candidate.to_vec(), (preview, input_columns))
        } else {
            progress(TaskPhase::Computing);
            let preview = preview_frame(pipeline.frame(), task)?;
            let columns = preview.input_columns.clone();
            (context.to_vec(), (preview, columns))
        };
        let (preview, input_columns) = rendered;
        let source_version = self.loaded_source_version(&rendered_commands)?;
        self.verify_source_version(&rendered_commands, &source_version)?;
        Ok(Preview {
            columns: preview.columns,
            total_columns: preview.total_columns,
            input_columns,
            rows: preview.rows,
            has_more_rows: preview.has_more_rows,
            mode,
            sample_rows: self.sample_rows(mode),
            draft_issue,
            cache: self.cache.info(),
            source_version,
            commands: rendered_commands,
        })
    }
}

fn worker_loop(schedule: Arc<(Mutex<Schedule>, Condvar)>, events: mpsc::Sender<Event>) {
    let mut engine = Engine::new();
    loop {
        let claimed = {
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
            let request = if preview_id.is_some() && (action_id.is_none() || preview_id < action_id)
            {
                state.preview.take()
            } else {
                state.actions.pop_front()
            };
            request.map(|request| {
                let task = TaskControl::new();
                state.active = Some((request.id, task.clone()));
                (request, task)
            })
        };
        let Some((request, task)) = claimed else {
            continue;
        };
        let id = request.id;
        let result = catch_unwind(AssertUnwindSafe(|| {
            engine.process_with_task(request, &task, |phase| {
                let _ = events.send(Event {
                    id,
                    result: Ok(Outcome::Working(phase)),
                });
            })
        }))
        .unwrap_or_else(|_| {
            Err(QuiltError::operation(
                "TUI worker",
                "background operation panicked",
            ))
        });
        let (lock, _) = &*schedule;
        if let Ok(mut state) = lock.lock() {
            if state
                .active
                .as_ref()
                .is_some_and(|(active_id, _)| *active_id == id)
            {
                state.active = None;
            }
        }
        let result = if task.is_cancelled() {
            match result {
                Ok(Outcome::Saved(path)) => Ok(Outcome::Saved(path)),
                _ => Ok(Outcome::Cancelled),
            }
        } else {
            result.map_err(bound_error)
        };
        if events.send(Event { id, result }).is_err() {
            return;
        }
    }
}

fn bound_error(error: QuiltError) -> QuiltError {
    let (message, _) = bound_detail_ref(&error.to_string());
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
    task: &TaskControl,
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
    let callback_task = task.clone();
    let sink = frame.clone().sink_batches(
        PlanCallback::new(move |batch: DataFrame| {
            if callback_task.is_cancelled() {
                return Ok(true);
            }
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
    task.check()?;
    sink.collect()
        .map_err(|error| QuiltError::operation("load TUI sample", error.to_string()))?;
    task.check()?;
    let mut sample = sample
        .lock()
        .map_err(|_| QuiltError::operation("load TUI sample", "sample lock poisoned"))?;
    let rows = sample.rows;
    let mut batches = std::mem::take(&mut sample.batches).into_iter();
    let Some(mut output) = batches.next() else {
        let empty = task.collect(frame.limit(0))?;
        return Ok((empty, 0));
    };
    for batch in batches {
        output
            .vstack_mut(&batch)
            .map_err(|error| QuiltError::operation("load TUI sample", error.to_string()))?;
    }
    Ok((output, rows))
}

fn preview_frame(frame: &LazyFrame, task: &TaskControl) -> Result<FramePreview, QuiltError> {
    let schema = frame
        .clone()
        .collect_schema()
        .map_err(|e| QuiltError::operation("preview schema", e.to_string()))?;
    let total_columns = schema.len();
    let input_columns = schema
        .iter()
        .map(|(name, dtype)| (name.to_string(), dtype.to_string()))
        .collect::<Vec<_>>();
    let columns: Vec<(String, String)> = input_columns
        .iter()
        .take(MAX_COLUMNS)
        .map(|(name, dtype)| (bound_cell(name.clone()), bound_cell(dtype.clone())))
        .collect();
    let display_exprs: Vec<Expr> = schema
        .iter()
        .take(MAX_COLUMNS)
        .map(|(name, dtype)| {
            let column = crate::controllers::nested::literal_col(name.as_str());
            if matches!(dtype, DataType::Struct(_) | DataType::List(_)) {
                crate::controllers::nested::json_expr(column, dtype).alias(name.clone())
            } else {
                column
            }
        })
        .collect();
    let df = task.collect(
        frame
            .clone()
            .select(display_exprs)
            .limit((PREVIEW_ROWS + 1) as IdxSize),
    )?;
    let has_more_rows = df.height() > PREVIEW_ROWS;
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
        total_columns,
        input_columns,
        rows,
        has_more_rows,
    })
}

fn input_columns(frame: &LazyFrame) -> Result<Vec<(String, String)>, QuiltError> {
    frame
        .clone()
        .collect_schema()
        .map(|schema| {
            schema
                .iter()
                .map(|(name, dtype)| (name.to_string(), dtype.to_string()))
                .collect()
        })
        .map_err(|error| QuiltError::operation("preview schema", error.to_string()))
}

fn detail_expression(name: &str, dtype: &DataType) -> Expr {
    let column = crate::controllers::nested::literal_col(name);
    if matches!(dtype, DataType::Struct(_) | DataType::List(_)) {
        crate::controllers::nested::json_expr(column, dtype).alias(name)
    } else {
        column
    }
}

fn append_series_detail(out: &mut String, name: &str, series: &Series) -> bool {
    if !append_detail(out, name) || !append_detail(out, ": ") {
        return false;
    }
    if let Ok(values) = series.str() {
        if let Some(value) = values.get(0) {
            if !append_detail(out, value) {
                return false;
            }
        } else if !append_detail(out, "null") {
            return false;
        }
    } else {
        let value = series
            .get(0)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "null".to_owned());
        if !append_detail(out, &value) {
            return false;
        }
    }
    append_detail(out, "\n")
}

fn append_detail(out: &mut String, value: &str) -> bool {
    let available = MAX_DETAIL_BYTES.saturating_sub(out.len());
    if value.len() <= available {
        out.push_str(value);
        return true;
    }
    let mut end = available;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&value[..end]);
    false
}

fn append_detail_suffix(out: &mut String) {
    const SUFFIX: &str = "\n[Detail text truncated at 256 KiB]";
    let target = MAX_DETAIL_BYTES.saturating_sub(SUFFIX.len());
    while !out.is_char_boundary(target.min(out.len())) {
        out.pop();
    }
    out.truncate(target.min(out.len()));
    out.push_str(SUFFIX);
}

fn bound_detail(value: String) -> (String, bool) {
    bound_detail_ref(&value)
}

fn bound_detail_ref(value: &str) -> (String, bool) {
    let mut bounded = String::with_capacity(value.len().min(MAX_DETAIL_BYTES));
    let complete = append_detail(&mut bounded, value);
    if complete {
        (bounded, false)
    } else {
        append_detail_suffix(&mut bounded);
        (bounded, true)
    }
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
    use crate::controllers::command_model::{CountArgs, DumpCacheArgs, SelectArgs};

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
            active: None,
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
        enqueue(
            &mut schedule,
            Request {
                id: 4,
                commands: Vec::new(),
                action: Action::DraftPreview {
                    mode: PreviewMode::Sample,
                    context: Vec::new(),
                    issue: Some("unfinished".into()),
                },
            },
        )
        .unwrap();
        assert_eq!(schedule.preview.as_ref().unwrap().id, 4);
        assert_eq!(schedule.actions.front().unwrap().id, 2);
    }

    #[test]
    fn wide_preview_reports_total_schema_width_with_bounded_display() {
        let columns = (0..MAX_COLUMNS + 1)
            .map(|index| {
                let name = format!("column_{index}");
                if index == 0 {
                    Series::new(name.into(), vec!["x".repeat(MAX_CELL_CHARS + 100)]).into()
                } else {
                    Series::new(name.into(), vec![index as i64]).into()
                }
            })
            .collect();
        let frame = DataFrame::new_infer_height(columns).unwrap().lazy();
        let preview = preview_frame(&frame, &TaskControl::new()).unwrap();
        assert_eq!(preview.total_columns, MAX_COLUMNS + 1);
        assert_eq!(preview.columns.len(), MAX_COLUMNS);
        assert_eq!(preview.rows.len(), 1);
        assert_eq!(preview.rows[0].len(), MAX_COLUMNS);
        assert!(!preview.has_more_rows);
        assert_eq!(preview.rows[0][0].chars().count(), MAX_CELL_CHARS + 1);
        assert!(preview.rows[0][0].ends_with('…'));
    }

    #[test]
    fn preview_marks_more_rows_independently_of_omitted_columns() {
        let frame = df!("a" => 0..PREVIEW_ROWS as i64 + 1).unwrap().lazy();
        let preview = preview_frame(&frame, &TaskControl::new()).unwrap();
        assert_eq!(preview.rows.len(), PREVIEW_ROWS);
        assert!(preview.has_more_rows);

        let wide = (0..MAX_COLUMNS + 3)
            .map(|index| Series::new(format!("c{index}").into(), &[index as i64]).into())
            .collect();
        let wide = DataFrame::new_infer_height(wide).unwrap().lazy();
        let preview = preview_frame(&wide, &TaskControl::new()).unwrap();
        assert_eq!(preview.columns.len(), MAX_COLUMNS);
        assert!(!preview.has_more_rows);
    }

    #[test]
    fn inspect_reads_full_cell_and_row_details_with_a_fixed_byte_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("details.csv");
        let value = "猫".repeat(MAX_DETAIL_BYTES / 2);
        std::fs::write(&path, format!("text,n\n{value},7\n")).unwrap();
        let commands = vec![load(&path)];
        let mut engine = Engine::new();
        let source_version = engine.source_version(&commands).unwrap();
        let cell = engine
            .process(Request {
                id: 1,
                commands: commands.clone(),
                action: Action::Inspect {
                    mode: PreviewMode::Full,
                    row: 0,
                    column: Some(0),
                    source_version: source_version.clone(),
                },
            })
            .unwrap();
        let Outcome::Detail {
            text,
            truncated: true,
            ..
        } = cell
        else {
            panic!("long cell was not returned as a bounded detail")
        };
        assert!(text.len() <= MAX_DETAIL_BYTES);
        assert!(text.ends_with("[Detail text truncated at 256 KiB]"));
        assert!(text.is_char_boundary(text.len()));

        let row = engine
            .process(Request {
                id: 2,
                commands: commands.clone(),
                action: Action::Inspect {
                    mode: PreviewMode::Full,
                    row: 0,
                    column: None,
                    source_version,
                },
            })
            .unwrap();
        let Outcome::Detail {
            text,
            truncated: true,
            ..
        } = row
        else {
            panic!("long row was not returned as a bounded detail")
        };
        assert!(text.len() <= MAX_DETAIL_BYTES);
        assert!(text.starts_with("text: "));
        assert!(text.ends_with("[Detail text truncated at 256 KiB]"));
    }

    #[test]
    fn full_export_keeps_rows_beyond_preview_and_progress_reports_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("many.csv");
        let mut input = String::from("n\n");
        for value in 0..(SAMPLE_ROWS + 5) {
            input.push_str(&format!("{value}\n"));
        }
        std::fs::write(&path, input).unwrap();
        let commands = vec![load(&path)];
        let output = dir.path().join("full.jsonl");
        let mut engine = Engine::new();
        let sampled = engine
            .process(Request {
                id: 1,
                commands: commands.clone(),
                action: Action::Preview(PreviewMode::Sample),
            })
            .unwrap();
        let Outcome::Preview(sampled) = sampled else {
            panic!("sample request did not return a preview")
        };
        assert_eq!(sampled.rows.len(), PREVIEW_ROWS);
        assert_eq!(sampled.sample_rows, Some(SAMPLE_ROWS));

        let mut phases = Vec::new();
        let saved = engine
            .process_with_task(
                Request {
                    id: 2,
                    commands,
                    action: Action::ExportData {
                        path: output.clone(),
                        format: crate::controllers::arguments::OutputFormat::Jsonl,
                    },
                },
                &TaskControl::new(),
                |phase| phases.push(phase),
            )
            .unwrap();
        assert!(matches!(saved, Outcome::Saved(path) if path == output));
        assert_eq!(
            std::fs::read_to_string(&output).unwrap().lines().count(),
            SAMPLE_ROWS + 5
        );
        assert_eq!(phases, vec![TaskPhase::Loading, TaskPhase::Saving]);
    }

    #[test]
    fn detail_inspection_rejects_a_stale_source_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.csv");
        std::fs::write(&path, "value\n1\n").unwrap();
        let commands = vec![load(&path)];
        let mut engine = Engine::new();
        let stale = engine.source_version(&commands).unwrap();
        std::fs::write(&path, "value\n100\n").unwrap();
        let result = engine.process(Request {
            id: 1,
            commands,
            action: Action::Inspect {
                mode: PreviewMode::Full,
                row: 0,
                column: Some(0),
                source_version: stale,
            },
        });
        assert!(result.is_err());
    }

    #[test]
    fn worker_errors_keep_long_causes_available_for_the_detail_view() {
        let cause = "cause ".repeat(200);
        let result = bound_error(QuiltError::operation("preview", cause.clone())).to_string();
        assert!(result.contains(&cause));
        assert!(result.len() > 512);
        assert!(result.len() <= MAX_DETAIL_BYTES + 64);
    }

    #[test]
    fn draft_preview_keeps_full_context_schema_and_falls_back_on_invalid_command() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("draft.csv");
        std::fs::write(&path, "hoge,other\n1,2\n").unwrap();
        let context = vec![load(&path)];
        let candidate = vec![
            load(&path),
            TypedCommand::Select(SelectArgs {
                columns: vec!["hoge".into()],
            }),
        ];
        let mut engine = Engine::new();
        let Outcome::Preview(selected) = engine
            .process(Request {
                id: 1,
                commands: candidate,
                action: Action::DraftPreview {
                    mode: PreviewMode::Sample,
                    context: context.clone(),
                    issue: None,
                },
            })
            .unwrap()
        else {
            panic!("draft did not return a preview")
        };
        assert_eq!(selected.columns.len(), 1);
        assert_eq!(
            selected
                .input_columns
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["hoge", "other"]
        );
        assert!(selected.draft_issue.is_none());

        let Outcome::Preview(fallback) = engine
            .process(Request {
                id: 2,
                commands: context.clone(),
                action: Action::DraftPreview {
                    mode: PreviewMode::Sample,
                    context: context.clone(),
                    issue: Some("incomplete select".into()),
                },
            })
            .unwrap()
        else {
            panic!("draft fallback did not return a preview")
        };
        assert_eq!(fallback.columns.len(), 2);
        assert_eq!(fallback.input_columns.len(), 2);
        assert_eq!(fallback.draft_issue.as_deref(), Some("incomplete select"));

        let invalid = vec![
            load(&path),
            TypedCommand::Select(SelectArgs {
                columns: vec!["missing".into()],
            }),
        ];
        assert!(engine
            .process(Request {
                id: 3,
                commands: invalid,
                action: Action::Preview(PreviewMode::Sample)
            })
            .is_err());
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
                .sample_pipeline(&cmds, &TaskControl::new())
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
                .full_pipeline(&cmds, &TaskControl::new())
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
        assert!(engine.sample_pipeline(&cmds, &TaskControl::new()).is_err());
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
        assert!(engine.sample_pipeline(&edited, &TaskControl::new()).is_ok());
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
        assert!(engine.sample_pipeline(&bad, &TaskControl::new()).is_err());
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
                .full_pipeline(&cmds, &TaskControl::new())
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
                .full_pipeline(&cmds, &TaskControl::new())
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
