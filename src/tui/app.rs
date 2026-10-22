use super::{
    columns::{ColumnPicker, PickerResult},
    detail::DetailView,
    input,
    worker::Worker,
    Action, Event, Outcome, Preview, PreviewMode, Request, TaskPhase,
};
use crate::controllers::{
    arguments::OutputFormat,
    command_model::{registry, CommandCategory, TypedCommand},
};
use crate::QuiltError;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event as CEvent, KeyCode, KeyEvent,
    KeyEventKind, KeyModifiers, MouseEventKind,
};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
    Frame, Terminal,
};
use std::collections::HashSet;
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

struct TtyGuard;
impl TtyGuard {
    fn enter() -> Result<Self, QuiltError> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(QuiltError::usage("TUI requires an interactive terminal"));
        }
        enable_raw_mode().map_err(|e| QuiltError::operation("TUI terminal", e.to_string()))?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)
            .map_err(|e| QuiltError::operation("TUI terminal", e.to_string()))?;
        Ok(guard)
    }
}
impl Drop for TtyGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[derive(Clone)]
struct History {
    commands: Vec<TypedCommand>,
    sources: Vec<(usize, usize, String)>,
    label: String,
}
struct PendingCommit {
    id: u64,
    commands: Vec<TypedCommand>,
    label: String,
    edit: Option<(usize, usize)>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Focus {
    #[default]
    Input,
    Table,
    Pipeline,
}
struct App {
    worker: Worker,
    committed: Vec<TypedCommand>,
    draft: String,
    cursor: usize,
    preview: Option<Preview>,
    mode: PreviewMode,
    generation: u64,
    latest_preview: u64,
    pending_commit: Option<PendingCommit>,
    history: Vec<History>,
    redo: Vec<History>,
    sources: Vec<(usize, usize, String)>,
    edit_step: Option<(usize, usize)>,
    explicit: HashSet<u64>,
    status: String,
    error: String,
    dirty_at: Option<Instant>,
    columns_offset: usize,
    rows_offset: usize,
    selected_row: usize,
    focus: Focus,
    fullscreen: bool,
    pipeline_offset: usize,
    detail: Option<DetailView>,
    working: Option<(u64, TaskPhase, Instant)>,
    cancelled: HashSet<u64>,
    latest_detail: u64,
    completion: usize,
    candidates: Vec<input::Completion>,
    prefix_mode: bool,
    help_overlay: bool,
    help_scroll: u16,
    picker: Option<ColumnPicker>,
    pending_picker: bool,
    preview_pending: bool,
    quitting: bool,
}

pub(super) fn run(
    initial: Vec<TypedCommand>,
    initial_source: Option<String>,
) -> Result<(), QuiltError> {
    let _guard = TtyGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal =
        Terminal::new(backend).map_err(|e| QuiltError::operation("TUI terminal", e.to_string()))?;
    let mut sources = Vec::new();
    if let Some(source) = initial_source {
        if !initial.is_empty() {
            sources.push((0, initial.len(), source));
        }
    }
    let mut app = App {
        worker: Worker::new(),
        committed: initial,
        draft: String::new(),
        cursor: 0,
        preview: None,
        mode: PreviewMode::Sample,
        generation: 0,
        latest_preview: 0,
        pending_commit: None,
        history: Vec::new(),
        redo: Vec::new(),
        sources,
        edit_step: None,
        explicit: HashSet::new(),
        status: "Ready".into(),
        error: String::new(),
        dirty_at: None,
        columns_offset: 0,
        rows_offset: 0,
        selected_row: 0,
        focus: Focus::Input,
        fullscreen: false,
        pipeline_offset: 0,
        detail: None,
        working: None,
        cancelled: HashSet::new(),
        latest_detail: 0,
        completion: 0,
        candidates: Vec::new(),
        prefix_mode: false,
        help_overlay: false,
        help_scroll: 0,
        picker: None,
        pending_picker: false,
        preview_pending: false,
        quitting: false,
    };
    app.refresh_candidates();
    if !app.committed.is_empty() {
        app.schedule_preview();
    }
    while !app.quitting {
        app.drain_events();
        if app
            .dirty_at
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(300))
        {
            app.schedule_preview();
        }
        terminal
            .draw(|f| render(f, &mut app))
            .map_err(|e| QuiltError::operation("TUI render", e.to_string()))?;
        if event::poll(Duration::from_millis(40))
            .map_err(|e| QuiltError::operation("TUI input", e.to_string()))?
        {
            match event::read().map_err(|e| QuiltError::operation("TUI input", e.to_string()))? {
                CEvent::Key(key) => app.key(key)?,
                CEvent::Paste(text) => {
                    if app.pending_commit.is_none()
                        && app.picker.is_none()
                        && app.detail.is_none()
                        && !app.help_overlay
                        && !app.prefix_mode
                        && app.focus == Focus::Input
                    {
                        app.insert(&text);
                        app.changed();
                    } else {
                        app.status = "Waiting for command validation…".into();
                    }
                }
                CEvent::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        app.rows_offset = app.rows_offset.saturating_add(3);
                        if app.focus == Focus::Table {
                            app.selected_row = app.selected_row.saturating_add(3);
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        app.rows_offset = app.rows_offset.saturating_sub(3);
                        if app.focus == Focus::Table {
                            app.selected_row = app.selected_row.saturating_sub(3);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    terminal
        .draw(|f| {
            let a = f.area();
            f.render_widget(
                Paragraph::new("Cancelling queued actions; finishing active work…")
                    .style(Style::default().fg(Color::Yellow)),
                a,
            );
        })
        .ok();
    drop(app); // Worker joins before the alternate screen is restored.
    drop(terminal);
    Ok(())
}

impl App {
    fn insert(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !is_terminal_control(*c)).collect();
        self.draft.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }
    fn changed(&mut self) {
        self.dirty_at = Some(Instant::now());
        self.latest_preview = 0;
        self.preview_pending = false;
        self.error.clear();
        self.completion = 0;
        self.refresh_candidates();
    }
    fn refresh_candidates(&mut self) {
        self.candidates = input::complete(
            &self.draft,
            self.cursor,
            self.preview
                .as_ref()
                .map_or(&[], |p| p.input_columns.as_slice()),
        );
        let editing_first_step = self.edit_step.is_some_and(|(start, _)| start == 0);
        if !self.committed.is_empty() && !editing_first_step {
            self.candidates.retain(|candidate| {
                candidate.kind != input::CompletionKind::Command
                    || registry()
                        .iter()
                        .find(|command| command.name == candidate.display)
                        .is_none_or(|command| command.category != CommandCategory::Initializer)
            });
        }
        if self.draft.trim().is_empty()
            && self
                .committed
                .last()
                .is_some_and(|command| command.name() == "select")
        {
            if let Some(index) = self.candidates.iter().position(|candidate| {
                candidate.kind == input::CompletionKind::Command && candidate.display == "count"
            }) {
                let count = self.candidates.remove(index);
                self.candidates.insert(0, count);
            }
        }
        if self.candidates.is_empty() {
            self.completion = 0;
        } else {
            self.completion %= self.candidates.len();
        }
    }
    fn queue(&mut self, commands: Vec<TypedCommand>, action: Action) -> Result<u64, QuiltError> {
        self.generation += 1;
        let id = self.generation;
        let is_preview = matches!(&action, Action::Preview(_) | Action::DraftPreview { .. });
        self.worker.submit(Request {
            id,
            commands,
            action,
        })?;
        if !is_preview {
            self.explicit.insert(id);
        }
        Ok(id)
    }
    fn candidate(&self) -> Result<Vec<TypedCommand>, QuiltError> {
        let all = if let Some((start, len)) = self.edit_step {
            self.committed[..start]
                .iter()
                .cloned()
                .chain(input::parse_commands(&self.draft)?)
                .chain(self.committed[start + len..].iter().cloned())
                .collect()
        } else {
            let mut all = self.committed.clone();
            if !self.draft.trim().is_empty() {
                all.extend(input::parse_commands(&self.draft)?);
            }
            all
        };
        Ok(all)
    }
    fn draft_context(&self) -> Vec<TypedCommand> {
        let mut context = if let Some((start, _)) = self.edit_step {
            self.committed[..start].to_vec()
        } else {
            self.committed.clone()
        };
        context.extend(input::preview_prefix(&self.draft[..self.cursor]));
        context
    }
    fn schedule_preview(&mut self) {
        self.dirty_at = None;
        if self.pending_commit.is_some() {
            return;
        }
        if self.draft.trim_start().starts_with(':') {
            self.preview_pending = false;
            self.pending_picker = false;
            self.status = "Built-in draft; Enter executes it".into();
            return;
        }
        if self.draft.trim().is_empty() && self.edit_step.is_none() {
            match self.queue(self.committed.clone(), Action::Preview(self.mode)) {
                Ok(id) => {
                    self.latest_preview = id;
                    self.preview_pending = true;
                }
                Err(e) => {
                    self.preview_pending = false;
                    self.error = safe(&e.to_string());
                }
            }
            return;
        }

        let context = self.draft_context();
        let empty_edit = self.draft.trim().is_empty() && self.edit_step.is_some();
        let (commands, issue) = if empty_edit {
            (
                context.clone(),
                Some("Empty edit draft; showing input before edited step".into()),
            )
        } else {
            match self.candidate() {
                Ok(commands) => (commands, None),
                Err(e) => (context.clone(), Some(safe(&e.to_string()))),
            }
        };
        if context.is_empty() && issue.is_some() {
            self.preview = None;
            self.preview_pending = false;
            self.pending_picker = false;
            self.refresh_candidates();
            self.status = format!("Draft incomplete; {}", issue.unwrap_or_default());
            return;
        }
        match self.queue(
            commands,
            Action::DraftPreview {
                mode: self.mode,
                context,
                issue,
            },
        ) {
            Ok(id) => {
                self.latest_preview = id;
                self.preview_pending = true;
                self.status = "Updating draft preview…".into();
            }
            Err(e) => {
                self.preview_pending = false;
                self.error = safe(&e.to_string());
            }
        }
    }
    fn drain_events(&mut self) {
        while let Some(event) = self.worker.poll() {
            self.handle_event(event);
        }
    }
    fn handle_event(&mut self, event: Event) {
        let Event { id, result } = event;
        if !matches!(&result, Ok(Outcome::Working(_))) {
            self.working = self.working.take().filter(|(task, _, _)| *task != id);
        }
        match result {
            Ok(Outcome::Preview(p)) => {
                if self.cancelled.remove(&id) {
                    self.explicit.remove(&id);
                    if self
                        .working
                        .as_ref()
                        .is_some_and(|(task, _, _)| *task == id)
                    {
                        self.working = None;
                    }
                    return;
                }
                if id != self.latest_preview
                    && self
                        .pending_commit
                        .as_ref()
                        .is_none_or(|pending| pending.id != id)
                {
                    return;
                }
                self.columns_offset = self.columns_offset.min(p.columns.len().saturating_sub(1));
                self.selected_row = self.selected_row.min(p.rows.len().saturating_sub(1));
                self.rows_offset = self.rows_offset.min(p.rows.len().saturating_sub(1));
                self.pipeline_offset = self.pipeline_offset.min(p.commands.len().saturating_sub(1));
                let draft_issue = p.draft_issue.clone();
                self.preview = Some(p);
                self.preview_pending = false;
                self.error.clear();
                self.status = draft_issue.map_or_else(
                    || "Preview updated".into(),
                    |issue| format!("Input before draft — {}", safe(&issue)),
                );
                self.refresh_candidates();
                if self.pending_picker {
                    self.pending_picker = false;
                    self.open_picker_now();
                }
                if let Some(pending) = self.pending_commit.take() {
                    if pending.id == id {
                        self.push_history(pending.label);
                        if let Some((start, len)) = pending.edit {
                            self.replace_source(
                                start,
                                len,
                                pending
                                    .commands
                                    .len()
                                    .saturating_sub(self.committed.len().saturating_sub(len)),
                                self.draft.clone(),
                            );
                        } else {
                            let start = self.committed.len();
                            let added = pending.commands.len().saturating_sub(start);
                            if added > 0 {
                                self.sources.push((start, added, self.draft.clone()));
                            }
                        }
                        self.committed = pending.commands;
                        self.edit_step = None;
                        self.draft.clear();
                        self.cursor = 0;
                        self.refresh_candidates();
                        self.status = "Command committed".into();
                    }
                }
            }
            Ok(Outcome::Working(phase)) => {
                if self.cancelled.contains(&id) {
                    return;
                }
                let started = self
                    .working
                    .as_ref()
                    .filter(|(task, _, _)| *task == id)
                    .map_or_else(Instant::now, |(_, _, started)| *started);
                self.working = Some((id, phase, started));
                self.status = format!("{}…", phase.label());
            }
            Ok(Outcome::Cancelled) => {
                self.cancelled.remove(&id);
                self.explicit.remove(&id);
                if self
                    .working
                    .as_ref()
                    .is_some_and(|(task, _, _)| *task == id)
                {
                    self.working = None;
                }
                if id == self.latest_preview {
                    self.preview_pending = false;
                    self.pending_picker = false;
                    self.dirty_at = None;
                }
                if self
                    .pending_commit
                    .as_ref()
                    .is_some_and(|pending| pending.id == id)
                {
                    self.pending_commit = None;
                    self.preview_pending = false;
                }
                self.status = "Action cancelled".into();
            }
            Ok(Outcome::Detail {
                title,
                text,
                truncated,
            }) => {
                let was_cancelled = self.cancelled.remove(&id);
                let is_current = id == self.latest_detail;
                self.explicit.remove(&id);
                self.working = self.working.take().filter(|(task, _, _)| *task != id);
                if was_cancelled || !is_current {
                    return;
                }
                self.latest_detail = 0;
                self.detail = Some(DetailView::new(
                    if truncated {
                        format!("{title} (truncated)")
                    } else {
                        title
                    },
                    text,
                ));
            }
            Ok(Outcome::Cached(c)) => {
                if self.cancelled.remove(&id) {
                    self.explicit.remove(&id);
                    return;
                }
                self.working = self.working.take().filter(|(task, _, _)| *task != id);
                if self.explicit.remove(&id) {
                    if let Some(preview) = &mut self.preview {
                        preview.cache = Some(c.clone());
                    }
                    self.status = format!("Cache: {} bytes, {} steps", c.bytes, c.steps);
                    self.error.clear();
                }
            }
            Ok(Outcome::CacheCleared) => {
                if self.cancelled.remove(&id) {
                    self.explicit.remove(&id);
                    return;
                }
                self.working = self.working.take().filter(|(task, _, _)| *task != id);
                if self.explicit.remove(&id) {
                    if let Some(preview) = &mut self.preview {
                        preview.cache = None;
                    }
                    self.status = "Cache cleared".into();
                    self.error.clear();
                }
            }
            Ok(Outcome::Saved(path)) => {
                self.cancelled.remove(&id);
                self.explicit.remove(&id);
                self.working = self.working.take().filter(|(task, _, _)| *task != id);
                self.status = format!("Saved {}", safe(&path.to_string_lossy()));
                self.error.clear();
            }
            Err(e) => {
                if self.cancelled.remove(&id) {
                    self.explicit.remove(&id);
                    return;
                }
                if self
                    .working
                    .as_ref()
                    .is_some_and(|(task, _, _)| *task == id)
                {
                    self.working = None;
                }
                let is_commit = self
                    .pending_commit
                    .as_ref()
                    .is_some_and(|pending| pending.id == id);
                if self.explicit.remove(&id) || id == self.latest_preview || is_commit {
                    self.error = safe(&e.to_string());
                    if id == self.latest_preview {
                        self.preview_pending = false;
                        self.pending_picker = false;
                    }
                    if is_commit {
                        self.pending_commit = None;
                    }
                }
            }
        }
    }
    fn inspect(&mut self, column: Option<usize>) {
        let Some(preview) = &self.preview else {
            self.status = "Wait for a preview before opening row details".into();
            return;
        };
        if preview.rows.is_empty() {
            self.status = "There are no preview rows to inspect".into();
            return;
        }
        let row = self.selected_row.min(preview.rows.len().saturating_sub(1));
        match self.queue(
            preview.commands.clone(),
            Action::Inspect {
                mode: preview.mode,
                row,
                column,
                source_version: preview.source_version.clone(),
            },
        ) {
            Ok(id) => {
                self.latest_detail = id;
                self.working = Some((id, TaskPhase::Inspecting, Instant::now()));
                self.status = format!("Reading row {}…", row + 1);
            }
            Err(e) => self.error = safe(&e.to_string()),
        }
    }
    fn edit_pipeline_step(&mut self) {
        let step = self.pipeline_offset;
        if let Some((start, len, text)) = self
            .sources
            .iter()
            .find(|(start, len, _)| step >= *start && step < start.saturating_add(*len))
            .cloned()
        {
            self.edit_step = Some((start, len));
            self.draft = text;
            self.cursor = self.draft.len();
            self.focus = Focus::Input;
            self.fullscreen = false;
            self.changed();
            self.status = format!(
                "Editing source group, steps {}–{}; Enter validates replacement",
                start + 1,
                start + len
            );
        } else {
            self.status = format!(
                "Step {} has no editable source text in this TUI session",
                step + 1
            );
        }
    }
    fn move_table(&mut self, dx: isize, dy: isize) {
        if let Some(preview) = &self.preview {
            let last_column = preview.columns.len().saturating_sub(1);
            self.columns_offset = self
                .columns_offset
                .saturating_add_signed(dx)
                .min(last_column);
            let last_row = preview.rows.len().saturating_sub(1);
            self.selected_row = self.selected_row.saturating_add_signed(dy).min(last_row);
        }
    }
    fn cancel_work(&mut self) {
        let mut ids = self.explicit.clone();
        if let Some((id, _, _)) = self.working.take() {
            ids.insert(id);
        }
        if let Some(pending) = self.pending_commit.take() {
            ids.insert(pending.id);
            self.preview_pending = false;
        }
        if self.preview_pending && self.latest_preview != 0 {
            ids.insert(self.latest_preview);
        }
        self.dirty_at = None;
        self.preview_pending = false;
        self.pending_picker = false;
        self.latest_preview = 0;
        let had_ids = !ids.is_empty();
        self.cancelled.extend(ids.iter().copied());
        for id in ids {
            self.explicit.remove(&id);
        }
        self.latest_detail = 0;
        let queued_or_active = self.worker.cancel();
        if queued_or_active || had_ids {
            self.status = "Cancelling active action…".into();
        } else {
            self.status = "No active action to cancel".into();
        }
    }
    fn push_history(&mut self, label: String) {
        self.history.push(History {
            commands: self.committed.clone(),
            sources: self.sources.clone(),
            label,
        });
        self.redo.clear();
    }
    fn replace_source(&mut self, start: usize, old_len: usize, new_len: usize, text: String) {
        let end = start + old_len;
        self.sources.retain(|(s, _, _)| *s < start || *s >= end);
        let delta = new_len as isize - old_len as isize;
        for (s, _, _) in &mut self.sources {
            if *s >= end {
                *s = (*s as isize + delta).max(0) as usize;
            }
        }
        if new_len > 0 {
            self.sources.push((start, new_len, text));
            self.sources.sort_by_key(|(s, _, _)| *s);
        }
    }
    fn commit(&mut self) {
        if self.pending_commit.is_some() {
            if self.draft.trim() == ":quit" {
                self.quitting = true;
            }
            return;
        }
        if self.draft.trim().is_empty() {
            return;
        }
        self.pending_picker = false;
        if self.draft.trim_start().starts_with(':') {
            self.directive();
            return;
        }
        match input::parse_commands(&self.draft) {
            Err(e) => self.error = safe(&e.to_string()),
            Ok(parsed) => {
                let edit = self.edit_step;
                let next = if let Some((start, len)) = edit {
                    let mut v = self.committed[..start].to_vec();
                    v.extend(parsed);
                    v.extend_from_slice(&self.committed[start + len..]);
                    v
                } else {
                    let mut v = self.committed.clone();
                    v.extend(parsed);
                    v
                };
                let invalid = next.iter().any(|c| {
                    matches!(
                        c.category(),
                        CommandCategory::Finalizer | CommandCategory::Automation
                    )
                });
                if invalid {
                    self.error =
                        "Finalizers and run are available only through explicit built-ins".into();
                    return;
                }
                match self.queue(next.clone(), Action::Preview(self.mode)) {
                    Ok(id) => {
                        self.latest_preview = id;
                        self.preview_pending = true;
                        self.dirty_at = None;
                        self.pending_commit = Some(PendingCommit {
                            id,
                            commands: next,
                            label: self.draft.clone(),
                            edit,
                        });
                        self.status = "Validating command…".into();
                    }
                    Err(e) => self.error = e.to_string(),
                }
            }
        }
    }
    fn directive(&mut self) {
        let parsed = input::words(&self.draft);
        let words = match parsed {
            Ok(words) => words,
            Err(e) => {
                self.error = safe(&e.to_string());
                return;
            }
        };
        let cmd = words.first().map(String::as_str).unwrap_or("");
        let args = &words[1..];
        let result=match cmd {
            ":quit" if args.is_empty()=>{self.quitting=true;Ok(())},
            ":undo" if args.is_empty()=>{self.undo();Ok(())},":redo" if args.is_empty()=>{self.redo();Ok(())},
            ":write" if args.len()==1=>self.queue(self.committed.clone(),Action::ExportYaml(PathBuf::from(&args[0]))).map(|_|()),
            ":export" if args.len() == 1 || args.len() == 3 => {
                let path = PathBuf::from(&args[0]);
                let format = if args.len() == 3 {
                    if args[1] != "--format" {
                        Err(QuiltError::usage("use :export PATH [--format csv|jsonl]"))
                    } else {
                        OutputFormat::parse(&args[2].to_ascii_lowercase())
                    }
                } else {
                    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
                    if extension.eq_ignore_ascii_case("jsonl") || extension.eq_ignore_ascii_case("ndjson") {
                        Ok(OutputFormat::Jsonl)
                    } else {
                        Ok(OutputFormat::Csv)
                    }
                };
                format.and_then(|format| {
                    self.queue(
                        self.committed.clone(),
                        Action::ExportData { path, format },
                    )
                    .map(|_| ())
                })
            }
            ":cache" if args.is_empty()=>self.queue(self.committed.clone(),Action::Cache).map(|_|()),
            ":cache" if args.len()==1=>self.queue(self.committed.clone(),Action::SaveCache(PathBuf::from(&args[0]))).map(|_|()),
            ":clear-cache" if args.is_empty()=>self.queue(self.committed.clone(),Action::ClearCache).map(|_|()),
            ":edit" if args.len()==1=>{let n=args[0].parse::<usize>().ok().filter(|n|*n>0).ok_or_else(||QuiltError::usage(":edit requires a 1-based step number"));match n{Err(e)=>Err(e),Ok(n)=>if let Some((start,len,text))=self.sources.iter().find(|(s,l,_)|n>*s&&n<=*s+*l).cloned(){self.edit_step=Some((start,len));self.draft=text;self.cursor=self.draft.len();self.status=format!("Editing source group for steps {}–{}; Enter validates the replacement",start+1,start+len);self.changed();Ok(())}else{Err(QuiltError::usage("Original text is available only for commands entered in this TUI session"))}}},
            _=>Err(QuiltError::usage("Use :write PATH, :export PATH [--format csv|jsonl], :cache [PATH], :clear-cache, :undo, :redo, :edit STEP, or :quit (quote paths with spaces)")),
        };
        match result {
            Ok(()) => {
                if cmd != ":edit" {
                    self.draft.clear();
                    self.cursor = 0;
                    self.edit_step = None;
                    self.dirty_at = None;
                    self.refresh_candidates();
                }
                self.error.clear();
            }
            Err(e) => self.error = safe(&e.to_string()),
        }
    }
    fn key(&mut self, k: KeyEvent) -> Result<(), QuiltError> {
        if k.kind == KeyEventKind::Release {
            return Ok(());
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quitting = true;
            return Ok(());
        }
        if let Some(detail) = &mut self.detail {
            if detail.handle_key(k) {
                self.detail = None;
                self.latest_detail = 0;
            }
            return Ok(());
        }
        if self.help_overlay {
            match k.code {
                KeyCode::Esc | KeyCode::Char('?') => self.help_overlay = false,
                KeyCode::Up => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::Down => self.help_scroll = self.help_scroll.saturating_add(1),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(8),
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(8),
                KeyCode::Home => self.help_scroll = 0,
                KeyCode::End => self.help_scroll = u16::MAX,
                _ => {}
            }
            return Ok(());
        }
        if self.picker.is_some() {
            if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('a') {
                self.prefix_mode = true;
                self.status =
                    "Prefix: ? help; Esc cancels prefix; close picker for other actions".into();
                return Ok(());
            }
            if self.prefix_mode {
                self.prefix_mode = false;
                match k.code {
                    KeyCode::Esc => {
                        self.status = "Prefix cancelled; column selection remains open".into()
                    }
                    KeyCode::Char('?') => {
                        self.help_overlay = true;
                        self.help_scroll = 0;
                    }
                    _ => self.status = "Close column selection before other prefix actions".into(),
                }
                return Ok(());
            }
            let result = self.picker.as_mut().unwrap().handle_key(k);
            match result {
                PickerResult::Continue => {}
                PickerResult::Cancel => {
                    self.picker = None;
                    self.status = "Column selection cancelled".into();
                }
                PickerResult::Accept(columns) => {
                    self.picker = None;
                    self.apply_picker_selection(columns);
                }
            }
            return Ok(());
        }
        if self.prefix_mode {
            self.prefix_mode = false;
            if (self.pending_commit.is_some() || self.working.is_some())
                && !matches!(
                    k.code,
                    KeyCode::Esc
                        | KeyCode::Char('?')
                        | KeyCode::Char('h')
                        | KeyCode::Char('t')
                        | KeyCode::Char('b')
                        | KeyCode::Char('i')
                        | KeyCode::Char('f')
                        | KeyCode::Char('d')
                        | KeyCode::Char('e')
                        | KeyCode::Char('z')
                        | KeyCode::Char('q')
                )
            {
                self.status = "Action in progress; help and navigation remain available".into();
                return Ok(());
            }
            match k.code {
                KeyCode::Esc => self.status = "Prefix cancelled".into(),
                KeyCode::Char('?') => {
                    self.help_overlay = true;
                    self.help_scroll = 0;
                }
                KeyCode::Char('h') => {
                    let selected = self.completion.min(self.candidates.len().saturating_sub(1));
                    let command = self.candidates.get(selected);
                    let title = command.map_or_else(
                        || "Command help".to_owned(),
                        |c| format!("Help: {}", c.display),
                    );
                    let text = command
                        .and_then(input::completion_help)
                        .unwrap_or_else(|| input::help(&self.draft, self.cursor));
                    self.detail = Some(DetailView::new(title, text));
                }
                KeyCode::Char('t') => self.focus = Focus::Table,
                KeyCode::Char('b') => self.focus = Focus::Pipeline,
                KeyCode::Char('i') => {
                    self.focus = Focus::Input;
                    self.fullscreen = false;
                }
                KeyCode::Char('f') => {
                    if self.fullscreen {
                        self.fullscreen = false;
                    } else {
                        if self.focus == Focus::Input {
                            self.focus = Focus::Table;
                        }
                        self.fullscreen = true;
                    }
                }
                KeyCode::Char('d') => self.inspect(None),
                KeyCode::Char('e') => {
                    if let Some(issue) = self.preview.as_ref().and_then(|p| p.draft_issue.as_ref())
                    {
                        self.detail = Some(DetailView::new("Draft issue details", issue.clone()));
                    } else if self.error.is_empty() {
                        self.status = "There is no current error to expand".into();
                    } else {
                        self.detail = Some(DetailView::new("Error details", self.error.clone()));
                    }
                }
                KeyCode::Char('o') => self.prompt_action("export"),
                KeyCode::Char('z') => self.cancel_work(),
                KeyCode::Char('c') => self.open_picker(),
                KeyCode::Char('w') => self.prompt_action("write"),
                KeyCode::Char('C') => {
                    let commands = self.committed.clone();
                    match self.queue(commands, Action::Cache) {
                        Ok(_) => self.status = "Creating session checkpoint…".into(),
                        Err(e) => self.error = safe(&e.to_string()),
                    }
                }
                KeyCode::Char('x') => {
                    let commands = self.committed.clone();
                    if let Err(e) = self.queue(commands, Action::ClearCache) {
                        self.error = safe(&e.to_string());
                    }
                }
                KeyCode::Char('p') => {
                    self.mode = if self.mode == PreviewMode::Sample {
                        PreviewMode::Full
                    } else {
                        PreviewMode::Sample
                    };
                    self.schedule_preview();
                }
                KeyCode::Char('u') => self.undo(),
                KeyCode::Char('r') => self.redo(),
                KeyCode::Char('q') => self.quitting = true,
                _ => self.status = "Unknown prefix key; press Ctrl+A ? for help".into(),
            }
            return Ok(());
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            if k.code == KeyCode::Char('a') {
                self.prefix_mode = true;
                self.status = "Prefix: t table b pipeline i input h help d details e error f full screen z cancel o export; c columns C cache w write x clear p mode u/r undo/redo q quit".into();
            }
            return Ok(());
        }
        let focus_at_key_start = self.focus;
        match self.focus {
            Focus::Table => match k.code {
                KeyCode::Esc => {
                    self.focus = Focus::Input;
                    self.fullscreen = false;
                }
                KeyCode::Up | KeyCode::Char('k') => self.move_table(0, -1),
                KeyCode::Down | KeyCode::Char('j') => self.move_table(0, 1),
                KeyCode::Left | KeyCode::Char('h') => self.move_table(-1, 0),
                KeyCode::Right | KeyCode::Char('l') => self.move_table(1, 0),
                KeyCode::PageUp => self.move_table(0, -20),
                KeyCode::PageDown => self.move_table(0, 20),
                KeyCode::Enter => self.inspect(Some(self.columns_offset)),
                _ => {}
            },
            Focus::Pipeline => match k.code {
                KeyCode::Esc => {
                    self.focus = Focus::Input;
                    self.fullscreen = false;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.pipeline_offset = self.pipeline_offset.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let last = self.committed.len().saturating_sub(1);
                    self.pipeline_offset = self.pipeline_offset.saturating_add(1).min(last);
                }
                KeyCode::PageUp => self.pipeline_offset = self.pipeline_offset.saturating_sub(8),
                KeyCode::PageDown => {
                    let last = self.committed.len().saturating_sub(1);
                    self.pipeline_offset = self.pipeline_offset.saturating_add(8).min(last);
                }
                KeyCode::Enter if self.pending_commit.is_some() => {
                    self.status = "Waiting for validation before editing a pipeline step".into();
                }
                KeyCode::Enter => self.edit_pipeline_step(),
                _ => {}
            },
            Focus::Input => {}
        }
        if focus_at_key_start != Focus::Input {
            return Ok(());
        }
        if self.pending_commit.is_some() {
            if k.code == KeyCode::Enter && self.draft.trim() == ":quit" {
                self.quitting = true;
            } else {
                self.status = "Waiting for validation… (Ctrl+A h help; Ctrl+A z cancels)".into();
            }
            return Ok(());
        }
        if k.modifiers.contains(KeyModifiers::ALT) {
            match k.code {
                KeyCode::Left => {
                    let last = self
                        .preview
                        .as_ref()
                        .map_or(0, |p| p.columns.len().saturating_sub(1));
                    self.columns_offset = self.columns_offset.min(last).saturating_sub(1);
                }
                KeyCode::Right => {
                    let last = self
                        .preview
                        .as_ref()
                        .map_or(0, |p| p.columns.len().saturating_sub(1));
                    self.columns_offset = self.columns_offset.saturating_add(1).min(last);
                }
                _ => {}
            }
            return Ok(());
        }
        match k.code {
            KeyCode::Esc => {
                self.pending_picker = false;
                self.draft.clear();
                self.cursor = 0;
                self.dirty_at = None;
                self.edit_step = None;
                self.error.clear();
                self.completion = 0;
                self.refresh_candidates();
                if self.committed.is_empty() {
                    self.preview = None;
                    self.latest_preview = 0;
                    self.preview_pending = false;
                } else {
                    self.schedule_preview();
                }
            }
            KeyCode::Enter => self.commit(),
            KeyCode::Char(':') if self.draft.is_empty() => {
                self.insert(":");
                self.changed()
            }
            KeyCode::Char(c) => {
                self.insert(&c.to_string());
                self.changed()
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let p = prev_boundary(&self.draft, self.cursor);
                    self.draft.replace_range(p..self.cursor, "");
                    self.cursor = p;
                    self.changed();
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.draft.len() {
                    let n = next_boundary(&self.draft, self.cursor);
                    self.draft.replace_range(self.cursor..n, "");
                    self.changed();
                }
            }
            KeyCode::Left => {
                let old_context = self.draft_context();
                self.cursor = prev_boundary(&self.draft, self.cursor);
                self.refresh_after_cursor_move(old_context);
            }
            KeyCode::Right => {
                let old_context = self.draft_context();
                self.cursor = next_boundary(&self.draft, self.cursor);
                self.refresh_after_cursor_move(old_context);
            }
            KeyCode::Home => {
                let old_context = self.draft_context();
                self.cursor = 0;
                self.refresh_after_cursor_move(old_context);
            }
            KeyCode::End => {
                let old_context = self.draft_context();
                self.cursor = self.draft.len();
                self.refresh_after_cursor_move(old_context);
            }
            KeyCode::Tab => self.complete(),
            KeyCode::Up => {
                if !self.candidates.is_empty() {
                    self.completion =
                        (self.completion + self.candidates.len() - 1) % self.candidates.len();
                }
            }
            KeyCode::Down => {
                if !self.candidates.is_empty() {
                    self.completion = (self.completion + 1) % self.candidates.len();
                }
            }
            KeyCode::PageUp => self.rows_offset = self.rows_offset.saturating_sub(20),
            KeyCode::PageDown => self.rows_offset = self.rows_offset.saturating_add(20),
            _ => {}
        }
        Ok(())
    }
    fn refresh_after_cursor_move(&mut self, old_context: Vec<TypedCommand>) {
        if self.draft_context() != old_context {
            self.changed();
        } else {
            self.refresh_candidates();
        }
    }
    fn open_picker(&mut self) {
        if input::select_target(&self.draft, self.cursor).is_none() {
            self.status = "Column picker is available for a select command or blank draft".into();
            return;
        }
        if self.dirty_at.is_some() {
            self.pending_picker = true;
            self.schedule_preview();
        } else if self.preview_pending || self.latest_preview == 0 || self.preview.is_none() {
            self.pending_picker = true;
            if !self.preview_pending {
                self.schedule_preview();
            }
        } else {
            self.open_picker_now();
        }
    }
    fn open_picker_now(&mut self) {
        let Some(target) = input::select_target(&self.draft, self.cursor) else {
            self.status = "Column picker is available for a select command or blank draft".into();
            return;
        };
        let columns = self
            .preview
            .as_ref()
            .map_or_else(Vec::new, |p| p.input_columns.clone());
        self.picker = Some(ColumnPicker::new(columns, target.columns));
    }
    fn apply_picker_selection(&mut self, columns: Vec<String>) {
        let Some(target) = input::select_target(&self.draft, self.cursor) else {
            self.status = "Draft changed; column selection was not applied".into();
            return;
        };
        let replacement = input::format_select(&columns);
        self.draft.replace_range(target.range.clone(), &replacement);
        self.cursor = target.range.start + replacement.len();
        self.changed();
        self.status = "Column selection updated; Enter to commit".into();
    }
    fn complete(&mut self) {
        if let Some(c) = self
            .candidates
            .get(self.completion % self.candidates.len().max(1))
        {
            self.draft.replace_range(c.range.clone(), &c.replacement);
            self.cursor = c.range.start + c.replacement.len();
            self.changed();
        }
    }
    fn prompt_action(&mut self, kind: &str) {
        if kind == "write" {
            self.draft = ":write ".into();
            self.cursor = self.draft.len();
            self.edit_step = None;
            self.dirty_at = None;
            self.latest_preview = 0;
            self.preview_pending = false;
            self.pending_picker = false;
            self.completion = 0;
            self.error.clear();
            self.refresh_candidates();
            self.status = "Enter a YAML path, then press Enter".into();
        } else if kind == "export" {
            self.draft = ":export ".into();
            self.cursor = self.draft.len();
            self.edit_step = None;
            self.dirty_at = None;
            self.latest_preview = 0;
            self.preview_pending = false;
            self.pending_picker = false;
            self.completion = 0;
            self.error.clear();
            self.refresh_candidates();
            self.status = "Enter a new path to export the full data set (CSV by default)".into();
        }
    }
    fn reset_draft_for_history_change(&mut self) {
        self.draft.clear();
        self.cursor = 0;
        self.edit_step = None;
        self.dirty_at = None;
        self.latest_preview = 0;
        self.preview_pending = false;
        self.pending_picker = false;
        self.completion = 0;
        self.error.clear();
        self.refresh_candidates();
    }
    fn preview_committed(&mut self) {
        if self.committed.is_empty() {
            self.preview = None;
            self.latest_preview = 0;
            self.preview_pending = false;
        } else {
            self.schedule_preview();
        }
    }
    fn undo(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.redo.push(History {
                commands: self.committed.clone(),
                sources: self.sources.clone(),
                label: prev.label,
            });
            self.committed = prev.commands;
            self.sources = prev.sources;
            self.reset_draft_for_history_change();
            self.preview_committed();
        }
    }
    fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.history.push(History {
                commands: self.committed.clone(),
                sources: self.sources.clone(),
                label: next.label,
            });
            self.committed = next.commands;
            self.sources = next.sources;
            self.reset_draft_for_history_change();
            self.preview_committed();
        }
    }
}
fn prev_boundary(s: &str, p: usize) -> usize {
    s[..p].char_indices().next_back().map_or(0, |(i, _)| i)
}
fn next_boundary(s: &str, p: usize) -> usize {
    s[p..].char_indices().nth(1).map_or(s.len(), |(i, _)| p + i)
}
fn is_terminal_control(c: char) -> bool {
    c.is_control()
        || matches!(c,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
}
fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if is_terminal_control(c) {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);
    let (table_area, context_area, headers_area) = if chunks[1].width >= 86 {
        let widths = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(24),
                Constraint::Min(36),
                Constraint::Percentage(28),
            ])
            .split(chunks[1]);
        (widths[1], widths[2], widths[0])
    } else {
        let stacked = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(chunks[1]);
        let sidebar_width = chunks[1].width.saturating_sub(14).clamp(10, 24);
        let row = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sidebar_width), Constraint::Min(1)])
            .split(stacked[0]);
        (row[1], stacked[1], row[0])
    };
    let shown_mode = app.preview.as_ref().map_or(app.mode, |p| p.mode);
    let preview_state = if app.dirty_at.is_some() || app.preview_pending {
        " | Updating preview…"
    } else {
        ""
    };
    let banner = format!(
        " QUILT | {} preview{} | Ctrl+A ? help | Ctrl+A c columns | Tab complete | Alt←/→ columns | Enter apply",
        if shown_mode == PreviewMode::Sample {
            format!(
                "Sample first {} rows",
                app.preview
                    .as_ref()
                    .and_then(|p| p.sample_rows)
                    .unwrap_or(super::SAMPLE_ROWS)
            )
        } else {
            "Full data".into()
        },
        preview_state
    );
    f.render_widget(
        Paragraph::new(truncate_display(&banner, area.width as usize)).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        chunks[0],
    );
    if app.fullscreen && app.focus == Focus::Table {
        draw_table(f, chunks[1], app);
        draw_input(f, chunks[2], app);
        draw_footer(f, chunks[3], app);
        if app.help_overlay {
            draw_keyboard_help(f, area, &mut app.help_scroll);
        } else if let Some(picker) = &mut app.picker {
            picker.render(f);
        } else if let Some(detail) = &mut app.detail {
            detail.render(f, area);
        }
        return;
    }
    if app.focus == Focus::Pipeline {
        if app.fullscreen {
            draw_pipeline(f, chunks[1], app);
        } else {
            draw_headers(f, headers_area, app);
            draw_table(f, table_area, app);
            draw_pipeline(f, context_area, app);
        }
        draw_input(f, chunks[2], app);
        draw_footer(f, chunks[3], app);
        if app.help_overlay {
            draw_keyboard_help(f, area, &mut app.help_scroll);
        } else if let Some(picker) = &mut app.picker {
            picker.render(f);
        } else if let Some(detail) = &mut app.detail {
            detail.render(f, area);
        }
        return;
    }
    draw_headers(f, headers_area, app);
    draw_table(f, table_area, app);
    let (completion_area, help_area, pipeline_area) = if context_area.height >= 25 {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Min(8),
                Constraint::Length(5),
            ])
            .split(context_area);
        (areas[0], areas[1], Some(areas[2]))
    } else {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(42), Constraint::Min(1)])
            .split(context_area);
        (areas[0], areas[1], None)
    };
    let candidate_rows = completion_area.height.saturating_sub(2) as usize;
    let two_lines = candidate_rows >= 4 && completion_area.width >= 26;
    let lines_per_candidate = if two_lines { 2 } else { 1 };
    let visible_candidates = (candidate_rows / lines_per_candidate).max(1);
    let selected_candidate = app.completion.min(app.candidates.len().saturating_sub(1));
    let candidate_start = selected_candidate
        .saturating_sub(visible_candidates / 2)
        .min(app.candidates.len().saturating_sub(visible_candidates));
    let candidate_end = (candidate_start + visible_candidates).min(app.candidates.len());
    let completion_title = if app.candidates.is_empty() {
        "Completions 0/0".to_owned()
    } else {
        format!(
            "Completions {}-{} /{} ↑↓",
            candidate_start + 1,
            candidate_end,
            app.candidates.len()
        )
    };
    let candidate_width = completion_area.width.saturating_sub(2) as usize;
    let mut completion_lines = Vec::new();
    for (index, candidate) in app
        .candidates
        .iter()
        .enumerate()
        .take(candidate_end)
        .skip(candidate_start)
    {
        let selected = index == selected_candidate;
        let marker = if selected { "› " } else { "  " };
        let name_style = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        };
        if two_lines {
            completion_lines.push(Line::from(vec![
                Span::styled(marker, name_style),
                Span::styled(
                    truncate_display(
                        &safe(&candidate.display),
                        candidate_width.saturating_sub(UnicodeWidthStr::width(marker)),
                    ),
                    name_style,
                ),
            ]));
            completion_lines.push(Line::from(Span::styled(
                truncate_display(
                    &format!("  {}", safe(&candidate.description)),
                    candidate_width,
                ),
                Style::default().fg(Color::DarkGray),
            )));
        } else {
            let label = format!(
                "{marker}{} — {}",
                safe(&candidate.display),
                safe(&candidate.description)
            );
            completion_lines.push(Line::from(Span::styled(
                truncate_display(&label, candidate_width),
                name_style,
            )));
        }
    }
    if app.candidates.is_empty() {
        completion_lines.push(Line::from("No completions"));
    }
    f.render_widget(
        Paragraph::new(completion_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(completion_title),
        ),
        completion_area,
    );
    let command_help = app
        .candidates
        .get(selected_candidate)
        .and_then(input::completion_help);
    let help = command_help
        .clone()
        .unwrap_or_else(|| input::help(&app.draft, app.cursor));
    let help_title = command_help
        .as_ref()
        .and_then(|_| app.candidates.get(selected_candidate))
        .map_or_else(|| "Help".to_owned(), |c| format!("Help: {}", c.display));
    f.render_widget(
        Paragraph::new(
            help.lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .block(Block::default().borders(Borders::ALL).title(help_title))
        .wrap(Wrap { trim: false }),
        help_area,
    );
    if let Some(pipeline_area) = pipeline_area {
        draw_pipeline(f, pipeline_area, app);
    }
    draw_input(f, chunks[2], app);
    draw_footer(f, chunks[3], app);
    if app.help_overlay {
        draw_keyboard_help(f, area, &mut app.help_scroll);
    } else if let Some(picker) = &mut app.picker {
        picker.render(f);
    } else if let Some(detail) = &mut app.detail {
        detail.render(f, area);
    }
}

fn draw_input(f: &mut Frame, input_area: Rect, app: &App) {
    let safe_draft = safe(&app.draft);
    let selected_candidate = app.completion.min(app.candidates.len().saturating_sub(1));
    let cursor_col = UnicodeWidthStr::width(safe(&app.draft[..app.cursor]).as_str());
    let input_width = input_area.width.saturating_sub(2) as usize;
    let input_scroll = cursor_col.saturating_sub(input_width.saturating_sub(1));
    let selected_completion = app.candidates.get(selected_candidate);
    let ghost = input::placeholder(
        &app.draft,
        app.cursor,
        selected_completion,
        !app.committed.is_empty(),
    );
    let input_line = Line::from(vec![
        Span::raw(safe_draft),
        Span::styled(
            safe(&ghost),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        ),
    ]);
    let input_title = if let Some((start, len)) = app.edit_step {
        if len == 1 {
            format!("Edit step {} | Enter replace, Esc cancel", start + 1)
        } else {
            format!(
                "Edit source group, steps {}–{} | Enter replace, Esc cancel",
                start + 1,
                start + len
            )
        }
    } else if app.draft.trim_start().starts_with(":export") {
        "Export full data | Enter writes a new CSV/JSONL file".to_owned()
    } else {
        "Command (Tab complete, Enter commit)".to_owned()
    };
    let mut input_block = Block::default().borders(Borders::ALL).title(input_title);
    if app.focus == Focus::Input {
        input_block = input_block.border_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    let input = Paragraph::new(input_line)
        .scroll((0, input_scroll.min(u16::MAX as usize) as u16))
        .block(input_block);
    f.render_widget(input, input_area);
    if input_area.width > 2 && input_area.height > 1 {
        let x = input_area.x + 1 + cursor_col.saturating_sub(input_scroll) as u16;
        if app.focus == Focus::Input
            && app.detail.is_none()
            && app.picker.is_none()
            && !app.help_overlay
        {
            f.set_cursor_position((
                x.min(input_area.right().saturating_sub(1)),
                (input_area.y + 1).min(input_area.bottom().saturating_sub(1)),
            ));
        }
    }
}

fn draw_footer(f: &mut Frame, footer_area: Rect, app: &App) {
    let compact = footer_area.width <= 100 || app.working.is_some();
    let focus_hint = match (app.focus, compact) {
        (Focus::Input, true) => "Input | Tab/↑↓ | ^A h/t/b/o",
        (Focus::Pipeline, true) => "Pipeline | ↑↓ select, Enter edit | Esc input",
        (Focus::Table, true) => "Table | ↑↓ row ←→ col | Enter detail",
        (Focus::Input, false) => "Input | Tab/↑↓ complete | ^A h help t/b panes o export",
        (Focus::Pipeline, false) => "Pipeline | ↑↓ select Enter edit Esc input | ^A f",
        (Focus::Table, false) if app.fullscreen => {
            "Table+ | ↑↓ row ←→ col | Enter detail Esc input | ^A d row f shrink"
        }
        (Focus::Table, false) => {
            "Table | ↑↓ row ←→ col | Enter detail Esc input | ^A d row f expand"
        }
    };
    let status = if !app.error.is_empty() {
        format!("! {} | ^A e", truncate_display(&safe(&app.error), 15))
    } else if let Some((_, phase, started)) = app.working.as_ref() {
        let phase = match phase {
            TaskPhase::Loading => "Loading",
            TaskPhase::Computing => "Computing",
            TaskPhase::Inspecting => "Inspecting",
            TaskPhase::Saving => "Saving",
            TaskPhase::Caching => "Caching",
        };
        format!("{} {}s | ^A z cancel", phase, started.elapsed().as_secs())
    } else {
        safe(&app.status)
    };
    let footer = format!("{} | {}", focus_hint, status);
    f.render_widget(
        Paragraph::new(truncate_display(&footer, footer_area.width as usize)).style(
            Style::default().fg(if app.error.is_empty() {
                Color::Gray
            } else {
                Color::Red
            }),
        ),
        footer_area,
    );
}

fn draw_pipeline(f: &mut Frame, area: Rect, app: &mut App) {
    let commands = app.committed.as_slice();
    let total = commands.len();
    app.pipeline_offset = app.pipeline_offset.min(total.saturating_sub(1));
    let inner_rows = area.height.saturating_sub(2) as usize;
    let visible = inner_rows.max(1);
    let start = app
        .pipeline_offset
        .saturating_sub(visible.saturating_sub(1) / 2)
        .min(total.saturating_sub(visible));
    let end = (start + visible).min(total);
    let mut lines = Vec::new();
    if total == 0 {
        lines.push(Line::from("No pipeline steps"));
    } else {
        for (index, command) in commands.iter().enumerate().take(end).skip(start) {
            let marker = if index == app.pipeline_offset {
                "› "
            } else {
                "  "
            };
            let source = app.sources.iter().find(|(source_start, len, _)| {
                index >= *source_start && index < source_start.saturating_add(*len)
            });
            let line = if let Some((source_start, len, original)) = source {
                let stage = input::stage_sources(original)
                    .get(index - *source_start)
                    .copied()
                    .unwrap_or("");
                if *len > 1 {
                    format!(
                        "{marker}{}. [source group {}–{}] {}",
                        index + 1,
                        *source_start + 1,
                        *source_start + *len,
                        stage
                    )
                } else {
                    format!("{marker}{}. {}", index + 1, stage)
                }
            } else {
                format!("{marker}{}. {}", index + 1, command.name())
            };
            let style = if index == app.pipeline_offset {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(
                Line::from(truncate_display(
                    &safe(&line),
                    area.width.saturating_sub(2) as usize,
                ))
                .style(style),
            );
        }
    }
    let title = if total == 0 {
        "Pipeline 0 steps".to_owned()
    } else {
        format!(
            "Pipeline {} steps | {}/{}",
            total,
            app.pipeline_offset + 1,
            total
        )
    };
    let mut block = Block::default().borders(Borders::ALL).title(title);
    if app.focus == Focus::Pipeline {
        block = block.border_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_keyboard_help(f: &mut Frame, screen: Rect, scroll: &mut u16) {
    let width = screen.width.saturating_sub(4).clamp(1, 76);
    let height = screen.height.saturating_sub(2).clamp(1, 18);
    let area = Rect {
        x: screen.x + screen.width.saturating_sub(width) / 2,
        y: screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let lines = [
        "Ctrl+A then ?  Scrollable keyboard help",
        "Ctrl+A then h  Expanded help for the current command",
        "Ctrl+A then t/b/i  Focus table, pipeline, or command input",
        "Ctrl+A then f  Expand or shrink the focused table/pipeline pane",
        "Table focus: arrows or h/j/k/l move selection; Enter opens cell details",
        "Ctrl+A then d  Inspect the selected row; Ctrl+A e  Expand the full error",
        "Pipeline focus: arrows or j/k move steps; Enter edits its full source group",
        "Ctrl+A then z  Cancel active work; help and pane navigation remain available",
        "Ctrl+A then o  Export full data after entering a new path (CSV or JSONL)",
        "Ctrl+A then c  Choose select columns (Space toggle, h/j/k/l move)",
        "Ctrl+A then w  Write YAML checkpoint path",
        "Ctrl+A then C  Create session checkpoint",
        "Ctrl+A then x  Clear cache",
        "Ctrl+A then p  Toggle sample/full preview",
        "Ctrl+A then u/r  Undo / redo",
        "Ctrl+A then q  Quit; Ctrl+C quits",
        "? or Esc closes keyboard help; Esc returns pane focus to command input",
        "Tab completes input; Up/Down choose completion; Alt+Left/Right scroll columns",
        "Enter commits input; :export PATH [--format csv|jsonl] explicitly exports all rows",
    ];
    f.render_widget(Clear, area);
    let wrapped = wrap_static_lines(&lines, area.width.saturating_sub(2) as usize);
    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = wrapped.len().saturating_sub(visible).min(u16::MAX as usize) as u16;
    *scroll = (*scroll).min(max_scroll);
    let paragraph = Paragraph::new(wrapped.join("\n")).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title("Keyboard help (↑/↓ PgUp/PgDn; Esc)"),
    );
    f.render_widget(paragraph.scroll((*scroll, 0)), area);
}

fn wrap_static_lines(lines: &[&str], max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut wrapped = Vec::new();
    for line in lines {
        let mut current = String::new();
        let mut width = 0;
        for c in line.chars() {
            let char_width = UnicodeWidthStr::width(c.encode_utf8(&mut [0; 4]));
            if width > 0 && width + char_width > max_width {
                wrapped.push(std::mem::take(&mut current));
                width = 0;
            }
            current.push(c);
            width += char_width;
        }
        wrapped.push(current);
    }
    wrapped
}

fn draw_headers(f: &mut Frame, area: Rect, app: &App) {
    let Some(preview) = &app.preview else {
        let mut block = Block::default().borders(Borders::ALL).title("Headers");
        if app.focus == Focus::Table {
            block = block.border_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        }
        f.render_widget(Paragraph::new("Waiting for columns…").block(block), area);
        return;
    };
    let columns = &preview.columns;
    let title = if columns.len() == preview.total_columns {
        format!("Headers ({})", preview.total_columns)
    } else {
        format!("Headers {}/{}", columns.len(), preview.total_columns)
    };
    let selected = app.columns_offset.min(columns.len().saturating_sub(1));
    let inner_height = area.height.saturating_sub(2) as usize;
    let mut lines = Vec::new();
    if columns.is_empty() {
        lines.push(ratatui::text::Line::from("No columns"));
    } else if inner_height > 0 {
        let show_above = selected > 0;
        let show_below = selected + 1 < columns.len();
        let slots = inner_height.saturating_sub(show_above as usize + show_below as usize);
        if slots == 0 {
            lines.push(ratatui::text::Line::from(truncate_display(
                &safe(&format!(
                    "› {} : {}",
                    columns[selected].0, columns[selected].1
                )),
                area.width.saturating_sub(2) as usize,
            )));
        } else {
            let mut start = selected.saturating_sub(slots.saturating_sub(1) / 2);
            start = start.min(columns.len().saturating_sub(slots));
            let end = (start + slots).min(columns.len());
            if start > 0 {
                lines.push(ratatui::text::Line::from("↑ earlier headers"));
            }
            for (i, (name, kind)) in columns.iter().enumerate().take(end).skip(start) {
                let marker = if i == selected { "› " } else { "  " };
                let label = truncate_display(
                    &safe(&format!("{marker}{name} : {kind}")),
                    area.width.saturating_sub(2) as usize,
                );
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                lines.push(ratatui::text::Line::from(label).style(style));
            }
            if end < columns.len() {
                lines.push(ratatui::text::Line::from("↓ more headers"));
            }
        }
    }
    let mut block = Block::default().borders(Borders::ALL).title(title);
    if app.focus == Focus::Table {
        block = block.border_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(lines)).block(block),
        area,
    );
}

fn truncate_display(value: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0;
    for c in value.chars() {
        let char_width = UnicodeWidthStr::width(c.encode_utf8(&mut [0; 4]));
        if width + char_width > max_width.saturating_sub(1) {
            out.push('…');
            break;
        }
        out.push(c);
        width += char_width;
    }
    out
}
fn draw_table(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(p) = &app.preview else {
        let mut block = Block::default().borders(Borders::ALL).title("Preview");
        if app.focus == Focus::Table {
            block = block.border_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        }
        f.render_widget(
            Paragraph::new("Waiting for a valid pipeline…").block(block),
            area,
        );
        return;
    };
    let cols = &p.columns;
    let width = (area.width.saturating_sub(3) as usize / 14).max(1);
    let start = app.columns_offset.min(cols.len().saturating_sub(1));
    app.columns_offset = start;
    let end = (start + width).min(cols.len());
    let visible_rows = area.height.saturating_sub(3) as usize;
    app.selected_row = app.selected_row.min(p.rows.len().saturating_sub(1));
    let max_offset = p.rows.len().saturating_sub(visible_rows.max(1));
    if matches!(app.focus, Focus::Table) {
        if app.selected_row < app.rows_offset {
            app.rows_offset = app.selected_row;
        } else if app.selected_row >= app.rows_offset.saturating_add(visible_rows.max(1)) {
            app.rows_offset = app
                .selected_row
                .saturating_add(1)
                .saturating_sub(visible_rows.max(1));
        }
    }
    app.rows_offset = app.rows_offset.min(max_offset);
    let row_end = app
        .rows_offset
        .saturating_add(visible_rows)
        .min(p.rows.len());
    let row_label = if p.rows.is_empty() {
        "0 rows".to_owned()
    } else {
        format!(
            "rows {}–{} of {}",
            app.rows_offset + 1,
            row_end,
            p.rows.len()
        )
    };
    let column_label = if start == end {
        format!("0 of {} columns", p.total_columns)
    } else {
        format!(
            "columns {}–{}; {} loaded of {}",
            start + 1,
            end,
            cols.len(),
            p.total_columns
        )
    };
    let title = format!(
        "{} | {} | {}{} | cache {} B",
        if p.draft_issue.is_some() {
            "Input before draft"
        } else {
            "Preview"
        },
        row_label,
        column_label,
        if p.has_more_rows {
            " (more rows available)"
        } else {
            ""
        },
        p.cache.as_ref().map_or(0, |c| c.bytes)
    );
    if start == end {
        let mut block = Block::default().borders(Borders::ALL).title(title);
        if app.focus == Focus::Table {
            block = block.border_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        }
        f.render_widget(Paragraph::new("No columns to display").block(block), area);
        return;
    }
    let header = cols[start..end]
        .iter()
        .map(|(n, t)| Cell::from(safe(&format!("{n} : {t}"))))
        .collect::<Vec<_>>();
    let rows = p
        .rows
        .iter()
        .enumerate()
        .take(row_end)
        .skip(app.rows_offset)
        .map(|(row_index, r)| {
            let cells = r
                .iter()
                .enumerate()
                .skip(start)
                .take(end - start)
                .map(|(column_index, value)| {
                    let cell = Cell::from(safe(value));
                    if app.focus == Focus::Table
                        && row_index == app.selected_row
                        && column_index == app.columns_offset
                    {
                        cell.style(
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        )
                    } else {
                        cell
                    }
                })
                .collect::<Vec<_>>();
            let row = Row::new(cells);
            if app.focus == Focus::Table && row_index == app.selected_row {
                row.style(Style::default().bg(Color::DarkGray))
            } else {
                row
            }
        })
        .collect::<Vec<_>>();
    let widths = vec![Constraint::Min(12); end - start];
    let mut block = Block::default().borders(Borders::ALL).title(title);
    if app.focus == Focus::Table {
        block = block.border_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(Style::default().add_modifier(Modifier::BOLD)))
        .block(block)
        .column_spacing(1);
    f.render_widget(table, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn test_app() -> App {
        App {
            worker: Worker::new(),
            committed: Vec::new(),
            draft: String::new(),
            cursor: 0,
            preview: None,
            mode: PreviewMode::Sample,
            generation: 0,
            latest_preview: 0,
            pending_commit: None,
            history: Vec::new(),
            redo: Vec::new(),
            sources: Vec::new(),
            edit_step: None,
            explicit: HashSet::new(),
            status: "Ready".into(),
            error: String::new(),
            dirty_at: None,
            columns_offset: 0,
            rows_offset: 0,
            selected_row: 0,
            focus: Focus::Input,
            fullscreen: false,
            pipeline_offset: 0,
            detail: None,
            working: None,
            cancelled: HashSet::new(),
            latest_detail: 0,
            completion: 0,
            candidates: Vec::new(),
            prefix_mode: false,
            help_overlay: false,
            help_scroll: 0,
            picker: None,
            pending_picker: false,
            preview_pending: false,
            quitting: false,
        }
    }

    #[test]
    fn unicode_cursor_and_terminal_text_sanitization() {
        let s = "a界🙂";
        assert_eq!(prev_boundary(s, s.len()), 4);
        assert_eq!(next_boundary(s, 1), 4);
        assert_eq!(safe("row\u{1b}[31m\n"), "row\\u{1b}[31m\\n");
    }

    #[test]
    fn key_release_does_not_duplicate_text_input() {
        let mut app = test_app();
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        app.key(release).unwrap();
        assert!(app.draft.is_empty());
        let press =
            KeyEvent::new_with_kind(KeyCode::Char('x'), KeyModifiers::NONE, KeyEventKind::Press);
        app.key(press).unwrap();
        assert_eq!(app.draft, "x");
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.draft, "x?");
    }

    #[test]
    fn narrow_test_backend_renders_without_panicking() {
        let mut terminal = Terminal::new(TestBackend::new(24, 8)).unwrap();
        let mut app = App {
            worker: Worker::new(),
            committed: Vec::new(),
            draft: "select 名".into(),
            cursor: "select 名".len(),
            preview: Some(Preview {
                columns: vec![("名字".into(), "String".into())],
                total_columns: 1,
                input_columns: vec![("名字".into(), "String".into())],
                rows: vec![vec!["值".into()]],
                has_more_rows: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
                draft_issue: None,
                cache: None,
                commands: Vec::new(),
                source_version: String::new(),
            }),
            mode: PreviewMode::Sample,
            generation: 0,
            latest_preview: 0,
            pending_commit: None,
            history: Vec::new(),
            redo: Vec::new(),
            sources: Vec::new(),
            edit_step: None,
            explicit: HashSet::new(),
            status: "Ready".into(),
            error: String::new(),
            dirty_at: None,
            columns_offset: 0,
            rows_offset: 0,
            selected_row: 0,
            focus: Focus::Input,
            fullscreen: false,
            pipeline_offset: 0,
            detail: None,
            working: None,
            cancelled: HashSet::new(),
            latest_detail: 0,
            completion: 0,
            candidates: Vec::new(),
            prefix_mode: false,
            help_overlay: false,
            help_scroll: 0,
            picker: None,
            pending_picker: false,
            preview_pending: false,
            quitting: false,
        };
        terminal.draw(|f| render(f, &mut app)).unwrap();
    }

    #[test]
    fn preview_update_bounds_scroll_after_result_shrinks() {
        let mut app = test_app();
        app.rows_offset = 80;
        app.latest_preview = 1;
        app.handle_event(Event {
            id: 1,
            result: Ok(Outcome::Preview(Preview {
                columns: vec![
                    ("hoge".into(), "str".into()),
                    ("count".into(), "u32".into()),
                ],
                total_columns: 2,
                input_columns: vec![("hoge".into(), "str".into())],
                rows: vec![vec!["A".into(), "60".into()], vec!["B".into(), "40".into()]],
                has_more_rows: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(100),
                draft_issue: None,
                cache: None,
                commands: Vec::new(),
                source_version: String::new(),
            })),
        });
        let mut terminal = Terminal::new(TestBackend::new(110, 18)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(app.rows_offset < 2);
        assert!(rendered.contains("40"), "{rendered}");
    }

    #[test]
    fn pipeline_focus_shows_arguments_and_edits_the_explicit_source_group_range() {
        let mut app = test_app();
        let source = "load source.csv - filter true - filter true - filter true - filter true - filter true - filter true - filter true - filter true - filter true";
        let commands = input::parse_commands(source).unwrap();
        app.committed = commands.clone();
        app.sources.push((0, commands.len(), source.into()));
        app.preview = Some(Preview {
            columns: vec![("value".into(), "Boolean".into())],
            total_columns: 1,
            input_columns: vec![("value".into(), "Boolean".into())],
            rows: vec![vec!["true".into()]],
            has_more_rows: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(100),
            draft_issue: None,
            cache: None,
            commands: commands.clone(),
            source_version: "test".into(),
        });
        app.focus = Focus::Pipeline;
        app.fullscreen = true;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("source group 1–10"), "{rendered}");
        assert!(rendered.contains("filter"), "{rendered}");
        assert_eq!(terminal.backend().buffer()[(0, 1)].fg, Color::Cyan);

        for _ in 1..commands.len() {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                .unwrap();
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.edit_step, Some((0, commands.len())));
        assert_eq!(app.draft, source);
        assert!(app.status.contains("steps 1–10"));
        assert!(app.pending_commit.is_none());
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            rendered.contains("Edit source group, steps 1–10"),
            "{rendered}"
        );

        app.draft = "filter unsaved".into();
        app.cursor = app.draft.len();
        app.focus = Focus::Table;
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.focus, Focus::Input);
        assert_eq!(app.draft, "filter unsaved");
    }

    #[test]
    fn cancelling_pending_preview_quarantines_late_commit_and_preserves_draft() {
        let mut app = test_app();
        app.committed = input::parse_commands("load existing.csv").unwrap();
        app.draft = "filter true".into();
        app.cursor = app.draft.len();
        let next = input::parse_commands("load existing.csv - filter true").unwrap();
        app.pending_commit = Some(PendingCommit {
            id: 42,
            commands: next.clone(),
            label: app.draft.clone(),
            edit: None,
        });
        app.latest_preview = 42;
        app.preview_pending = true;
        app.cancel_work();
        app.handle_event(Event {
            id: 42,
            result: Ok(Outcome::Preview(Preview {
                columns: Vec::new(),
                total_columns: 0,
                input_columns: Vec::new(),
                rows: Vec::new(),
                has_more_rows: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(100),
                draft_issue: None,
                cache: None,
                commands: next,
                source_version: "test".into(),
            })),
        });
        assert_eq!(app.committed.len(), 1);
        assert_eq!(app.draft, "filter true");
        assert!(app.pending_commit.is_none());

        let mut initial_preview = test_app();
        initial_preview.latest_preview = 77;
        initial_preview.preview_pending = true;
        initial_preview.pending_picker = true;
        initial_preview.dirty_at = Some(Instant::now());
        initial_preview.working = Some((77, TaskPhase::Loading, Instant::now()));
        initial_preview.cancel_work();
        assert_eq!(initial_preview.latest_preview, 0);
        assert!(!initial_preview.preview_pending);
        assert!(!initial_preview.pending_picker);
        assert!(initial_preview.dirty_at.is_none());
        assert!(initial_preview.working.is_none());
        initial_preview.handle_event(Event {
            id: 77,
            result: Ok(Outcome::Cancelled),
        });
        assert!(!initial_preview.preview_pending);
        assert!(initial_preview.dirty_at.is_none());
    }

    #[test]
    fn save_success_is_reported_even_when_cancel_was_requested() {
        let mut app = test_app();
        app.cancelled.insert(7);
        app.explicit.insert(7);
        app.working = Some((7, TaskPhase::Saving, Instant::now()));
        app.handle_event(Event {
            id: 7,
            result: Ok(Outcome::Saved(PathBuf::from("full.csv"))),
        });
        assert_eq!(app.status, "Saved full.csv");
        assert!(!app.cancelled.contains(&7));
    }

    #[test]
    fn expanded_error_and_keyboard_help_render_at_80x24_without_changing_draft() {
        let mut app = test_app();
        app.draft = "filter this text stays intact".into();
        app.cursor = app.draft.len();
        app.error = "A detailed error cause that can be read in the expanded view".into();
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE))
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Error details"), "{rendered}");
        assert!(rendered.contains("detailed error cause"), "{rendered}");
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.draft, "filter this text stays intact");
        app.help_overlay = true;
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Keyboard help"), "{rendered}");
        assert!(rendered.contains("full data"), "{rendered}");
    }

    #[test]
    fn headers_remain_visible_and_follow_the_selected_column() {
        let mut terminal = Terminal::new(TestBackend::new(110, 18)).unwrap();
        let mut app = test_app();
        app.columns_offset = 7;
        app.preview = Some(Preview {
            columns: (0..16)
                .map(|i| {
                    if i == 7 {
                        ("列\t八".into(), "String".into())
                    } else {
                        (format!("Header {i:02}"), "String".into())
                    }
                })
                .collect(),
            total_columns: 20,
            input_columns: vec![("列\t八".into(), "String".into())],
            rows: Vec::new(),
            has_more_rows: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
            commands: Vec::new(),
            source_version: String::new(),
        });
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(rendered.contains("Headers"));
        assert!(rendered.contains("16/20"));
        // Ratatui reserves a continuation cell after wide glyphs, so the flattened
        // buffer may contain padding between the CJK glyph and following text.
        assert!(rendered.contains("列"));
        assert!(rendered.contains("\\t八"));
        assert!(rendered.contains("↑ earlier headers"));
        assert!(rendered.contains("↓ more headers"));
        assert!(app.draft.is_empty());

        for _ in 0..30 {
            app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT))
                .unwrap();
        }
        assert_eq!(app.columns_offset, 15);
        app.key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT))
            .unwrap();
        assert_eq!(app.columns_offset, 14);

        app.latest_preview = 1;
        app.handle_event(Event {
            id: 1,
            result: Ok(Outcome::Preview(Preview {
                columns: vec![("only".into(), "String".into())],
                total_columns: 1,
                input_columns: vec![("only".into(), "String".into())],
                rows: Vec::new(),
                has_more_rows: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
                draft_issue: None,
                cache: None,
                commands: Vec::new(),
                source_version: String::new(),
            })),
        });
        assert_eq!(app.columns_offset, 0);
    }

    #[test]
    fn draft_preview_fallback_keeps_pipeline_uncommitted_and_completes_input_schema() {
        let mut app = test_app();
        app.committed = input::parse_commands("load source.csv").unwrap();
        app.draft = "select ".into();
        app.cursor = app.draft.len();
        app.latest_preview = 4;
        app.handle_event(Event {
            id: 4,
            result: Ok(Outcome::Preview(Preview {
                columns: vec![("projected".into(), "String".into())],
                total_columns: 1,
                input_columns: vec![
                    ("alpha".into(), "String".into()),
                    ("beta".into(), "Int64".into()),
                    ("hoge".into(), "String".into()),
                ],
                rows: vec![vec!["source".into()]],
                has_more_rows: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
                draft_issue: Some("invalid column; showing input before draft".into()),
                cache: None,
                commands: Vec::new(),
                source_version: String::new(),
            })),
        });
        app.draft.clear();
        app.cursor = 0;
        app.refresh_candidates();
        assert!(!app.candidates.iter().any(|c| c.display == "load"));
        app.edit_step = Some((0, 1));
        app.refresh_candidates();
        assert!(app.candidates.iter().any(|c| c.display == "load"));
        app.edit_step = None;
        app.draft = "select ".into();
        app.cursor = app.draft.len();
        app.refresh_candidates();
        assert_eq!(app.committed.len(), 1);
        assert!(app.pending_commit.is_none());
        assert!(app.status.contains("Input before draft"));
        assert!(app
            .candidates
            .iter()
            .any(|candidate| candidate.display == "alpha"));
        assert!(app
            .candidates
            .iter()
            .any(|candidate| candidate.display == "beta"));
        assert!(!app
            .candidates
            .iter()
            .any(|candidate| candidate.display == "projected"));
    }

    #[test]
    fn column_picker_cancel_preserves_draft_and_accept_rewrites_select_target() {
        let mut app = test_app();
        app.draft = "select alpha,beta".into();
        app.cursor = app.draft.len();
        app.latest_preview = 1;
        app.preview = Some(Preview {
            columns: vec![("alpha".into(), "String".into())],
            total_columns: 1,
            input_columns: vec![
                ("alpha".into(), "String".into()),
                ("beta".into(), "Int64".into()),
            ],
            rows: Vec::new(),
            has_more_rows: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
            commands: Vec::new(),
            source_version: String::new(),
        });
        let original = app.draft.clone();
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.picker.is_some());
        assert!(app.explicit.is_empty());
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.help_overlay);
        assert!(app.picker.is_some());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(app.picker.is_some());
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.draft, original);

        app.open_picker();
        app.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.draft, "select beta");
        assert!(app.dirty_at.is_some());

        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Char('C'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.explicit.len(), 1);
        assert!(app.status.contains("checkpoint"));
    }

    #[test]
    fn completion_pane_scrolls_through_commands_and_ghost_is_display_only() {
        let mut app = test_app();
        app.committed = input::parse_commands("load source.csv").unwrap();
        app.preview = Some(Preview {
            columns: vec![("alpha".into(), "String".into())],
            total_columns: 1,
            input_columns: vec![("alpha".into(), "String".into())],
            rows: Vec::new(),
            has_more_rows: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
            commands: Vec::new(),
            source_version: String::new(),
        });
        app.refresh_candidates();
        let count_index = app
            .candidates
            .iter()
            .position(|c| c.display == "count")
            .unwrap();
        app.completion = count_index;
        let original_committed = app.committed.clone();
        let mut terminal = Terminal::new(TestBackend::new(180, 36)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let input_row = 33;
        let count_ghost = &terminal.backend().buffer()[(1, input_row)];
        assert_eq!(count_ghost.symbol(), "c");
        assert_eq!(count_ghost.fg, Color::DarkGray);
        let ghost_row = (1..80)
            .map(|x| terminal.backend().buffer()[(x, input_row)].symbol())
            .collect::<String>();
        assert!(ghost_row.starts_with("count [columns]"));
        assert!(app.draft.is_empty());
        assert!(app.pending_commit.is_none());
        assert_eq!(app.committed, original_committed);
        let count_description: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(count_description.contains("Count repeated rows"));

        app.completion = app.candidates.len() - 1;
        let selected = app.candidates.last().unwrap().clone();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains(&selected.display));
        assert!(rendered.contains(&format!("Help: {}", selected.display)));
        assert!(rendered.contains(&format!("/{} ↑↓", app.candidates.len())));

        app.completion = 0;
        app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.completion, app.candidates.len() - 1);
    }

    #[test]
    fn prefix_help_and_right_panes_keep_draft_intact() {
        let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
        let mut app = test_app();
        app.draft = "select alpha".into();
        app.cursor = app.draft.len();
        app.preview = Some(Preview {
            columns: vec![("alpha".into(), "String".into())],
            total_columns: 1,
            input_columns: vec![("alpha".into(), "String".into())],
            rows: Vec::new(),
            has_more_rows: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
            commands: Vec::new(),
            source_version: String::new(),
        });
        let draft = app.draft.clone();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("Completions"));
        assert!(rendered.contains("Help"));
        assert!(rendered.contains("Pipeline"));

        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let help: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(help.contains("Keyboard help"));
        app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.help_scroll, 8);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.draft, draft);
        assert!(!app.help_overlay);

        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
            .unwrap();
        let mut narrow = Terminal::new(TestBackend::new(36, 8)).unwrap();
        narrow.draw(|f| render(f, &mut app)).unwrap();
        let scrolled_help: String = narrow
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(scrolled_help.contains("all rows"));
    }

    #[test]
    fn stale_preview_errors_do_not_replace_newer_commit_feedback() {
        let mut app = test_app();
        app.generation = 8;
        app.latest_preview = 8;
        app.draft = "select x".into();
        app.pending_commit = Some(PendingCommit {
            id: 8,
            commands: Vec::new(),
            label: "select x".into(),
            edit: None,
        });
        app.handle_event(Event {
            id: 7,
            result: Err(QuiltError::usage("stale")),
        });
        assert!(app.error.is_empty());
        assert!(app.pending_commit.is_some());
        app.handle_event(Event {
            id: 8,
            result: Err(QuiltError::usage("invalid column")),
        });
        assert!(app.error.contains("invalid column"));
        assert!(app.pending_commit.is_none());
        assert_eq!(app.draft, "select x");

        let stale_preview = || Preview {
            columns: Vec::new(),
            total_columns: 0,
            input_columns: Vec::new(),
            rows: Vec::new(),
            has_more_rows: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
            commands: Vec::new(),
            source_version: String::new(),
        };
        app.pending_commit = None;
        app.latest_preview = 0;
        app.draft = ":cache".into();
        app.status = "Built-in draft; Enter executes it".into();
        app.working = Some((1, TaskPhase::Computing, Instant::now()));
        app.handle_event(Event {
            id: 1,
            result: Ok(Outcome::Preview(stale_preview())),
        });
        assert!(app.working.is_none());
        assert_eq!(app.draft, ":cache");
        assert_eq!(app.status, "Built-in draft; Enter executes it");

        app.working = Some((2, TaskPhase::Saving, Instant::now()));
        app.handle_event(Event {
            id: 1,
            result: Ok(Outcome::Preview(stale_preview())),
        });
        assert_eq!(app.working.as_ref().map(|(id, _, _)| *id), Some(2));
    }

    #[test]
    fn quoted_cache_path_queues_an_explicit_action() {
        let mut app = test_app();
        app.draft = ":cache \"folder with spaces/data.parquet\"".into();
        app.directive();
        assert_eq!(app.explicit.len(), 1);
        assert!(app.error.is_empty());
    }

    #[test]
    fn successful_cache_and_save_events_clear_old_errors() {
        let mut app = test_app();
        app.preview = Some(Preview {
            columns: Vec::new(),
            total_columns: 0,
            input_columns: Vec::new(),
            rows: Vec::new(),
            has_more_rows: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
            commands: Vec::new(),
            source_version: String::new(),
        });
        app.error = "invalid draft".into();
        app.explicit.insert(1);
        app.handle_event(Event {
            id: 1,
            result: Ok(Outcome::Cached(super::super::CacheInfo {
                bytes: 128,
                steps: 2,
            })),
        });
        assert!(app.error.is_empty());
        assert_eq!(app.status, "Cache: 128 bytes, 2 steps");
        assert_eq!(
            app.preview.as_ref().unwrap().cache.as_ref().unwrap().bytes,
            128
        );

        app.error = "old save error".into();
        app.explicit.insert(2);
        app.handle_event(Event {
            id: 2,
            result: Ok(Outcome::Saved(PathBuf::from("checkpoint.parquet"))),
        });
        assert!(app.error.is_empty());
        assert!(app.status.contains("Saved checkpoint.parquet"));
    }

    #[test]
    fn escape_cancels_edit_and_clears_draft_error() {
        let mut app = test_app();
        app.draft = "select missing".into();
        app.cursor = app.draft.len();
        app.edit_step = Some((0, 1));
        app.error = "invalid column".into();
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(app.draft.is_empty());
        assert!(app.edit_step.is_none());
        assert!(app.error.is_empty());
        assert_eq!(app.latest_preview, 0);
    }

    #[test]
    fn undo_clears_edit_state_before_previewing_shorter_pipeline() {
        let prior = input::parse_commands("load dummy.csv").unwrap();
        let current = input::parse_commands("load dummy.csv - head 5").unwrap();
        let mut app = test_app();
        app.committed = current;
        app.sources = vec![(0, 1, "load dummy.csv".into()), (1, 1, "head 5".into())];
        app.history.push(History {
            commands: prior.clone(),
            sources: vec![(0, 1, "load dummy.csv".into())],
            label: "head 5".into(),
        });
        app.edit_step = Some((1, 1));
        app.draft = "head 10".into();
        app.cursor = app.draft.len();

        app.undo();

        assert!(app.draft.is_empty());
        assert!(app.edit_step.is_none());
        assert_eq!(app.candidate().unwrap(), prior);
    }

    #[test]
    fn write_prompt_replaces_draft_and_completion_ranges() {
        let mut app = test_app();
        app.draft = "select abcdef".into();
        app.cursor = app.draft.len();
        app.edit_step = Some((0, 1));
        app.candidates = vec![input::Completion {
            display: "abcdef".into(),
            replacement: "abcdef".into(),
            range: 7..13,
            description: "Column · String".into(),
            kind: input::CompletionKind::Column,
        }];

        app.prompt_action("write");
        app.complete();

        assert_eq!(app.draft, ":write ");
        assert!(app.edit_step.is_none());
        assert!(app.candidates.is_empty());
    }
}
