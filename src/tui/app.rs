use super::{
    columns::{ColumnPicker, PickerResult},
    input,
    worker::Worker,
    Action, Event, Outcome, Preview, PreviewMode, Request,
};
use crate::controllers::command_model::{CommandCategory, TypedCommand};
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
                        && !app.help_overlay
                        && !app.prefix_mode
                    {
                        app.insert(&text);
                        app.changed();
                    } else {
                        app.status = "Waiting for command validation…".into();
                    }
                }
                CEvent::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        app.rows_offset = app.rows_offset.saturating_add(3)
                    }
                    MouseEventKind::ScrollUp => app.rows_offset = app.rows_offset.saturating_sub(3),
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
        match result {
            Ok(Outcome::Preview(p)) => {
                if id != self.latest_preview
                    && self
                        .pending_commit
                        .as_ref()
                        .is_none_or(|pending| pending.id != id)
                {
                    return;
                }
                self.columns_offset = self.columns_offset.min(p.columns.len().saturating_sub(1));
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
            Ok(Outcome::Cached(c)) => {
                if self.explicit.remove(&id) {
                    if let Some(preview) = &mut self.preview {
                        preview.cache = Some(c.clone());
                    }
                    self.status = format!("Cache: {} bytes, {} steps", c.bytes, c.steps);
                    self.error.clear();
                }
            }
            Ok(Outcome::CacheCleared) => {
                if self.explicit.remove(&id) {
                    if let Some(preview) = &mut self.preview {
                        preview.cache = None;
                    }
                    self.status = "Cache cleared".into();
                    self.error.clear();
                }
            }
            Ok(Outcome::Saved(path)) => {
                if self.explicit.remove(&id) {
                    self.status = format!("Saved {}", safe(&path.to_string_lossy()));
                    self.error.clear();
                }
            }
            Err(e) => {
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
            ":cache" if args.is_empty()=>self.queue(self.committed.clone(),Action::Cache).map(|_|()),
            ":cache" if args.len()==1=>self.queue(self.committed.clone(),Action::SaveCache(PathBuf::from(&args[0]))).map(|_|()),
            ":clear-cache" if args.is_empty()=>self.queue(self.committed.clone(),Action::ClearCache).map(|_|()),
            ":edit" if args.len()==1=>{let n=args[0].parse::<usize>().ok().filter(|n|*n>0).ok_or_else(||QuiltError::usage(":edit requires a 1-based step number"));match n{Err(e)=>Err(e),Ok(n)=>if let Some((start,len,text))=self.sources.iter().find(|(s,l,_)|n>*s&&n<=*s+*l).cloned(){self.edit_step=Some((start,len));self.draft=text;self.cursor=self.draft.len();self.status=format!("Editing step {n}; Enter validates the replacement");self.changed();Ok(())}else{Err(QuiltError::usage("Original text is available only for commands entered in this TUI session"))}}},
            _=>Err(QuiltError::usage("Use :write PATH, :cache [PATH], :clear-cache, :undo, :redo, :edit STEP, or :quit (quote paths with spaces)")),
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
        if self.pending_commit.is_some()
            && !(k.code == KeyCode::Enter && self.draft.trim() == ":quit")
        {
            self.status = "Waiting for command validation…".into();
            return Ok(());
        }
        if self.prefix_mode {
            self.prefix_mode = false;
            match k.code {
                KeyCode::Esc => self.status = "Prefix cancelled".into(),
                KeyCode::Char('?') => {
                    self.help_overlay = true;
                    self.help_scroll = 0;
                }
                KeyCode::Char('s') => self.open_picker(),
                KeyCode::Char('w') => self.prompt_action("write"),
                KeyCode::Char('c') => {
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
                self.status = "Prefix: ? help s columns w write c checkpoint x clear p mode u/r undo/redo q quit".into();
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
                    self.completion = self.completion.saturating_sub(1) % self.candidates.len();
                }
            }
            KeyCode::Down => {
                if !self.candidates.is_empty() {
                    self.completion = (self.completion + 1) % self.candidates.len();
                }
            }
            KeyCode::PageUp => self.rows_offset = self.rows_offset.saturating_sub(20),
            KeyCode::PageDown => self.rows_offset += 20,
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
        " QUILT | {} preview{} | Ctrl+A ? help | Ctrl+A s columns | Tab complete | Alt←/→ columns | Enter apply",
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
        Paragraph::new(banner).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        chunks[0],
    );
    draw_headers(f, headers_area, app);
    draw_table(f, table_area, app);
    let help = input::help(&app.draft, app.cursor);
    let candidate_limit = 5;
    let candidate_start = app
        .completion
        .saturating_sub(candidate_limit / 2)
        .min(app.candidates.len().saturating_sub(candidate_limit));
    let candidates = app
        .candidates
        .iter()
        .enumerate()
        .skip(candidate_start)
        .take(candidate_limit)
        .map(|(i, c)| {
            format!(
                "{} {}",
                if i == app.completion { "›" } else { " " },
                safe(&c.display)
            )
        });
    let (completion_area, help_area, pipeline_area) = if context_area.height >= 25 {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7),
                Constraint::Min(8),
                Constraint::Length(7),
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
    let mut completion_lines = vec!["Candidates".to_owned()];
    completion_lines.extend(candidates);
    if app.candidates.is_empty() {
        completion_lines.push("No completions".into());
    }
    f.render_widget(
        Paragraph::new(completion_lines.join("\n"))
            .block(Block::default().borders(Borders::ALL).title("Completions"))
            .wrap(Wrap { trim: false }),
        completion_area,
    );
    f.render_widget(
        Paragraph::new(
            help.lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .block(Block::default().borders(Borders::ALL).title(
            if help.lines().count() > help_area.height.saturating_sub(2) as usize {
                "Help ↓"
            } else {
                "Help"
            },
        ))
        .wrap(Wrap { trim: false }),
        help_area,
    );
    if let Some(pipeline_area) = pipeline_area {
        let steps = app
            .committed
            .iter()
            .enumerate()
            .take(4)
            .map(|(i, c)| format!("{}. {}", i + 1, c.name()))
            .collect::<Vec<_>>();
        f.render_widget(
            Paragraph::new(if steps.is_empty() {
                "No committed steps".into()
            } else {
                steps.join("\n")
            })
            .block(Block::default().borders(Borders::ALL).title("Pipeline"))
            .wrap(Wrap { trim: false }),
            pipeline_area,
        );
    }
    let cursor_col = UnicodeWidthStr::width(&app.draft[..app.cursor]);
    let input_width = chunks[2].width.saturating_sub(2) as usize;
    let input_scroll = cursor_col.saturating_sub(input_width.saturating_sub(1));
    let input = Paragraph::new(safe(&app.draft))
        .scroll((0, input_scroll.min(u16::MAX as usize) as u16))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Command (Tab complete, Enter commit)"),
        );
    f.render_widget(input, chunks[2]);
    let status = if app.error.is_empty() {
        app.status.as_str()
    } else {
        app.error.as_str()
    };
    f.render_widget(
        Paragraph::new(safe(status)).style(Style::default().fg(if app.error.is_empty() {
            Color::Gray
        } else {
            Color::Red
        })),
        chunks[3],
    );
    if chunks[2].width > 2 && chunks[2].height > 1 {
        let x = chunks[2].x + 1 + cursor_col.saturating_sub(input_scroll) as u16;
        f.set_cursor_position((
            x.min(chunks[2].right().saturating_sub(1)),
            (chunks[2].y + 1).min(chunks[2].bottom().saturating_sub(1)),
        ));
    }
    if app.help_overlay {
        draw_keyboard_help(f, area, &mut app.help_scroll);
    } else if let Some(picker) = &mut app.picker {
        picker.render(f);
    }
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
        "Ctrl+A then ?  Full keyboard help",
        "Ctrl+A then s  Choose select columns (Space toggle, h/j/k/l move)",
        "Ctrl+A then w  Write YAML checkpoint path",
        "Ctrl+A then c  Create session checkpoint",
        "Ctrl+A then x  Clear cache",
        "Ctrl+A then p  Toggle sample/full preview",
        "Ctrl+A then u/r  Undo / redo",
        "Ctrl+A then q  Quit; Ctrl+C quits",
        "? or Esc closes this help; Esc cancels prefix/modal",
        "Tab complete; Up/Down choose completion; Alt+Left/Right scroll columns",
        "Enter commits the draft; colon commands remain available",
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
        f.render_widget(
            Paragraph::new("Waiting for columns…")
                .block(Block::default().borders(Borders::ALL).title("Headers")),
            area,
        );
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
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(lines))
            .block(Block::default().borders(Borders::ALL).title(title)),
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
fn draw_table(f: &mut Frame, area: Rect, app: &App) {
    let Some(p) = &app.preview else {
        f.render_widget(
            Paragraph::new("Waiting for a valid pipeline…")
                .block(Block::default().borders(Borders::ALL).title("Preview")),
            area,
        );
        return;
    };
    let cols = &p.columns;
    let width = (area.width as usize / 14).max(1);
    let start = app.columns_offset.min(cols.len().saturating_sub(1));
    let end = (start + width).min(cols.len());
    let title = format!(
        "{} | {} rows shown{} | cache {} B",
        if p.draft_issue.is_some() {
            "Input before draft"
        } else {
            "Preview"
        },
        p.rows.len(),
        if p.truncated { "+" } else { "" },
        p.cache.as_ref().map_or(0, |c| c.bytes)
    );
    if start == end {
        f.render_widget(
            Paragraph::new("No columns to display")
                .block(Block::default().borders(Borders::ALL).title(title)),
            area,
        );
        return;
    }
    let header = cols[start..end]
        .iter()
        .map(|(n, t)| Cell::from(safe(&format!("{n} : {t}"))))
        .collect::<Vec<_>>();
    let rows = p
        .rows
        .iter()
        .skip(app.rows_offset)
        .map(|r| {
            Row::new(
                r.iter()
                    .skip(start)
                    .take(end - start)
                    .map(|s| Cell::from(safe(s)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let widths = vec![Constraint::Min(12); end - start];
    let table = Table::new(rows, widths)
        .header(Row::new(header).style(Style::default().add_modifier(Modifier::BOLD)))
        .block(Block::default().borders(Borders::ALL).title(title))
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
                truncated: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
                draft_issue: None,
                cache: None,
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
            truncated: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
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
                truncated: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
                draft_issue: None,
                cache: None,
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
                truncated: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
                draft_issue: Some("invalid column; showing input before draft".into()),
                cache: None,
            })),
        });
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
            truncated: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
        });
        let original = app.draft.clone();
        app.open_picker();
        assert!(app.picker.is_some());
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
            truncated: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
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
        assert!(scrolled_help.contains("available"));
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
            truncated: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            draft_issue: None,
            cache: None,
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
        }];

        app.prompt_action("write");
        app.complete();

        assert_eq!(app.draft, ":write ");
        assert!(app.edit_step.is_none());
        assert!(app.candidates.is_empty());
    }
}
