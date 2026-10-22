use std::num::NonZeroU16;

const SCROLLBACK_ROWS: usize = 1000;

#[derive(Clone)]
pub struct Row {
    pub cells: Vec<vt100::Cell>,
    pub wrapped: bool,
}

impl Row {
    pub fn text(&self, start: usize, end: usize) -> String {
        let mut text = String::new();
        let cells = &self.cells[start.min(self.cells.len())..end.min(self.cells.len())];
        let printed_end = cells
            .iter()
            .rposition(|cell| cell.has_contents() || cell.is_wide_continuation())
            .map_or(0, |column| column + 1);
        for cell in &cells[..printed_end] {
            if cell.is_wide_continuation() {
                continue;
            }
            if cell.has_contents() {
                text.push_str(cell.contents());
            } else {
                text.push(' ');
            }
        }
        text
    }

    pub fn link_at(&self, column: usize) -> Option<(std::ops::Range<usize>, String)> {
        let non_space = |cell: &vt100::Cell| {
            cell.is_wide_continuation()
                || (cell.has_contents() && !cell.contents().chars().any(char::is_whitespace))
        };
        if !self.cells.get(column).is_some_and(non_space) {
            return None;
        }
        let start = (0..column)
            .rev()
            .find(|&i| !non_space(&self.cells[i]))
            .map_or(0, |i| i + 1);
        let end = (column..self.cells.len())
            .find(|&i| !non_space(&self.cells[i]))
            .unwrap_or(self.cells.len());
        let mut text = self.text(start, end);
        let url = text
            .trim_start_matches(['(', '[', '<'])
            .trim_end_matches(['.', ',', ')', ']', '>']);
        if !url.starts_with("https://") && !url.starts_with("http://") {
            return None;
        }
        let leading = text.len() - text.trim_start_matches(['(', '[', '<']).len();
        let trailing = text.len() - leading - url.len();
        text.truncate(text.len() - trailing);
        text.drain(..leading);
        Some((start + leading..end - trailing, text))
    }
}

pub struct Buffer {
    parser: vt100::Parser,
    source: Vec<u8>,
    pub rows: Vec<Row>,
    pub cursor: (usize, usize),
    pub cursor_hidden: bool,
    pub size: (u16, u16),
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new(24, 80)
    }
}

impl Buffer {
    pub fn new(rows: u16, cols: u16) -> Self {
        let rows = rows.max(1);
        let cols = cols.max(1);
        Self {
            parser: vt100::Parser::new(nonzero(rows), nonzero(cols), SCROLLBACK_ROWS),
            source: Vec::new(),
            rows: Vec::new(),
            cursor: (0, 0),
            cursor_hidden: false,
            size: (rows, cols),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.source.is_empty()
    }

    pub fn append(&mut self, bytes: &[u8]) {
        self.source.extend_from_slice(bytes);
        self.parser.process(bytes);
        self.snapshot();
    }

    pub fn replace(&mut self, bytes: &[u8]) {
        self.parser =
            vt100::Parser::new(nonzero(self.size.0), nonzero(self.size.1), SCROLLBACK_ROWS);
        self.source.clear();
        self.append(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> bool {
        let size = (rows.max(1), cols.max(1));
        if self.size == size {
            return false;
        }
        self.size = size;
        self.parser = vt100::Parser::new(nonzero(size.0), nonzero(size.1), SCROLLBACK_ROWS);
        self.parser.process(&self.source);
        self.snapshot();
        true
    }

    pub fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    pub fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    fn snapshot(&mut self) {
        let screen = self.parser.screen_mut();
        screen.set_scrollback(0);
        let history = screen.scrollback_len();
        let (cursor_row, cursor_col) = screen.cursor_position();
        self.cursor = (history + usize::from(cursor_row), usize::from(cursor_col));
        self.cursor_hidden = screen.hide_cursor();
        self.rows.clear();
        for line in 0..history + usize::from(self.size.0) {
            let row = if line < history {
                screen.set_scrollback(history - line);
                0
            } else {
                screen.set_scrollback(0);
                (line - history) as u16
            };
            self.rows.push(Row {
                cells: (0..self.size.1)
                    .filter_map(|col| screen.cell(row, col).cloned())
                    .collect(),
                wrapped: screen.row_wrapped(row),
            });
        }
        screen.set_scrollback(0);
    }

    pub fn selected(&self, anchor: (usize, usize), head: (usize, usize)) -> String {
        let (start, end) = if anchor <= head {
            (anchor, head)
        } else {
            (head, anchor)
        };
        let mut selected = String::new();
        for line in start.0..=end.0.min(self.rows.len().saturating_sub(1)) {
            let row = &self.rows[line];
            let from = if line == start.0 { start.1 } else { 0 };
            let to = if line == end.0 {
                end.1
            } else {
                row.cells.len()
            };
            selected.push_str(&row.text(from, to));
            if line < end.0 && !row.wrapped {
                selected.push('\n');
            }
        }
        selected
    }
}

fn nonzero(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value.max(1)).expect("positive terminal dimension")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streamed_output_keeps_colors_and_selection_across_refits() {
        let mut buffer = Buffer::new(3, 6);
        buffer.append(b"\x1b[31");
        buffer.append(b"mabcdefghi\x1b[0m\r\n");
        buffer.append("e\u{301}\t\u{754c}".as_bytes());
        assert_eq!(buffer.rows[0].cells[0].fgcolor(), vt100::Color::Idx(1));
        assert_eq!(buffer.selected((0, 0), (1, 3)), "abcdefghi");
        buffer.resize(3, 12);
        assert_eq!(buffer.rows[0].text(0, 12), "abcdefghi");
        assert_eq!(buffer.rows[0].cells[8].fgcolor(), vt100::Color::Idx(1));
        assert!(buffer.rows.iter().any(|row| row.text(0, 12).contains('界')));
    }

    #[test]
    fn terminal_control_sequences_replace_progress_and_keep_scrollback() {
        let mut buffer = Buffer::new(2, 20);
        buffer.append(b"first\r\nold progress\r\x1b[2Knew progress\r\nlast");
        assert_eq!(buffer.selected((0, 0), (2, 4)), "first\nnew progress\nlast");
        buffer.append(b"\x1b[?2004h");
        assert!(buffer.bracketed_paste());
    }

    #[test]
    fn links_exclude_neighboring_text_and_punctuation() {
        let mut buffer = Buffer::new(2, 80);
        buffer.append(b"log (https://example.com/x). next");
        let row = &buffer.rows[0];
        let (columns, url) = row.link_at(12).expect("link under pointer");
        assert_eq!(url, "https://example.com/x");
        assert_eq!(row.text(columns.start, columns.end), url);
        assert!(row.link_at(0).is_none());
        assert!(row.link_at(29).is_none());
    }

    #[test]
    fn copied_wrapped_rows_preserve_printed_spaces() {
        let mut buffer = Buffer::new(2, 4);
        buffer.append(b"ab  cd");
        assert_eq!(buffer.selected((0, 0), (1, 2)), "ab  cd");
    }

    #[test]
    fn scrollback_matches_the_original_limit_after_resize() {
        let output: String = (0..1200).map(|line| format!("{line:04}\r\n")).collect();
        let expected: String = (199..1200).map(|line| format!("{line:04}\n")).collect();
        let mut buffer = Buffer::new(2, 8);
        buffer.append(output.as_bytes());
        assert_eq!(
            buffer.selected((0, 0), (buffer.rows.len() - 1, 8)),
            expected
        );
        buffer.resize(2, 6);
        assert_eq!(
            buffer.selected((0, 0), (buffer.rows.len() - 1, 6)),
            expected
        );
    }
}
