//! A terminal's screen as the window draws it, from what the host's PTY
//! wrote (the WebView draws the same output with xterm.js): `vt100` keeps
//! the grid, its colours, the cursor and the lines scrolled away; this
//! turns each row into runs of one style, and keys into the bytes a
//! terminal sends for them. Pure, so it is tested here.
use std::num::NonZeroU16;

use base64::Engine as _;
use wisp_dto::native_conversations::TerminalOutput;

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

pub struct Screen {
    parser: vt100::Parser,
    /// How far the host's output has been read.
    pub cursor: Option<u64>,
    /// How the terminal ended, once it has.
    pub exit: Option<u32>,
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
            parser: vt100::Parser::new(rows, cols, SCROLLBACK),
            cursor: None,
            exit: None,
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
            self.parser.process(&bytes);
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

    /// The text on screen, for copying.
    pub fn text(&self) -> String {
        self.parser.screen().contents()
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
/// send (the window's own shortcuts).
pub fn key_bytes(key: Key, application_cursor: bool) -> Option<Vec<u8>> {
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
        _ => match key.typed {
            Some(text) if !text.is_empty() && !key.control => text.as_bytes().to_vec(),
            _ if key.name == "space" => vec![b' '],
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
        assert_eq!(key_bytes(key("enter"), false), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(key("backspace"), false), Some(vec![0x7f]));
        assert_eq!(key_bytes(key("up"), false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(key("up"), true), Some(b"\x1bOA".to_vec()));
        let ctrl_c = Key {
            name: "c",
            typed: Some("c"),
            control: true,
            ..Key::default()
        };
        assert_eq!(key_bytes(ctrl_c, false), Some(vec![3]));
        let typed = Key {
            name: "a",
            typed: Some("A"),
            shift: true,
            ..Key::default()
        };
        assert_eq!(key_bytes(typed, false), Some(b"A".to_vec()));
        let alt_b = Key {
            name: "b",
            typed: Some("b"),
            alt: true,
            ..Key::default()
        };
        assert_eq!(key_bytes(alt_b, false), Some(b"\x1bb".to_vec()));
        let back_tab = Key {
            name: "tab",
            shift: true,
            ..Key::default()
        };
        assert_eq!(key_bytes(back_tab, false), Some(b"\x1b[Z".to_vec()));
        // A key that types nothing sends nothing.
        assert_eq!(key_bytes(key("f12"), false), None);
        assert_eq!(key_bytes(key("space"), false), Some(b" ".to_vec()));
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
