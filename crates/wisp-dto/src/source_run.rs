//! Running part of an R or Python source file in its interpreter, the way
//! RStudio does: the selection when there is one, otherwise the complete
//! statement under the caret (a call spread over lines, a pipe, an
//! `if`/`else`, a `def` body), after which the caret moves on to the next
//! statement. Shared by the WebView's source editor (`ui/`) and the GPUI
//! desktop's, so both send an interpreter the same code for the same caret.
//!
//! Offsets are UTF-16 code units, which is what both a browser textarea
//! (`selectionStart`) and the GPUI input (`selected_text_range`) report;
//! Rust strings index by byte, so every conversion is explicit here.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSelection {
    pub before: String,
    pub selected: String,
    pub after: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceExecution {
    pub code: String,
    /// When running a collapsed caret, move it to the following line just like
    /// RStudio. A real selection stays selected, so this is `None` for it.
    pub next_caret_utf16: Option<u32>,
}

/// The byte index in `text` of UTF-16 offset `target`, clamped to the text
/// and never inside a character.
pub fn utf16_to_byte_index(text: &str, target: u32) -> usize {
    let mut utf16 = 0_u32;
    for (byte, ch) in text.char_indices() {
        if utf16 >= target {
            return byte;
        }
        let next = utf16.saturating_add(ch.len_utf16() as u32);
        if next > target {
            return byte;
        }
        utf16 = next;
    }
    text.len()
}

/// The UTF-16 offset of byte index `byte` in `text`.
pub fn byte_to_utf16_index(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())].encode_utf16().count() as u32
}

fn source_line_at(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())]
        .bytes()
        .filter(|ch| *ch == b'\n')
        .count()
        + 1
}

/// Split a textarea selection using its UTF-16 DOM offsets. Keeping this
/// conversion explicit matters for Chinese source comments and astral Unicode:
/// Rust string indexes are bytes, while `selectionStart`/`selectionEnd` are
/// UTF-16 code units.
pub fn source_selection(text: &str, start_utf16: u32, end_utf16: u32) -> SourceSelection {
    let (start_utf16, end_utf16) = if start_utf16 <= end_utf16 {
        (start_utf16, end_utf16)
    } else {
        (end_utf16, start_utf16)
    };
    let start = utf16_to_byte_index(text, start_utf16);
    let end = utf16_to_byte_index(text, end_utf16).max(start);
    // When the selection ends immediately after a newline, that newline still
    // belongs to the preceding rendered line; do not report the empty next line.
    let end_for_line = if end > start && text.as_bytes().get(end - 1) == Some(&b'\n') {
        end - 1
    } else {
        end
    };
    SourceSelection {
        before: text[..start].to_string(),
        selected: text[start..end].to_string(),
        after: text[end..].to_string(),
        start_line: source_line_at(text, start),
        end_line: source_line_at(text, end_for_line),
    }
}

/// RStudio-style execution unit: run the selection when present, otherwise
/// run the complete statement that contains the caret (parenthesized calls,
/// pipes, `if`/`function`/`def` bodies) and advance to the next statement.
pub fn source_execution(
    text: &str,
    start_utf16: u32,
    end_utf16: u32,
    language: &str,
) -> Option<SourceExecution> {
    if end_utf16 > start_utf16 {
        let selection = source_selection(text, start_utf16, end_utf16);
        return (!selection.selected.trim().is_empty()).then_some(SourceExecution {
            code: selection.selected,
            next_caret_utf16: None,
        });
    }

    let caret = utf16_to_byte_index(text, start_utf16);
    let lang = exec_lang(language);
    let (stmt_start, stmt_end) = statement_bounds(text, caret, lang);
    let code = text[stmt_start..stmt_end].to_string();
    if code.trim().is_empty() {
        return None;
    }
    let next = next_statement_caret(text, stmt_end);
    Some(SourceExecution {
        code,
        next_caret_utf16: Some(byte_to_utf16_index(text, next)),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecLang {
    R,
    Python,
    Generic,
}

fn exec_lang(name: &str) -> ExecLang {
    match name {
        "r" => ExecLang::R,
        "python" => ExecLang::Python,
        _ => ExecLang::Generic,
    }
}

fn line_start_at(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    text[..byte].rfind('\n').map_or(0, |newline| newline + 1)
}

fn line_end_at(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    text[byte..]
        .find('\n')
        .map_or(text.len(), |offset| byte + offset)
}

fn prev_line_start(text: &str, current_start: usize) -> Option<usize> {
    if current_start == 0 {
        return None;
    }
    Some(line_start_at(text, current_start - 1))
}

fn line_indent(text: &str, line_start: usize) -> usize {
    let mut n = 0usize;
    for ch in text[line_start.min(text.len())..].chars() {
        match ch {
            ' ' => n += 1,
            '\t' => n += 8 - (n % 8),
            _ => break,
        }
    }
    n
}

fn line_is_trivia(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#')
}

fn next_nontrivia_line(text: &str, from: usize) -> Option<(usize, usize)> {
    let mut from = from;
    while from < text.len() {
        if text.as_bytes()[from] == b'\n' {
            from += 1;
            if from >= text.len() {
                return None;
            }
        }
        let end = line_end_at(text, from);
        if !line_is_trivia(&text[from..end]) {
            return Some((from, end));
        }
        if end >= text.len() {
            return None;
        }
        from = end;
    }
    None
}

fn next_statement_caret(text: &str, stmt_end: usize) -> usize {
    let mut next = if stmt_end < text.len() && text.as_bytes()[stmt_end] == b'\n' {
        stmt_end + 1
    } else {
        stmt_end
    };
    while next < text.len() {
        let end = line_end_at(text, next);
        if !line_is_trivia(&text[next..end]) {
            break;
        }
        next = if end < text.len() { end + 1 } else { end };
    }
    next
}

/// Smallest range covering the statement that contains `caret`.
fn statement_bounds(text: &str, caret: usize, lang: ExecLang) -> (usize, usize) {
    let mut start = line_start_at(text, caret);
    while let Some(prev) = prev_nontrivia_line_start(text, start) {
        if may_join_previous(text, prev, start, lang) {
            start = prev;
        } else {
            break;
        }
    }
    (start, expand_statement_end(text, start, lang))
}

fn prev_nontrivia_line_start(text: &str, current_start: usize) -> Option<usize> {
    let mut look = current_start;
    loop {
        let prev = prev_line_start(text, look)?;
        if prev == look {
            return None;
        }
        let end = line_end_at(text, prev);
        if !line_is_trivia(&text[prev..end]) {
            return Some(prev);
        }
        look = prev;
    }
}

fn may_join_previous(text: &str, prev: usize, start: usize, lang: ExecLang) -> bool {
    let prev_end = line_end_at(text, prev);
    if chunk_is_incomplete(&text[prev..prev_end], lang) {
        return true;
    }
    match lang {
        ExecLang::Python => {
            let cur_end = line_end_at(text, start);
            let prev_indent = line_indent(text, prev);
            let cur_indent = line_indent(text, start);
            if cur_indent > 0 && prev_indent >= cur_indent {
                return true;
            }
            if python_opens_block(&text[prev..prev_end]) && cur_indent > prev_indent {
                return true;
            }
            python_suite_continuer(&text[start..cur_end]) && cur_indent <= prev_indent
        }
        ExecLang::R => {
            let cur_end = line_end_at(text, start);
            line_starts_with_keyword(&text[start..cur_end], "else")
        }
        ExecLang::Generic => false,
    }
}

fn expand_statement_end(text: &str, start: usize, lang: ExecLang) -> usize {
    let mut end = line_end_at(text, start);
    loop {
        if !should_extend_forward(text, start, end, lang) {
            break;
        }
        if end >= text.len() || text.as_bytes()[end] != b'\n' {
            break;
        }
        let next_end = line_end_at(text, end + 1);
        if next_end == end {
            break;
        }
        end = next_end;
    }
    end
}

fn should_extend_forward(text: &str, start: usize, end: usize, lang: ExecLang) -> bool {
    if chunk_is_incomplete(&text[start..end], lang) {
        return true;
    }
    match lang {
        ExecLang::Python => python_suite_continues(text, start, end),
        ExecLang::R => next_nontrivia_line(text, end).is_some_and(|(line_start, line_end)| {
            line_starts_with_keyword(&text[line_start..line_end], "else")
        }),
        ExecLang::Generic => false,
    }
}

fn python_suite_continues(text: &str, start: usize, end: usize) -> bool {
    let Some((next_start, next_end)) = next_nontrivia_line(text, end) else {
        return false;
    };
    let start_indent = line_indent(text, start);
    let next_indent = line_indent(text, next_start);
    let next_line = &text[next_start..next_end];
    if python_suite_continuer(next_line) && next_indent <= start_indent {
        return true;
    }
    next_indent > start_indent && python_chunk_has_block(&text[start..end])
}

fn python_chunk_has_block(chunk: &str) -> bool {
    chunk.split('\n').any(python_opens_block)
}

fn python_opens_block(line: &str) -> bool {
    let scan = scan_chunk(line, ExecLang::Python);
    !scan.unbalanced && scan.ends_with_colon
}

fn python_suite_continuer(line: &str) -> bool {
    let trimmed = line.trim_start();
    line_starts_with_keyword(trimmed, "else")
        || line_starts_with_keyword(trimmed, "elif")
        || line_starts_with_keyword(trimmed, "except")
        || line_starts_with_keyword(trimmed, "finally")
}

fn line_starts_with_keyword(line: &str, keyword: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix(keyword) else {
        return false;
    };
    rest.is_empty()
        || rest.starts_with(|ch: char| ch.is_whitespace() || ch == ':' || ch == '(' || ch == '#')
}

fn chunk_is_incomplete(code: &str, lang: ExecLang) -> bool {
    let scan = scan_chunk(code, lang);
    if scan.unbalanced || scan.trailing_continuation {
        return true;
    }
    match lang {
        ExecLang::R => ends_with_r_control(code),
        ExecLang::Python => scan.ends_with_colon,
        ExecLang::Generic => false,
    }
}

struct ChunkScan {
    unbalanced: bool,
    trailing_continuation: bool,
    ends_with_colon: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StringKind {
    None,
    Single,
    Double,
    Backtick,
    TripleSingle,
    TripleDouble,
}

fn scan_chunk(code: &str, lang: ExecLang) -> ChunkScan {
    let chars: Vec<char> = code.chars().collect();
    let mut parens = 0i32;
    let mut brackets = 0i32;
    let mut braces = 0i32;
    let mut string = StringKind::None;
    let mut escape = false;
    let mut line_sig = String::new();
    let mut last_sig = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if string != StringKind::None {
            if escape {
                escape = false;
                i += 1;
                continue;
            }
            let triple = matches!(string, StringKind::TripleSingle | StringKind::TripleDouble);
            if ch == '\\' && !triple {
                escape = true;
                i += 1;
                continue;
            }
            if triple {
                let q = if string == StringKind::TripleDouble {
                    '"'
                } else {
                    '\''
                };
                if ch == q && chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
                    string = StringKind::None;
                    line_sig.push('"');
                    i += 3;
                    continue;
                }
            } else if matches_string_close(string, ch) {
                string = StringKind::None;
                line_sig.push('"');
            }
            i += 1;
            continue;
        }
        if ch == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '\n' {
            if !line_sig.is_empty() {
                last_sig.clone_from(&line_sig);
            }
            line_sig.clear();
            i += 1;
            continue;
        }
        if ch == '\'' || ch == '"' || (ch == '`' && lang == ExecLang::R) {
            if lang == ExecLang::Python {
                let q = ch;
                if chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q) {
                    string = if q == '"' {
                        StringKind::TripleDouble
                    } else {
                        StringKind::TripleSingle
                    };
                    i += 3;
                    continue;
                }
            }
            string = match ch {
                '\'' => StringKind::Single,
                '`' => StringKind::Backtick,
                _ => StringKind::Double,
            };
            i += 1;
            continue;
        }
        match ch {
            '(' => parens += 1,
            ')' => parens -= 1,
            '[' => brackets += 1,
            ']' => brackets -= 1,
            '{' => braces += 1,
            '}' => braces -= 1,
            _ => {}
        }
        if !ch.is_whitespace() {
            line_sig.push(ch);
        }
        i += 1;
    }
    if !line_sig.is_empty() {
        last_sig = line_sig;
    }
    ChunkScan {
        unbalanced: string != StringKind::None || parens > 0 || brackets > 0 || braces > 0,
        trailing_continuation: line_has_continuation(&last_sig, lang),
        ends_with_colon: last_sig.ends_with(':'),
    }
}

fn matches_string_close(kind: StringKind, ch: char) -> bool {
    match kind {
        StringKind::Single => ch == '\'',
        StringKind::Double => ch == '"',
        StringKind::Backtick => ch == '`',
        StringKind::None | StringKind::TripleSingle | StringKind::TripleDouble => false,
    }
}

fn line_has_continuation(sig: &str, lang: ExecLang) -> bool {
    let t = sig.trim_end();
    if t.is_empty() {
        return false;
    }
    if lang == ExecLang::Python && t.ends_with('\\') {
        return true;
    }
    if t.ends_with('%') {
        return true;
    }
    const MULTI: &[&str] = &[
        "%>%", "%<>%", "%T>%", "%$%", "->>", "<<-", "<-", "->", ":::", "::", "&&", "||", "==",
        "!=", "<=", ">=", "**", "//",
    ];
    if MULTI.iter().any(|op| t.ends_with(op)) {
        return true;
    }
    match t.chars().last() {
        Some(
            ',' | '+' | '*' | '/' | '^' | '~' | '$' | '@' | '&' | '|' | '=' | '<' | '>' | '!' | '-',
        ) => true,
        Some(':') => lang != ExecLang::Python,
        Some('.') => lang == ExecLang::Python && python_dot_continuation(t),
        _ => false,
    }
}

fn python_dot_continuation(t: &str) -> bool {
    let Some(before) = t.strip_suffix('.') else {
        return false;
    };
    before.chars().last().is_some_and(|ch| {
        ch.is_ascii_alphabetic() || ch == '_' || ch == ')' || ch == ']' || ch == '}'
    })
}

fn ends_with_r_control(code: &str) -> bool {
    let skeleton = r_skeleton(code);
    let s = skeleton.trim_end();
    if s.is_empty() {
        return false;
    }
    if keyword_suffix(s, "repeat") || keyword_suffix(s, "else") {
        return true;
    }
    if !s.ends_with(')') {
        return false;
    }
    let Some(open) = matching_open_paren(s) else {
        return false;
    };
    let before = s[..open].trim_end();
    keyword_suffix(before, "if")
        || keyword_suffix(before, "for")
        || keyword_suffix(before, "while")
        || keyword_suffix(before, "function")
}

fn keyword_suffix(s: &str, keyword: &str) -> bool {
    let Some(prefix) = s.strip_suffix(keyword) else {
        return false;
    };
    prefix.is_empty()
        || prefix
            .chars()
            .last()
            .is_some_and(|ch| !ch.is_ascii_alphanumeric() && ch != '.' && ch != '_')
}

fn matching_open_paren(s: &str) -> Option<usize> {
    if !s.ends_with(')') {
        return None;
    }
    let mut depth = 0i32;
    for (i, ch) in s.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn r_skeleton(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    let mut out = String::new();
    let mut string = None;
    let mut escape = false;
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        if let Some(quote) = string {
            if escape {
                escape = false;
                i += 1;
                continue;
            }
            if ch == '\\' {
                escape = true;
                i += 1;
                continue;
            }
            if ch == quote {
                string = None;
                out.push('"');
            }
            i += 1;
            continue;
        }
        if ch == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '\'' || ch == '"' || ch == '`' {
            string = Some(ch);
            out.push('"');
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_selection_maps_utf16_offsets_and_reports_selected_lines() {
        let text = "第一行\nplot(1:3)\n🙂 done\n";
        let start = "第一行\n".encode_utf16().count() as u32;
        let end = "第一行\nplot(1:3)\n".encode_utf16().count() as u32;
        let selection = source_selection(text, start, end);
        assert_eq!(selection.before, "第一行\n");
        assert_eq!(selection.selected, "plot(1:3)\n");
        assert_eq!(selection.after, "🙂 done\n");
        assert_eq!((selection.start_line, selection.end_line), (2, 2));

        // The emoji occupies two UTF-16 code units but remains one Rust char.
        let emoji_start = "第一行\nplot(1:3)\n".encode_utf16().count() as u32;
        let emoji_end = emoji_start + "🙂".encode_utf16().count() as u32;
        assert_eq!(
            source_selection(text, emoji_start, emoji_end).selected,
            "🙂"
        );
    }

    #[test]
    fn source_execution_runs_selection_or_current_line_and_advances() {
        let text = "x <- 1\n绘图 <- x + 1\nprint(绘图)\n";
        let selected_start = "x <- 1\n".encode_utf16().count() as u32;
        let selected_end = selected_start + "绘图 <- x + 1".encode_utf16().count() as u32;
        assert_eq!(
            source_execution(text, selected_start, selected_end, "r"),
            Some(SourceExecution {
                code: "绘图 <- x + 1".into(),
                next_caret_utf16: None,
            })
        );

        let caret = selected_start + 2;
        assert_eq!(
            source_execution(text, caret, caret, "r"),
            Some(SourceExecution {
                code: "绘图 <- x + 1".into(),
                next_caret_utf16: Some("x <- 1\n绘图 <- x + 1\n".encode_utf16().count() as u32),
            })
        );
    }

    #[test]
    fn source_execution_runs_complete_r_and_python_statements() {
        let r = "sce <- FindVariableFeatures(sce,\n  verbose = FALSE)\nplot(1)\n";
        let expected = "sce <- FindVariableFeatures(sce,\n  verbose = FALSE)";
        let next = "sce <- FindVariableFeatures(sce,\n  verbose = FALSE)\n"
            .encode_utf16()
            .count() as u32;
        assert_eq!(
            source_execution(r, 0, 0, "r"),
            Some(SourceExecution {
                code: expected.into(),
                next_caret_utf16: Some(next),
            })
        );
        let continuation = "sce <- FindVariableFeatures(sce,\n  "
            .encode_utf16()
            .count() as u32;
        assert_eq!(
            source_execution(r, continuation, continuation, "r").map(|execution| execution.code),
            Some(expected.into())
        );

        let piped = "df %>%\n  filter(x > 1)\nnext <- 1\n";
        assert_eq!(
            source_execution(piped, 0, 0, "r").map(|execution| execution.code),
            Some("df %>%\n  filter(x > 1)".into())
        );

        let if_else = "if (x)\n  y\nelse\n  z\nplot(1)\n";
        assert_eq!(
            source_execution(if_else, 0, 0, "r").map(|execution| execution.code),
            Some("if (x)\n  y\nelse\n  z".into())
        );
        let else_body = "if (x)\n  y\nelse\n  ".encode_utf16().count() as u32;
        assert_eq!(
            source_execution(if_else, else_body, else_body, "r").map(|execution| execution.code),
            Some("if (x)\n  y\nelse\n  z".into())
        );

        // An explicit one-line selection is still sent as-is, even if incomplete.
        let first_line = "sce <- FindVariableFeatures(sce,".encode_utf16().count() as u32;
        assert_eq!(
            source_execution(r, 0, first_line, "r").map(|execution| execution.code),
            Some("sce <- FindVariableFeatures(sce,".into())
        );

        let py = "def foo():\n    if x:\n        return 1\n    return 2\nx = 1\n";
        assert_eq!(
            source_execution(py, 0, 0, "python").map(|execution| execution.code),
            Some("def foo():\n    if x:\n        return 1\n    return 2".into())
        );
        let inner = "def foo():\n    if x:\n        ".encode_utf16().count() as u32;
        assert_eq!(
            source_execution(py, inner, inner, "python").map(|execution| execution.code),
            Some("def foo():\n    if x:\n        return 1\n    return 2".into())
        );
        let if_else_py = "if x:\n    y\nelse:\n    z\nnext = 1\n";
        assert_eq!(
            source_execution(if_else_py, 0, 0, "python").map(|execution| execution.code),
            Some("if x:\n    y\nelse:\n    z".into())
        );

        let with_blank = "def foo():\n    x = 1\n\n    y = 2\nnext = 1\n";
        let y_caret = "def foo():\n    x = 1\n\n    ".encode_utf16().count() as u32;
        assert_eq!(
            source_execution(with_blank, y_caret, y_caret, "python")
                .map(|execution| execution.code),
            Some("def foo():\n    x = 1\n\n    y = 2".into())
        );

        let with_comment = "x <- 1\n# skip\ny <- 2\n";
        assert_eq!(
            source_execution(with_comment, 0, 0, "r"),
            Some(SourceExecution {
                code: "x <- 1".into(),
                next_caret_utf16: Some("x <- 1\n# skip\n".encode_utf16().count() as u32),
            })
        );

        let assigned = "in_dir <- \"data\"\nplot(1)\n";
        assert_eq!(
            source_execution(assigned, 0, 0, "r").map(|execution| execution.code),
            Some("in_dir <- \"data\"".into())
        );
    }

    #[test]
    fn offsets_convert_between_utf16_and_bytes_without_splitting_a_character() {
        let text = "a第🙂b";
        assert_eq!(utf16_to_byte_index(text, 0), 0);
        assert_eq!(utf16_to_byte_index(text, 1), 1);
        assert_eq!(utf16_to_byte_index(text, 2), 4);
        // Half of the emoji's surrogate pair stays before the emoji.
        assert_eq!(utf16_to_byte_index(text, 3), 4);
        assert_eq!(utf16_to_byte_index(text, 4), 8);
        assert_eq!(utf16_to_byte_index(text, 99), text.len());
        assert_eq!(byte_to_utf16_index(text, 8), 4);
        assert_eq!(byte_to_utf16_index(text, 99), 5);
    }
}
