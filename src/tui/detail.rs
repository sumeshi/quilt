use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthChar;

const MAX_DETAIL_BYTES: usize = 256 * 1024;
const MAX_TITLE_BYTES: usize = 2 * 1024;
const PAGE_STEP: usize = 8;

/// A bounded, scrollable view for long cell values and other detail text.
pub(super) struct DetailView {
    title: String,
    text: String,
    scroll: usize,
    horizontal: usize,
    wrap: bool,
}

impl DetailView {
    pub(super) fn new(title: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut bounded = String::with_capacity(text.len().min(MAX_DETAIL_BYTES) + 96);
        let clipped = text.len() > MAX_DETAIL_BYTES;
        let end = if clipped {
            let mut end = MAX_DETAIL_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            end
        } else {
            text.len()
        };
        bounded.push_str(&text[..end]);
        if clipped {
            bounded.push_str("\n\n[Detail text truncated at 256 KiB]");
        }
        // Avoid terminal control and directional formatting characters even in titles.
        let source_title = title.into();
        let title_end = if source_title.len() > MAX_TITLE_BYTES {
            let mut end = MAX_TITLE_BYTES;
            while !source_title.is_char_boundary(end) {
                end -= 1;
            }
            end
        } else {
            source_title.len()
        };
        let mut bounded_title = source_title[..title_end].to_owned();
        if title_end < source_title.len() {
            bounded_title.push('…');
        }
        let title = visible_text(&bounded_title, false);
        Self {
            title,
            text: visible_text(&bounded, true),
            scroll: 0,
            horizontal: 0,
            wrap: true,
        }
    }

    /// Returns true when the caller should close this view.
    pub(super) fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return true,
            KeyCode::Char('j') | KeyCode::Down => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(PAGE_STEP),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(PAGE_STEP),
            KeyCode::Home => {
                self.scroll = 0;
                self.horizontal = 0;
            }
            KeyCode::End => self.scroll = usize::MAX,
            KeyCode::Char('w') => {
                self.wrap = !self.wrap;
                self.scroll = 0;
                self.horizontal = 0;
            }
            KeyCode::Char('h') | KeyCode::Left if !self.wrap => {
                self.horizontal = self.horizontal.saturating_sub(4)
            }
            KeyCode::Char('l') | KeyCode::Right if !self.wrap => {
                self.horizontal = self.horizontal.saturating_add(4)
            }
            _ => {}
        }
        false
    }

    pub(super) fn render(&mut self, frame: &mut Frame, frame_area: Rect) {
        let area = overlay_area(frame_area);
        frame.render_widget(Clear, area);
        if area.width == 0 || area.height == 0 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(format!(" {} ", self.title));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // The last inner row carries the controls. On a very short terminal, the
        // content gets the available row and the hint is omitted.
        let show_hint = inner.height >= 2;
        let content_height = inner.height.saturating_sub(if show_hint { 1 } else { 0 });
        let content = Rect {
            height: content_height,
            ..inner
        };
        let hint_area = Rect {
            y: inner.y.saturating_add(content_height),
            height: inner.height.saturating_sub(content_height),
            ..inner
        };

        let width = content.width.max(1) as usize;
        let viewport = content.height as usize;
        let total_lines = count_lines(&self.text, width, self.wrap);
        let max_scroll = total_lines.saturating_sub(viewport);
        self.scroll = self.scroll.min(max_scroll);
        let mut visible = Vec::with_capacity(viewport.min(128));
        let start = self.scroll;
        let end = start.saturating_add(viewport);
        visit_lines(&self.text, width, self.wrap, |index, line| {
            if index >= start && index < end {
                visible.push(line.to_owned());
            }
        });
        let lines = visible.into_iter().map(Line::from).collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .scroll((0, self.horizontal.min(u16::MAX as usize) as u16)),
            content,
        );

        if show_hint {
            let hint = if self.wrap {
                "↑/↓ or j/k scroll  PgUp/PgDn page  Home/End  w nowrap  Esc close"
            } else {
                "↑/↓ or j/k scroll  PgUp/PgDn page  ←/→ or h/l  w wrap  Esc close"
            };
            frame.render_widget(
                Paragraph::new(hint)
                    .style(Style::default().fg(Color::Gray).add_modifier(Modifier::DIM)),
                hint_area,
            );
        }
    }
}

fn overlay_area(area: Rect) -> Rect {
    if area.width < 24 || area.height < 7 {
        return area;
    }
    let width = area.width.saturating_sub(4).min(120);
    let height = area.height.saturating_sub(2).min(36);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn count_lines(text: &str, width: usize, wrap: bool) -> usize {
    let mut count = 0;
    visit_lines(text, width, wrap, |_, _| count += 1);
    count
}

/// Visit visual rows without retaining the complete wrapped document. At most
/// the caller-selected viewport is collected by the renderer.
fn visit_lines(text: &str, width: usize, wrap: bool, mut visit: impl FnMut(usize, &str)) {
    let mut index = 0usize;
    let width = width.max(1);
    for source_line in text.split('\n') {
        if !wrap {
            visit(index, source_line);
            index += 1;
            continue;
        }
        let mut row = String::new();
        let mut row_width = 0usize;
        for ch in source_line.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if row_width > 0 && row_width.saturating_add(ch_width) > width {
                visit(index, &row);
                index += 1;
                row.clear();
                row_width = 0;
            }
            row.push(ch);
            row_width = row_width.saturating_add(ch_width);
        }
        visit(index, &row);
        index += 1;
    }
}

fn visible_text(input: &str, preserve_newline: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' if preserve_newline => out.push('\n'),
            '\t' if preserve_newline => out.push_str("    "),
            _ if is_bidi_control(ch) || ch.is_control() => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{{{:04X}}}", ch as u32);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch as u32,
        0x061C | 0x200E..=0x200F | 0x202A..=0x202E | 0x2066..=0x2069
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{backend::TestBackend, Terminal};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn escapes_terminal_controls_and_preserves_unicode_and_lines() {
        let mut view = DetailView::new("JSON", "café 🐈\nnext\t\u{1b}[31m\u{202e}end");
        let mut terminal = Terminal::new(TestBackend::new(64, 10)).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("café 🐈"));
        assert!(rendered.contains("next    "));
        assert!(rendered.contains("\\u{001B}[31m"));
        assert!(rendered.contains("\\u{202E}end"));
    }

    #[test]
    fn wraps_and_scrolls_to_end_and_home_in_tiny_viewport() {
        let content = (0..40)
            .map(|n| format!("行 {n:02} abcdefghijklmnop"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut view = DetailView::new("Long value", content);
        let mut terminal = Terminal::new(TestBackend::new(14, 5)).unwrap();
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("行"));
        assert!(rendered.contains("00"));
        assert_eq!(count_lines("abcdef", 3, true), 2);
        view.handle_key(key(KeyCode::Char('w')));
        assert!(!view.wrap);
        view.handle_key(key(KeyCode::Right));
        assert_eq!(view.horizontal, 4);
        view.handle_key(key(KeyCode::Home));
        view.handle_key(key(KeyCode::End));
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("行"));
        assert!(rendered.contains("39"));
        view.handle_key(key(KeyCode::Home));
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("行"));
        assert!(rendered.contains("00"));
        assert!(view.handle_key(key(KeyCode::Esc)));

        let capped = DetailView::new(
            "t".repeat(MAX_TITLE_BYTES + 1),
            "x".repeat(MAX_DETAIL_BYTES + 1),
        );
        assert!(capped.text.ends_with("[Detail text truncated at 256 KiB]"));
        assert!(capped.title.len() < MAX_TITLE_BYTES + 32);
    }
}
