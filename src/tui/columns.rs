use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use std::collections::HashSet;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) enum PickerResult {
    Continue,
    Accept(Vec<String>),
    Cancel,
}

pub(super) struct ColumnPicker {
    columns: Vec<(String, String)>,
    selected: Vec<String>,
    selected_set: HashSet<String>,
    filtered: Vec<usize>,
    focus: usize,
    scroll_row: usize,
    grid_across: usize,
    viewport_rows: usize,
    filter: String,
    searching: bool,
    message: String,
}

impl ColumnPicker {
    pub(super) fn new(columns: Vec<(String, String)>, selected: Vec<String>) -> Self {
        let names: HashSet<&str> = columns.iter().map(|(name, _)| name.as_str()).collect();
        let mut seen = HashSet::new();
        let selected: Vec<String> = selected
            .into_iter()
            .filter(|name| names.contains(name.as_str()) && seen.insert(name.clone()))
            .collect();
        let selected_set = selected.iter().cloned().collect();
        let filtered = (0..columns.len()).collect();
        Self {
            columns,
            selected,
            selected_set,
            filtered,
            focus: 0,
            scroll_row: 0,
            grid_across: 1,
            viewport_rows: 1,
            filter: String::new(),
            searching: false,
            message: String::new(),
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> PickerResult {
        if key.code == KeyCode::Esc {
            return PickerResult::Cancel;
        }
        if self.searching {
            match key.code {
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.refilter();
                }
                KeyCode::Left => self.move_focus(-1, 0),
                KeyCode::Right => self.move_focus(1, 0),
                KeyCode::Up => self.move_focus(0, -1),
                KeyCode::Down => self.move_focus(0, 1),
                KeyCode::PageUp => self.move_focus(0, -(self.viewport_rows as isize)),
                KeyCode::PageDown => self.move_focus(0, self.viewport_rows as isize),
                KeyCode::Home => self.focus = 0,
                KeyCode::End => self.focus = self.filtered.len().saturating_sub(1),
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.filter.push(c);
                    self.refilter();
                }
                _ => {}
            }
            return PickerResult::Continue;
        }

        self.message.clear();
        match key.code {
            KeyCode::Enter => {
                if self.selected.is_empty() {
                    self.message = "Select at least one column to apply".into();
                    PickerResult::Continue
                } else {
                    PickerResult::Accept(self.selected.clone())
                }
            }
            KeyCode::Char('/') => {
                self.searching = true;
                PickerResult::Continue
            }
            KeyCode::Char(' ') => {
                self.toggle_focus();
                PickerResult::Continue
            }
            KeyCode::Char('a') => {
                for &index in &self.filtered {
                    let name = self.columns[index].0.clone();
                    if self.selected_set.insert(name.clone()) {
                        self.selected.push(name);
                    }
                }
                PickerResult::Continue
            }
            KeyCode::Char('u') => {
                let clear: HashSet<String> = self
                    .filtered
                    .iter()
                    .map(|&i| self.columns[i].0.clone())
                    .collect();
                self.selected.retain(|name| !clear.contains(name));
                self.selected_set.retain(|name| !clear.contains(name));
                PickerResult::Continue
            }
            KeyCode::Char('i') => {
                for &index in &self.filtered {
                    let name = self.columns[index].0.clone();
                    if self.selected_set.remove(&name) {
                        self.selected.retain(|item| item != &name);
                    } else {
                        self.selected_set.insert(name.clone());
                        self.selected.push(name);
                    }
                }
                PickerResult::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_focus(0, -1);
                PickerResult::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_focus(0, 1);
                PickerResult::Continue
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.move_focus(-1, 0);
                PickerResult::Continue
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.move_focus(1, 0);
                PickerResult::Continue
            }
            KeyCode::PageUp => {
                self.move_focus(0, -(self.viewport_rows as isize));
                PickerResult::Continue
            }
            KeyCode::PageDown => {
                self.move_focus(0, self.viewport_rows as isize);
                PickerResult::Continue
            }
            KeyCode::Home => {
                self.focus = 0;
                PickerResult::Continue
            }
            KeyCode::End => {
                self.focus = self.filtered.len().saturating_sub(1);
                PickerResult::Continue
            }
            _ => PickerResult::Continue,
        }
    }

    pub(super) fn render(&mut self, frame: &mut Frame) {
        let screen = frame.area();
        let width = (screen.width.saturating_mul(9) / 10)
            .max(1)
            .min(screen.width);
        let height = (screen.height.saturating_mul(9) / 10)
            .max(1)
            .min(screen.height);
        let area = Rect {
            x: screen.x + screen.width.saturating_sub(width) / 2,
            y: screen.y + screen.height.saturating_sub(height) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, area);
        let title = " Choose columns ";
        frame.render_widget(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
            area,
        );
        if area.width < 3 || area.height < 3 {
            return;
        }
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width - 2,
            height: area.height - 2,
        };
        let focused_index = self.filtered.get(self.focus).copied();
        let footer_lines = if inner.height >= 6 {
            4
        } else {
            inner.height.min(2)
        };
        let header_lines = 1;
        let grid_height = inner.height.saturating_sub(footer_lines + header_lines);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_lines),
                Constraint::Length(grid_height),
                Constraint::Length(footer_lines),
            ])
            .split(inner);

        let header = if self.searching {
            format!(
                " /{}  (Enter: finish search, Esc: cancel)",
                safe_text(&self.filter)
            )
        } else {
            format!(
                " {} columns · {} selected · / search",
                self.columns.len(),
                self.selected.len()
            )
        };
        frame.render_widget(Paragraph::new(header), chunks[0]);

        let grid = chunks[1];
        if grid.height > 0 && grid.width > 0 {
            let cell_width = (grid.width / 3).clamp(8, 30);
            let across = (grid.width / cell_width).max(1) as usize;
            self.grid_across = across;
            let rows = self.filtered.len().div_ceil(across);
            let visible_rows = grid.height as usize;
            self.viewport_rows = visible_rows.max(1);
            if self.focus < self.filtered.len() {
                let row = self.focus / across;
                if row < self.scroll_row {
                    self.scroll_row = row;
                } else if row >= self.scroll_row + visible_rows {
                    self.scroll_row = row + 1 - visible_rows;
                }
            }
            self.scroll_row = self.scroll_row.min(rows.saturating_sub(visible_rows));
            let shown_start = self.scroll_row * across;
            let shown_end = (shown_start + visible_rows * across).min(self.filtered.len());
            let row_count = shown_end.saturating_sub(shown_start).div_ceil(across);
            for row in 0..row_count {
                let row_start = shown_start + row * across;
                let row_end = (row_start + across).min(shown_end);
                for pos in row_start..row_end {
                    let column = self.filtered[pos];
                    let (name, _) = &self.columns[column];
                    let marker = if self.selected_set.contains(name) {
                        "[x] "
                    } else {
                        "[ ] "
                    };
                    let label =
                        fit_text(&format!("{marker}{}", safe_text(name)), cell_width as usize);
                    let x_offset = (pos - row_start) as u16 * cell_width;
                    let cell = Rect {
                        x: grid.x + x_offset,
                        y: grid.y + row as u16,
                        width: cell_width.min(grid.width.saturating_sub(x_offset)),
                        height: 1,
                    };
                    let style = if pos == self.focus {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if self.selected_set.contains(name) {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default()
                    };
                    frame.render_widget(Paragraph::new(label).style(style), cell);
                }
            }
        }

        if chunks[2].height > 0 && chunks[2].width > 0 {
            let mut lines = Vec::new();
            if let Some(index) = focused_index {
                let (name, kind) = &self.columns[index];
                let info = format!("Focused: {}  ·  {}", safe_text(name), safe_text(kind));
                lines.extend(wrap_bounded(&info, chunks[2].width as usize, 2));
            }
            let range = if self.filtered.is_empty() || chunks[1].height == 0 {
                "0–0".to_owned()
            } else {
                let visible_rows = chunks[1].height as usize;
                let start = self.scroll_row * self.grid_across + 1;
                let end = (start - 1 + visible_rows * self.grid_across).min(self.filtered.len());
                format!("{start}–{end}")
            };
            let shown = format!(
                "Visible columns {range} / {}  ·  selected {}",
                self.filtered.len(),
                self.selected.len()
            );
            lines.push(shown);
            let help = if self.searching {
                "Arrows move · PgUp/PgDn · Home/End · Space types · Enter ends search · Esc cancels"
                    .to_owned()
            } else {
                "Space toggle · hjkl/arrows · PgUp/PgDn · Home/End · a all · i invert · u clear · Enter apply · Esc cancel".to_owned()
            };
            lines.push(if self.message.is_empty() {
                help
            } else {
                self.message.clone()
            });
            let bounded = lines
                .into_iter()
                .take(chunks[2].height as usize)
                .map(|line| Line::from(fit_text(&line, chunks[2].width as usize)))
                .collect::<Vec<_>>();
            frame.render_widget(Paragraph::new(bounded), chunks[2]);
        }
    }

    fn toggle_focus(&mut self) {
        if let Some(&index) = self.filtered.get(self.focus) {
            let name = self.columns[index].0.clone();
            if self.selected_set.remove(&name) {
                self.selected.retain(|item| item != &name);
            } else {
                self.selected_set.insert(name.clone());
                self.selected.push(name);
            }
        }
    }

    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.filtered = self
            .columns
            .iter()
            .enumerate()
            .filter(|(_, (name, kind))| {
                name.to_lowercase().contains(&needle) || kind.to_lowercase().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect();
        self.focus = 0;
        self.scroll_row = 0;
    }

    fn move_focus(&mut self, dx: isize, dy: isize) {
        let len = self.filtered.len();
        if len == 0 {
            self.focus = 0;
            return;
        }
        let stride = self.grid_across as isize;
        let next = (self.focus as isize + dx + dy * stride).clamp(0, len as isize - 1);
        self.focus = next as usize;
    }
}

fn safe_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn fit_text(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let target = width.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > target {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

fn wrap_bounded(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > width {
            lines.push(line);
            line = String::new();
            used = 0;
            if lines.len() == max_lines {
                let last = lines.last_mut().unwrap();
                *last = mark_truncated(last, width);
                return lines;
            }
            continue;
        }
        line.push(c);
        used += cw;
        index += 1;
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        let last = lines.last_mut().unwrap();
        *last = mark_truncated(last, width);
    }
    lines
}

fn mark_truncated(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let target = width.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > target {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    #[test]
    fn long_grid_navigation_and_filter_keep_hidden_selection() {
        let columns: Vec<(String, String)> = (0..240)
            .map(|i| (format!("column-{i:03}"), "String".to_owned()))
            .collect();
        let mut picker = ColumnPicker::new(columns.clone(), vec!["column-000".into()]);
        for _ in 0..150 {
            picker.handle_key(key(KeyCode::Down));
        }
        picker.handle_key(key(KeyCode::Char(' ')));
        assert!(picker.selected_set.contains("column-150"));
        picker.handle_key(key(KeyCode::Char('/')));
        for c in "column-239".chars() {
            picker.handle_key(key(KeyCode::Char(c)));
        }
        picker.handle_key(key(KeyCode::Enter));
        assert_eq!(picker.filtered.len(), 1);
        assert_eq!(picker.columns[picker.filtered[0]].0, "column-239");
        picker.handle_key(key(KeyCode::Char(' ')));
        match picker.handle_key(key(KeyCode::Enter)) {
            PickerResult::Accept(selected) => {
                assert_eq!(selected, ["column-000", "column-150", "column-239"])
            }
            _ => panic!("expected accepted column selection"),
        }

        let mut filtered = ColumnPicker::new(columns, vec![]);
        filtered.handle_key(key(KeyCode::Char('/')));
        for c in "column-1".chars() {
            filtered.handle_key(key(KeyCode::Char(c)));
        }
        filtered.handle_key(key(KeyCode::Enter));
        assert_eq!(
            filtered.columns[filtered.filtered[filtered.focus]].0,
            "column-100"
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| filtered.render(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut focus_is_rendered = false;
        for y in 0..20 {
            let row: String = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
            if row.contains("column-100") {
                focus_is_rendered = (0..100).any(|x| {
                    let cell = &buffer[(x, y)];
                    cell.bg == Color::Cyan
                });
                break;
            }
        }
        assert!(
            focus_is_rendered,
            "first filtered result must be visible and focused"
        );
        assert_eq!(filtered.grid_across, 3);
        filtered.handle_key(key(KeyCode::PageDown));
        assert!(
            filtered.focus > 0,
            "PageDown should advance by the visible grid page"
        );
        filtered.handle_key(key(KeyCode::End));
        assert_eq!(filtered.focus, filtered.filtered.len() - 1);
        filtered.handle_key(key(KeyCode::Home));
        assert_eq!(filtered.focus, 0);
    }

    #[test]
    fn unicode_control_names_and_tiny_terminal_render_safely() {
        let mut picker = ColumnPicker::new(
            vec![(
                "名\u{202e}🙂\nvery-long-column-name".into(),
                "型\u{1b}[31m".into(),
            )],
            vec![],
        );
        let mut terminal = Terminal::new(TestBackend::new(2, 2)).unwrap();
        terminal.draw(|frame| picker.render(frame)).unwrap();
        assert_eq!(safe_text("a\u{1b}\u{202e}b"), "a\\u{1b}\\u{202e}b");
        assert!(fit_text("名字🙂long", 5).ends_with('…'));
        assert!(wrap_bounded("12345678901234567890", 5, 2)[1].ends_with('…'));
        match picker.handle_key(key(KeyCode::Enter)) {
            PickerResult::Continue => assert!(!picker.message.is_empty()),
            _ => panic!("empty selection should remain in the picker"),
        }
    }
}
