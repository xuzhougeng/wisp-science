//! A terminal's screen as the window draws it, from what the host's PTY
//! wrote (the WebView draws the same output with xterm.js): `vt100` keeps
//! the grid, its colours, the cursor and the lines scrolled away; this
//! turns each row into runs of one style, keys into the bytes a terminal
//! sends for them, a mouse selection into the cells and text it covers,
//! and keeps what the system's input method is composing. Pure, so it is
//! tested here.
use std::{num::NonZeroU16, ops::Range};

use base64::Engine as _;
use wisp_dto::native_conversations::TerminalOutput;

use crate::notify::System;

/// Lines kept above the screen to scroll back to.
pub const SCROLLBACK: usize = 5_000;

/// How a run of cells looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: vt100::Color,
    pub bg: vt100::Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

/// Cells of one style, side by side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub style: Style,
}

/// Counts the lines `vt100` lets go of above the scrollback, on each
/// screen, so a line keeps one number while it scrolls.
#[derive(Default)]
struct Dropped {
    main: u64,
    alternate: u64,
}

impl vt100::Callbacks for Dropped {
    fn on_scroll(&mut self, _: vt100::capture::RowContents<'_>, alternate: bool) {
        if alternate {
            self.alternate += 1;
        } else {
            self.main += 1;
        }
    }
}

/// A place between two cells: a line, counted from the first the screen
/// ever had so scrolling does not move it, and a column edge, from 0 to
/// the width.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub line: u64,
    pub col: u16,
}

/// What a selection grows by: cells for a drag, words after a double
/// click, lines after a triple click, as xterm.js does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Cell,
    Word,
    Line,
}

/// Part of the screen picked with the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// What the first click took: one place for a drag, the word or the
    /// line it was on otherwise.
    anchor: (Spot, Spot),
    unit: Unit,
    /// From and up to, in reading order.
    pub start: Spot,
    pub end: Spot,
}

/// The characters xterm.js ends a word at (its `wordSeparator`).
const WORD_SEPARATORS: &str = " ()[]{}',\"`";

/// A cell that reads as a space: one, or one never written (`\0`).
fn is_blank(cell: &str) -> bool {
    cell == " " || cell == "\0"
}

pub struct Screen {
    parser: vt100::Parser<Dropped>,
    /// How far the host's output has been read.
    pub cursor: Option<u64>,
    /// How the terminal ended, once it has.
    pub exit: Option<u32>,
    selection: Option<Selection>,
}

fn size(rows: u16, cols: u16) -> (NonZeroU16, NonZeroU16) {
    (
        NonZeroU16::new(rows.max(2)).unwrap_or(NonZeroU16::MIN),
        NonZeroU16::new(cols.max(2)).unwrap_or(NonZeroU16::MIN),
    )
}

impl Screen {
    pub fn new(rows: u16, cols: u16) -> Self {
        let (rows, cols) = size(rows, cols);
        Self {
            parser: vt100::Parser::new_with_callbacks(rows, cols, SCROLLBACK, Dropped::default()),
            cursor: None,
            exit: None,
            selection: None,
        }
    }

    /// Takes in what the host read; a reset (the first read, or one that
    /// fell behind what the host keeps) starts the screen over from it.
    pub fn apply(&mut self, output: &TerminalOutput) {
        if output.reset {
            let (rows, cols) = self.size();
            *self = Self::new(rows, cols);
        }
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&output.base64) {
            let alternate = self.parser.screen().alternate_screen();
            self.parser.process(&bytes);
            // Another screen drops the selection, and lines let go of
            // take it with them, as in xterm.js.
            if self.parser.screen().alternate_screen() != alternate {
                self.selection = None;
            }
            let first = self.first_line();
            match self.selection {
                Some(selection) if selection.end.line < first => self.selection = None,
                Some(ref mut selection) if selection.start.line < first => {
                    selection.start = Spot {
                        line: first,
                        col: 0,
                    };
                }
                _ => {}
            }
        }
        self.cursor = Some(output.end);
        self.exit = output.exit_code;
    }

    /// Rows and columns.
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.screen().size();
        (rows.get(), cols.get())
    }

    /// Gives the screen `rows` by `cols`; whether that changed it.
    pub fn resize(&mut self, rows: u16, cols: u16) -> bool {
        if self.size() == (rows.max(2), cols.max(2)) {
            return false;
        }
        let (rows, cols) = size(rows, cols);
        self.parser.set_size(rows, cols);
        self.selection = None;
        true
    }

    /// Every row on screen, as runs of one style; a wide character's
    /// second cell is part of the first.
    pub fn rows(&self) -> Vec<Vec<Run>> {
        let screen = self.parser.screen();
        let (rows, cols) = self.size();
        (0..rows)
            .map(|row| {
                let mut runs: Vec<Run> = Vec::new();
                for col in 0..cols {
                    let Some(cell) = screen.cell(row, col) else {
                        continue;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let style = Style {
                        fg: cell.fgcolor(),
                        bg: cell.bgcolor(),
                        bold: cell.bold(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        inverse: cell.inverse(),
                    };
                    let text = if cell.has_contents() {
                        cell.contents()
                    } else {
                        " "
                    };
                    match runs.last_mut() {
                        Some(last) if last.style == style => last.text.push_str(text),
                        _ => runs.push(Run {
                            text: text.to_owned(),
                            style,
                        }),
                    }
                }
                // Trailing blanks in the default style draw nothing.
                if let Some(last) = runs.last_mut() {
                    if last.style == Style::default() {
                        let kept = last.text.trim_end().len();
                        last.text.truncate(kept);
                    }
                }
                runs.retain(|run| !run.text.is_empty());
                runs
            })
            .collect()
    }

    /// Where the cursor is drawn: not when the program hid it, nor while
    /// the view is scrolled back.
    pub fn cursor_at(&self) -> Option<(u16, u16)> {
        let screen = self.parser.screen();
        (!screen.hide_cursor() && screen.scrollback() == 0).then(|| screen.cursor_position())
    }

    /// Where typing goes, hidden or not: where a composition is shown.
    pub fn typing_at(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    /// Scrolls the view back (`lines` above zero) or forward; zero is the
    /// live screen.
    pub fn scroll(&mut self, lines: i32) {
        let screen = self.parser.screen_mut();
        let now = screen.scrollback() as i64;
        screen.set_scrollback((now + i64::from(lines)).max(0) as usize);
    }

    /// Lines scrolled back from the live screen.
    pub fn scrolled(&self) -> usize {
        self.parser.screen().scrollback()
    }

    /// Back to the live screen, as typing brings it.
    pub fn follow(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }

    pub fn application_cursor(&self) -> bool {
        self.parser.screen().application_cursor()
    }

    pub fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    /// The text on screen.
    #[cfg(test)]
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    /// The number of the first line still kept.
    fn first_line(&self) -> u64 {
        let screen = self.parser.screen();
        let dropped = self.parser.callbacks();
        if screen.alternate_screen() {
            dropped.alternate
        } else {
            dropped.main
        }
    }

    /// The number of the line shown on the view's first row.
    fn top_line(&self) -> u64 {
        let screen = self.parser.screen();
        self.first_line() + (screen.scrollback_len() - screen.scrollback()) as u64
    }

    /// The number of the last line kept: the live screen's last row.
    fn last_line(&self) -> u64 {
        let screen = self.parser.screen();
        self.first_line() + screen.scrollback_len() as u64 + u64::from(self.size().0) - 1
    }

    /// Reads `line` where it is, scrolled back or not, and puts the view
    /// back as it was.
    fn with_line<R>(
        &mut self,
        line: u64,
        read: impl FnOnce(&vt100::Screen, u16) -> R,
    ) -> Option<R> {
        let first = self.first_line();
        if line < first || line > self.last_line() {
            return None;
        }
        let index = (line - first) as usize;
        let screen = self.parser.screen_mut();
        let kept = screen.scrollback_len();
        let was = screen.scrollback();
        let row = if index < kept {
            screen.set_scrollback(kept - index);
            0
        } else {
            screen.set_scrollback(0);
            (index - kept) as u16
        };
        let result = read(screen, row);
        screen.set_scrollback(was);
        Some(result)
    }

    /// The cells of `line` as text, one string per cell (a wide
    /// character's second cell is empty), and whether it runs on into the
    /// next line.
    fn line_cells(&mut self, line: u64) -> Option<(Vec<String>, bool)> {
        let cols = self.size().1;
        self.with_line(line, |screen, row| {
            let cells = (0..cols)
                .map(|col| match screen.cell(row, col) {
                    Some(cell) if cell.is_wide_continuation() => String::new(),
                    Some(cell) if cell.has_contents() => cell.contents().to_owned(),
                    // Never written: kept apart from a written space so
                    // the end of a line can be trimmed as xterm.js does.
                    _ => "\0".to_owned(),
                })
                .collect();
            (cells, screen.row_wrapped(row))
        })
    }

    /// The place under the pointer: `row` of the view (above or below it
    /// is its first or last row) and `x` in cells from its left edge. The
    /// nearest column edge, and the cell the pointer is over.
    fn spot_at(&self, row: i64, x: f32) -> (Spot, Spot) {
        let (rows, cols) = self.size();
        let top = self.top_line();
        if row < 0 {
            let spot = Spot { line: top, col: 0 };
            return (spot, spot);
        }
        if row >= i64::from(rows) {
            let line = top + u64::from(rows) - 1;
            return (
                Spot { line, col: cols },
                Spot {
                    line,
                    col: cols - 1,
                },
            );
        }
        let line = top + row as u64;
        let x = x.max(0.);
        (
            Spot {
                line,
                col: (x.round() as u16).min(cols),
            },
            Spot {
                line,
                col: (x.floor() as u16).min(cols - 1),
            },
        )
    }

    /// The word under `cell`, or the run of blanks it is in.
    fn word_at(&mut self, cell: Spot) -> (Spot, Spot) {
        let Some((cells, _)) = self.line_cells(cell.line) else {
            return (cell, cell);
        };
        let at = usize::from(cell.col).min(cells.len().saturating_sub(1));
        // A wide character's second cell belongs to its first.
        let at = if cells[at].is_empty() && at > 0 {
            at - 1
        } else {
            at
        };
        let blanks = is_blank(&cells[at]);
        let joins = |text: &str| {
            if blanks {
                is_blank(text)
            } else {
                text.is_empty() || !(is_blank(text) || WORD_SEPARATORS.contains(text))
            }
        };
        let mut start = at;
        while start > 0 && joins(&cells[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < cells.len() && joins(&cells[end]) {
            end += 1;
        }
        (
            Spot {
                line: cell.line,
                col: start as u16,
            },
            Spot {
                line: cell.line,
                col: end as u16,
            },
        )
    }

    /// The whole of the line `line` is part of, over the lines it wraps
    /// onto.
    fn line_at(&mut self, line: u64) -> (Spot, Spot) {
        let cols = self.size().1;
        let mut first = line;
        while first > self.first_line()
            && self.line_cells(first - 1).is_some_and(|(_, wraps)| wraps)
        {
            first -= 1;
        }
        let mut last = line;
        while last < self.last_line() && self.line_cells(last).is_some_and(|(_, wraps)| wraps) {
            last += 1;
        }
        (
            Spot {
                line: first,
                col: 0,
            },
            Spot {
                line: last,
                col: cols,
            },
        )
    }

    /// Where the selection reaches with its moving end at the pointer.
    fn reach(&mut self, anchor: (Spot, Spot), unit: Unit, row: i64, x: f32) -> (Spot, Spot) {
        let (edge, cell) = self.spot_at(row, x);
        let (from, to) = match unit {
            Unit::Cell => (edge, edge),
            Unit::Word => self.word_at(cell),
            Unit::Line => self.line_at(cell.line),
        };
        if from < anchor.0 {
            (from, anchor.1)
        } else {
            (anchor.0, to.max(anchor.1))
        }
    }

    /// A press of the mouse at the pointer: `clicks` in a row picks a
    /// place, a word or a line.
    pub fn select_from(&mut self, row: i64, x: f32, clicks: usize) {
        let (edge, cell) = self.spot_at(row, x);
        let (unit, anchor) = match clicks {
            0 | 1 => (Unit::Cell, (edge, edge)),
            2 => (Unit::Word, self.word_at(cell)),
            _ => (Unit::Line, self.line_at(cell.line)),
        };
        self.selection = Some(Selection {
            anchor,
            unit,
            start: anchor.0,
            end: anchor.1,
        });
    }

    /// The selection's moving end to the pointer, by what it grows by; a
    /// selection is begun when there is none.
    pub fn select_to(&mut self, row: i64, x: f32) {
        let Some(selection) = self.selection else {
            self.select_from(row, x, 1);
            return;
        };
        let (start, end) = self.reach(selection.anchor, selection.unit, row, x);
        self.selection = Some(Selection {
            start,
            end,
            ..selection
        });
    }

    /// Everything kept, scrolled away or on screen.
    pub fn select_all(&mut self) {
        let start = Spot {
            line: self.first_line(),
            col: 0,
        };
        let end = Spot {
            line: self.last_line(),
            col: self.size().1,
        };
        self.selection = Some(Selection {
            anchor: (start, end),
            unit: Unit::Cell,
            start,
            end,
        });
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The selection, when it covers anything.
    pub fn selection(&self) -> Option<Selection> {
        self.selection
            .filter(|selection| selection.start < selection.end)
    }

    /// The columns selected on `row` of the view, from and up to.
    pub fn selected_cols(&self, row: u16) -> Option<(u16, u16)> {
        let selection = self.selection()?;
        let line = self.top_line() + u64::from(row);
        if line < selection.start.line || line > selection.end.line {
            return None;
        }
        let from = if line == selection.start.line {
            selection.start.col
        } else {
            0
        };
        let to = if line == selection.end.line {
            selection.end.col
        } else {
            self.size().1
        };
        (from < to).then_some((from, to))
    }

    /// The selected text, as xterm.js copies it: each line without the
    /// cells nothing was written to at its end, a line that wraps joined to
    /// the next, and lines ended by `\r\n` on Windows and `\n` elsewhere.
    pub fn selected_text(&mut self, system: System) -> Option<String> {
        let selection = self.selection()?;
        let mut lines: Vec<String> = Vec::new();
        let mut joined = false;
        for line in selection.start.line..=selection.end.line {
            let Some((cells, wraps)) = self.line_cells(line) else {
                continue;
            };
            let from = if line == selection.start.line {
                usize::from(selection.start.col)
            } else {
                0
            };
            let to = if line == selection.end.line {
                usize::from(selection.end.col)
            } else {
                cells.len()
            }
            .min(cells.len());
            let from = from.min(to);
            // Cells never written at the end are not copied.
            let written = cells
                .iter()
                .rposition(|cell| cell != "\0")
                .map_or(0, |last| last + 1);
            let text: String = cells[from..to.min(written).max(from)]
                .iter()
                .map(|cell| if cell == "\0" { " " } else { cell.as_str() })
                .collect();
            match lines.last_mut() {
                Some(last) if joined => last.push_str(&text),
                _ => lines.push(text),
            }
            joined = wraps;
        }
        let end = if system == System::Windows {
            "\r\n"
        } else {
            "\n"
        };
        Some(lines.join(end))
    }
}

/// What the system's input method is composing, before it is typed: shown
/// at the cursor and sent only once the input method commits it, as
/// xterm.js does with the browser's composition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composition {
    text: String,
    /// The input method's caret in it, in UTF-16 units.
    caret: Option<Range<usize>>,
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The part of `text` between two UTF-16 offsets, clamped to it.
fn utf16_slice(text: &str, range: Range<usize>) -> (String, Range<usize>) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let end = range.end.min(units.len());
    let start = range.start.min(end);
    (String::from_utf16_lossy(&units[start..end]), start..end)
}

impl Composition {
    /// What is being composed, when anything is.
    pub fn marked(&self) -> Option<&str> {
        (!self.text.is_empty()).then_some(self.text.as_str())
    }

    /// Its extent, in UTF-16 units, as the input method counts.
    pub fn marked_range(&self) -> Option<Range<usize>> {
        self.marked().map(|text| 0..utf16_len(text))
    }

    /// Where the input method's caret is: in the composition, or after it.
    pub fn selected_range(&self) -> Range<usize> {
        let len = utf16_len(&self.text);
        self.caret.clone().unwrap_or(len..len)
    }

    /// The composition's text in `range`, and the range it is.
    pub fn text_for_range(&self, range: Range<usize>) -> (String, Range<usize>) {
        utf16_slice(&self.text, range)
    }

    /// The input method shows `text` as being composed; empty ends it.
    pub fn mark(&mut self, text: &str, caret: Option<Range<usize>>) {
        self.text = text.to_owned();
        self.caret = caret;
    }

    /// The input method commits `text`; what to type, if anything.
    pub fn commit(&mut self, text: &str) -> Option<String> {
        *self = Self::default();
        (!text.is_empty()).then(|| text.to_owned())
    }

    /// The input method lets go of what it was composing (a click away,
    /// the input method switched): it is dropped, not typed, since only
    /// what an input method commits is meant to reach the program.
    pub fn unmark(&mut self) {
        *self = Self::default();
    }

    /// Where the input method's text from `offset` (UTF-16) starts, in
    /// characters into the composition.
    pub fn prefix(&self, offset: usize) -> String {
        utf16_slice(&self.text, 0..offset).0
    }
}

/// A key as the window reports it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Key<'a> {
    pub name: &'a str,
    /// What it types, when it types something.
    pub typed: Option<&'a str>,
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
}

/// The bytes a terminal sends for `key`, or none for a key it does not
/// send: the window's own shortcuts, and text, which the system's input
/// method types (`Composition`) so a character it composes reaches the
/// terminal as one. As xterm.js does, Option composes characters on
/// macOS, and AltGr (Ctrl+Alt) does on Windows; elsewhere Alt sends Escape
/// before the key.
pub fn key_bytes(key: Key, application_cursor: bool, system: System) -> Option<Vec<u8>> {
    let arrow = |letter: u8| {
        if application_cursor {
            vec![0x1b, b'O', letter]
        } else {
            vec![0x1b, b'[', letter]
        }
    };
    let mut bytes = match key.name {
        "enter" => vec![b'\r'],
        "backspace" => vec![0x7f],
        "tab" if key.shift => b"\x1b[Z".to_vec(),
        "tab" => vec![b'\t'],
        "escape" => vec![0x1b],
        "up" => arrow(b'A'),
        "down" => arrow(b'B'),
        "right" => arrow(b'C'),
        "left" => arrow(b'D'),
        "home" => arrow(b'H'),
        "end" => arrow(b'F'),
        "pageup" => b"\x1b[5~".to_vec(),
        "pagedown" => b"\x1b[6~".to_vec(),
        "delete" => b"\x1b[3~".to_vec(),
        "insert" => b"\x1b[2~".to_vec(),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "space" if key.control => vec![0],
        _ if key.control && key.alt && system == System::Windows => return None,
        name if key.control && name.len() == 1 => {
            let byte = name.as_bytes()[0].to_ascii_lowercase();
            match byte {
                b'a'..=b'z' => vec![byte - b'a' + 1],
                b'[' => vec![0x1b],
                b'\\' => vec![0x1c],
                b']' => vec![0x1d],
                _ => return None,
            }
        }
        _ if !key.alt || key.control || system == System::Mac => return None,
        "space" => vec![b' '],
        _ => match key.typed {
            Some(text) if !text.is_empty() => text.as_bytes().to_vec(),
            _ => return None,
        },
    };
    // Alt sends Escape first, as `meta sends escape` terminals do.
    if key.alt && !key.control && bytes.len() == 1 {
        bytes.insert(0, 0x1b);
    }
    Some(bytes)
}

/// What pasting `text` sends: line ends as Return, and fenced when the
/// program asked for bracketed paste.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    let mut bytes = Vec::with_capacity(text.len() + 12);
    if bracketed {
        bytes.extend_from_slice(b"\x1b[200~");
    }
    bytes.extend_from_slice(text.as_bytes());
    if bracketed {
        bytes.extend_from_slice(b"\x1b[201~");
    }
    bytes
}

/// A colour as red, green and blue: the xterm palette for an indexed one,
/// and `default` for the terminal's own.
pub fn rgb(color: vt100::Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    const BASE: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xcd, 0x31, 0x31),
        (0x0d, 0xbc, 0x79),
        (0xe5, 0xe5, 0x10),
        (0x24, 0x72, 0xc8),
        (0xbc, 0x3f, 0xbc),
        (0x11, 0xa8, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x66, 0x66, 0x66),
        (0xf1, 0x4c, 0x4c),
        (0x23, 0xd1, 0x8b),
        (0xf5, 0xf5, 0x43),
        (0x3b, 0x8e, 0xea),
        (0xd6, 0x70, 0xd6),
        (0x29, 0xb8, 0xdb),
        (0xff, 0xff, 0xff),
    ];
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => (r, g, b),
        vt100::Color::Idx(index @ 0..=15) => BASE[index as usize],
        vt100::Color::Idx(index @ 16..=231) => {
            let step = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            let cube = index - 16;
            (step(cube / 36), step((cube / 6) % 6), step(cube % 6))
        }
        vt100::Color::Idx(index) => {
            let level = 8 + (index - 232) * 10;
            (level, level, level)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(bytes: &[u8], reset: bool, end: u64) -> TerminalOutput {
        TerminalOutput {
            terminal_id: "t".into(),
            start: 0,
            end,
            base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            reset,
            exit_code: None,
        }
    }

    #[test]
    fn output_is_drawn_as_runs_of_one_style() {
        let mut screen = Screen::new(4, 20);
        screen.apply(&output(b"plain \x1b[1;31mred\x1b[0m end\r\nnext", true, 30));
        let rows = screen.rows();
        let texts: Vec<&str> = rows[0].iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, ["plain ", "red", " end"]);
        assert_eq!(rows[0][1].style.fg, vt100::Color::Idx(1));
        assert!(rows[0][1].style.bold);
        assert_eq!(rows[1][0].text, "next");
        assert!(rows[2].is_empty());
        assert_eq!(screen.cursor_at(), Some((1, 4)));
        assert_eq!(screen.cursor, Some(30));
        // More output carries on; a reset starts over.
        screen.apply(&output(b"!", false, 31));
        assert_eq!(screen.rows()[1][0].text, "next!");
        screen.apply(&output(b"fresh", true, 40));
        assert_eq!(screen.rows()[0][0].text, "fresh");
        assert!(screen.rows()[1].is_empty());
    }

    #[test]
    fn the_screen_resizes_and_scrolls_back() {
        let mut screen = Screen::new(3, 10);
        assert!(!screen.resize(3, 10));
        assert!(screen.resize(2, 12));
        assert_eq!(screen.size(), (2, 12));
        let lines: Vec<u8> = (0..6)
            .flat_map(|n| format!("line{n}\r\n").into_bytes())
            .collect();
        screen.apply(&output(&lines, false, 1));
        screen.scroll(2);
        assert_eq!(screen.scrolled(), 2);
        assert_eq!(screen.cursor_at(), None);
        assert!(screen.text().contains("line3"));
        screen.scroll(-10);
        assert_eq!(screen.scrolled(), 0);
        screen.scroll(1);
        screen.follow();
        assert_eq!(screen.scrolled(), 0);
    }

    #[test]
    fn keys_become_what_a_terminal_sends() {
        let key = |name: &'static str| Key {
            name,
            ..Key::default()
        };
        let linux = System::Linux;
        assert_eq!(key_bytes(key("enter"), false, linux), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(key("backspace"), false, linux), Some(vec![0x7f]));
        assert_eq!(key_bytes(key("up"), false, linux), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(key("up"), true, linux), Some(b"\x1bOA".to_vec()));
        let ctrl_c = Key {
            name: "c",
            typed: Some("c"),
            control: true,
            ..Key::default()
        };
        for system in [System::Mac, System::Linux, System::Windows] {
            assert_eq!(key_bytes(ctrl_c, false, system), Some(vec![3]));
        }
        // Text is the input method's to type, on every system.
        let typed = Key {
            name: "a",
            typed: Some("A"),
            shift: true,
            ..Key::default()
        };
        for system in [System::Mac, System::Linux, System::Windows] {
            assert_eq!(key_bytes(typed, false, system), None);
            assert_eq!(key_bytes(key("space"), false, system), None);
        }
        // Alt sends Escape first off macOS; Option composes on macOS.
        let alt_b = Key {
            name: "b",
            typed: Some("b"),
            alt: true,
            ..Key::default()
        };
        assert_eq!(key_bytes(alt_b, false, linux), Some(b"\x1bb".to_vec()));
        assert_eq!(
            key_bytes(alt_b, false, System::Windows),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(key_bytes(alt_b, false, System::Mac), None);
        let alt_backspace = Key {
            alt: true,
            ..key("backspace")
        };
        assert_eq!(
            key_bytes(alt_backspace, false, System::Mac),
            Some(vec![0x1b, 0x7f])
        );
        // AltGr on Windows types its character.
        let alt_gr_q = Key {
            name: "q",
            typed: Some("@"),
            control: true,
            alt: true,
            ..Key::default()
        };
        assert_eq!(key_bytes(alt_gr_q, false, System::Windows), None);
        assert_eq!(key_bytes(alt_gr_q, false, linux), Some(vec![0x11]));
        let back_tab = Key {
            name: "tab",
            shift: true,
            ..Key::default()
        };
        assert_eq!(key_bytes(back_tab, false, linux), Some(b"\x1b[Z".to_vec()));
        // A key that types nothing sends nothing.
        assert_eq!(key_bytes(key("f12"), false, linux), None);
    }

    fn screen(rows: u16, cols: u16, text: &[u8]) -> Screen {
        let mut screen = Screen::new(rows, cols);
        screen.apply(&output(text, false, 1));
        screen
    }

    #[test]
    fn a_drag_selects_from_edge_to_edge_and_copies_as_xterm_does() {
        let mut screen = screen(4, 20, b"hello world\r\n  two  \r\nthree");
        assert_eq!(screen.selection(), None);
        // A press without a drag selects nothing.
        screen.select_from(0, 6.2, 1);
        assert_eq!(screen.selection(), None);
        assert_eq!(screen.selected_text(System::Linux), None);
        // Dragging to the middle of the next row: the nearest edges.
        screen.select_to(1, 4.6);
        assert_eq!(screen.selected_cols(0), Some((6, 20)));
        assert_eq!(screen.selected_cols(1), Some((0, 5)));
        assert_eq!(screen.selected_cols(2), None);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("world\n  two")
        );
        assert_eq!(
            screen.selected_text(System::Windows).as_deref(),
            Some("world\r\n  two")
        );
        // Back past where it began; past the last row is its end. Spaces
        // written at a line's end are kept, cells never written are not.
        screen.select_to(-3, 0.);
        assert_eq!(screen.selected_text(System::Mac).as_deref(), Some("hello "));
        screen.select_to(9, 0.);
        assert_eq!(
            screen.selected_text(System::Mac).as_deref(),
            Some("world\n  two  \nthree\n")
        );
        // Typing, a new size and another screen each drop it.
        screen.clear_selection();
        assert_eq!(screen.selection(), None);
        screen.select_from(0, 0., 1);
        screen.select_to(0, 5.);
        assert!(screen.resize(4, 21));
        assert_eq!(screen.selection(), None);
        screen.select_from(0, 0., 1);
        screen.select_to(0, 5.);
        screen.apply(&output(b"\x1b[?1049h", false, 2));
        assert_eq!(screen.selection(), None);
    }

    #[test]
    fn a_double_click_takes_a_word_and_a_triple_click_the_wrapped_line() {
        let mut screen = screen(4, 20, b"run (foo.bar) now");
        screen.select_from(0, 7.5, 2);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("foo.bar")
        );
        // Dragging on grows it by words.
        screen.select_to(0, 15.);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("foo.bar) now")
        );
        screen.select_to(0, 1.);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("run (foo.bar")
        );
        // On blank cells, the run of them.
        screen.select_from(0, 18.5, 2);
        assert_eq!(screen.selected_cols(0), Some((17, 20)));

        let mut screen = screen_wrapped();
        screen.select_from(1, 1., 3);
        assert_eq!(screen.selected_cols(0), Some((0, 10)));
        assert_eq!(screen.selected_cols(1), Some((0, 10)));
        assert_eq!(screen.selected_cols(2), None);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("abcdefghijKLM")
        );
        // A drag over the wrap joins the lines too.
        screen.select_from(0, 8., 1);
        screen.select_to(2, 2.);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("ijKLM\nne")
        );
    }

    fn screen_wrapped() -> Screen {
        screen(4, 10, b"abcdefghijKLM\r\nnext")
    }

    #[test]
    fn wide_characters_are_copied_whole_and_selected_as_one_word() {
        let mut screen = screen(2, 10, "中文ab x".as_bytes());
        screen.select_from(0, 0., 1);
        screen.select_to(0, 3.);
        assert_eq!(screen.selected_text(System::Linux).as_deref(), Some("中文"));
        // A double click on a wide character's second half.
        screen.select_from(0, 1.5, 2);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("中文ab")
        );
        assert_eq!(screen.selected_cols(0), Some((0, 6)));
    }

    #[test]
    fn a_selection_stays_on_its_lines_as_they_scroll_and_goes_with_them() {
        let mut screen = screen(3, 10, b"first\r\nsecond\r\n");
        screen.select_from(0, 0., 1);
        screen.select_to(0, 5.);
        let lines: Vec<u8> = (0..4)
            .flat_map(|n| format!("more{n}\r\n").into_bytes())
            .collect();
        screen.apply(&output(&lines, false, 2));
        // "first" scrolled away above the screen: still it.
        assert_eq!(screen.selected_cols(0), None);
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("first")
        );
        screen.scroll(10);
        assert_eq!(screen.selected_cols(0), Some((0, 5)));
        assert_eq!(screen.scrolled(), 4);
        // Copying leaves the view where it was.
        assert_eq!(
            screen.selected_text(System::Linux).as_deref(),
            Some("first")
        );
        assert_eq!(screen.scrolled(), 4);
        screen.follow();
        // Everything kept, scrolled away or not.
        screen.select_all();
        let all = screen.selected_text(System::Linux).unwrap();
        assert!(
            all.starts_with("first\nsecond\nmore0\nmore1\nmore2\nmore3"),
            "{all}"
        );
        // Once its lines are let go of, so is the selection.
        screen.select_from(0, 0., 1);
        screen.select_to(0, 2.);
        let flood: Vec<u8> = (0..SCROLLBACK + 10)
            .flat_map(|n| format!("x{n}\r\n").into_bytes())
            .collect();
        screen.apply(&output(&flood, false, 3));
        assert_eq!(screen.selection(), None);
    }

    #[test]
    fn a_composition_is_kept_until_the_input_method_commits_it() {
        let mut composing = Composition::default();
        assert_eq!(composing.marked(), None);
        assert_eq!(composing.marked_range(), None);
        assert_eq!(composing.selected_range(), 0..0);
        composing.mark("zhong", None);
        assert_eq!(composing.marked(), Some("zhong"));
        assert_eq!(composing.marked_range(), Some(0..5));
        assert_eq!(composing.selected_range(), 5..5);
        assert_eq!(composing.text_for_range(1..3), ("ho".to_owned(), 1..3));
        assert_eq!(composing.text_for_range(3..99), ("ng".to_owned(), 3..5));
        // Committed, it is typed once and the composition ends.
        assert_eq!(composing.commit("中"), Some("中".to_owned()));
        assert_eq!(composing.marked(), None);
        // An emptied composition types nothing.
        composing.mark("ni", Some(1..1));
        assert_eq!(composing.selected_range(), 1..1);
        composing.mark("", None);
        assert_eq!(composing.marked(), None);
        assert_eq!(composing.commit(""), None);
        // UTF-16, as the input method counts; let go of, nothing is typed.
        composing.mark("😀é", None);
        assert_eq!(composing.marked_range(), Some(0..3));
        assert_eq!(composing.prefix(2), "😀");
        composing.unmark();
        assert_eq!(composing.marked(), None);
    }

    #[test]
    fn pasted_text_is_fenced_when_asked_and_colours_follow_xterm() {
        assert_eq!(paste_bytes("a\nb", false), b"a\rb".to_vec());
        assert_eq!(paste_bytes("x", true), b"\x1b[200~x\x1b[201~".to_vec());
        assert_eq!(rgb(vt100::Color::Default, (1, 2, 3)), (1, 2, 3));
        assert_eq!(rgb(vt100::Color::Idx(1), (0, 0, 0)), (0xcd, 0x31, 0x31));
        assert_eq!(rgb(vt100::Color::Idx(16), (9, 9, 9)), (0, 0, 0));
        assert_eq!(rgb(vt100::Color::Idx(231), (9, 9, 9)), (255, 255, 255));
        assert_eq!(rgb(vt100::Color::Idx(232), (9, 9, 9)), (8, 8, 8));
        assert_eq!(rgb(vt100::Color::Rgb(4, 5, 6), (0, 0, 0)), (4, 5, 6));
    }
}
