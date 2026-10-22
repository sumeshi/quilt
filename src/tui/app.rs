use super::{input, worker::Worker, Action, Event, Outcome, Preview, PreviewMode, Request};
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
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
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
            .draw(|f| render(f, &app))
            .map_err(|e| QuiltError::operation("TUI render", e.to_string()))?;
        if event::poll(Duration::from_millis(40))
            .map_err(|e| QuiltError::operation("TUI input", e.to_string()))?
        {
            match event::read().map_err(|e| QuiltError::operation("TUI input", e.to_string()))? {
                CEvent::Key(key) => app.key(key)?,
                CEvent::Paste(text) => {
                    if app.pending_commit.is_none() {
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
        self.error.clear();
        self.completion = 0;
        self.refresh_candidates();
    }
    fn refresh_candidates(&mut self) {
        self.candidates = input::complete(
            &self.draft,
            self.cursor,
            self.preview.as_ref().map_or(&[], |p| p.columns.as_slice()),
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
        let is_preview = matches!(&action, Action::Preview(_));
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
    fn schedule_preview(&mut self) {
        self.dirty_at = None;
        if self.pending_commit.is_some() {
            return;
        }
        match self.candidate() {
            Ok(commands) => {
                if let Err(e) = self.queue(commands, Action::Preview(self.mode)) {
                    self.error = e.to_string();
                } else {
                    self.latest_preview = self.generation;
                }
            }
            Err(_) => {
                self.status = "Draft incomplete; keeping last preview".into();
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
                self.preview = Some(p);
                self.error.clear();
                self.status = "Preview updated".into();
                self.refresh_candidates();
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
        if self.pending_commit.is_some()
            && !(k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c'))
            && !(k.code == KeyCode::Enter && self.draft.trim() == ":quit")
        {
            self.status = "Waiting for command validation…".into();
            return Ok(());
        }
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            match k.code {
                KeyCode::Char('c') => self.quitting = true,
                KeyCode::Char('s') => self.prompt_action("write"),
                KeyCode::Char('z') => self.undo(),
                KeyCode::Char('y') => self.redo(),
                _ => {}
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
                self.cursor = prev_boundary(&self.draft, self.cursor);
                self.refresh_candidates();
            }
            KeyCode::Right => {
                self.cursor = next_boundary(&self.draft, self.cursor);
                self.refresh_candidates();
            }
            KeyCode::Home => {
                self.cursor = 0;
                self.refresh_candidates();
            }
            KeyCode::End => {
                self.cursor = self.draft.len();
                self.refresh_candidates();
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
            KeyCode::F(5) => {
                self.mode = if self.mode == PreviewMode::Sample {
                    PreviewMode::Full
                } else {
                    PreviewMode::Sample
                };
                self.schedule_preview()
            }
            KeyCode::F(6) => {
                let c = self.committed.clone();
                let _ = self.queue(c, Action::Cache);
            }
            _ => {}
        }
        Ok(())
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
        self.completion = 0;
        self.error.clear();
        self.refresh_candidates();
    }
    fn preview_committed(&mut self) {
        if self.committed.is_empty() {
            self.preview = None;
            self.latest_preview = 0;
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

fn render(f: &mut Frame, app: &App) {
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
    let banner=format!(" QUILT  |  {} preview  |  F5 toggle sample/full  F6 cache  Ctrl-Z/Y undo/redo  Ctrl-C quit",if shown_mode==PreviewMode::Sample{format!("Sample first {} rows",app.preview.as_ref().and_then(|p|p.sample_rows).unwrap_or(super::SAMPLE_ROWS))}else{"Full data".into()});
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
    let narrow = context_area.height < 12;
    let step_limit = if narrow { 0 } else { 4 };
    let candidate_limit = if narrow { 2 } else { 5 };
    let candidate_start = app
        .completion
        .saturating_sub(candidate_limit / 2)
        .min(app.candidates.len().saturating_sub(candidate_limit));
    let steps = app
        .committed
        .iter()
        .enumerate()
        .take(step_limit)
        .map(|(i, c)| format!("{}. {}", i + 1, c.name()));
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
    let help_width = context_area.width.saturating_sub(4).max(8) as usize;
    let help_lines = help
        .lines()
        .filter(|line| !line.trim().is_empty())
        .take(if narrow { 0 } else { 2 })
        .map(|line| truncate_display(&safe(line), help_width));
    let mut right_lines = vec!["Completions".to_owned()];
    right_lines.extend(candidates);
    if !narrow {
        right_lines.push("Steps".into());
        right_lines.extend(steps);
        right_lines.push("Context".into());
        right_lines.extend(help_lines);
    }
    let right = right_lines.join("\n");
    f.render_widget(
        Paragraph::new(right)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Context / pipeline"),
            )
            .wrap(Wrap { trim: false }),
        context_area,
    );
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
        "Preview | {} rows shown{} | cache {} B",
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
    }

    #[test]
    fn narrow_test_backend_renders_without_panicking() {
        let mut terminal = Terminal::new(TestBackend::new(24, 8)).unwrap();
        let app = App {
            worker: Worker::new(),
            committed: Vec::new(),
            draft: "select 名".into(),
            cursor: "select 名".len(),
            preview: Some(Preview {
                columns: vec![("名字".into(), "String".into())],
                total_columns: 1,
                rows: vec![vec!["值".into()]],
                truncated: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
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
            quitting: false,
        };
        terminal.draw(|f| render(f, &app)).unwrap();
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
            rows: Vec::new(),
            truncated: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
            cache: None,
        });
        terminal.draw(|f| render(f, &app)).unwrap();
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
                rows: Vec::new(),
                truncated: false,
                mode: PreviewMode::Sample,
                sample_rows: Some(10_000),
                cache: None,
            })),
        });
        assert_eq!(app.columns_offset, 0);
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
            rows: Vec::new(),
            truncated: false,
            mode: PreviewMode::Sample,
            sample_rows: Some(10_000),
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
