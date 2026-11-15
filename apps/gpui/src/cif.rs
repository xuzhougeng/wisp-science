//! Reading a CIF file (a crystallographic information file, or the mmCIF a
//! protein structure comes in) into its single items and loops, for the
//! structure preview. Only the first data block is read; tag names are
//! lower case with `.` as `_`, so `_atom_site.Cartn_x` and
//! `_atom_site_Cartn_x` are the same name.
use std::collections::HashMap;

#[derive(Debug, Default, PartialEq)]
pub struct Loop {
    pub names: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Loop {
    /// The column a name is in.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|kept| kept == name)
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Cif {
    pub items: HashMap<String, String>,
    pub loops: Vec<Loop>,
}

impl Cif {
    /// The loop that holds `name`.
    pub fn loop_with(&self, name: &str) -> Option<&Loop> {
        self.loops.iter().find(|table| table.column(name).is_some())
    }

    /// A single item as a number, without its standard uncertainty:
    /// `10.123(4)` is 10.123.
    pub fn number(&self, name: &str) -> Option<f64> {
        number(self.items.get(name)?)
    }
}

/// A value as a number, without its standard uncertainty; `?` and `.` (not
/// known, not applicable) are none.
pub fn number(value: &str) -> Option<f64> {
    let value = value.split('(').next().unwrap_or_default().trim();
    value.parse().ok()
}

#[derive(Debug, PartialEq)]
enum Token {
    /// A word as written: a keyword, a tag or a plain value.
    Bare(String),
    /// A quoted string or text field, which is always a value.
    Quoted(String),
}

impl Token {
    fn text(self) -> String {
        match self {
            Token::Bare(text) | Token::Quoted(text) => text,
        }
    }

    fn tag(&self) -> Option<String> {
        match self {
            Token::Bare(text) if text.starts_with('_') => {
                Some(text.to_ascii_lowercase().replace('.', "_"))
            }
            _ => None,
        }
    }

    /// `loop_`, `data_…`, `save_…`, `global_` or `stop_`.
    fn keyword(&self) -> Option<String> {
        let Token::Bare(text) = self else {
            return None;
        };
        let lower = text.to_ascii_lowercase();
        ["loop_", "data_", "save_", "global_", "stop_"]
            .iter()
            .any(|keyword| lower.starts_with(keyword))
            .then_some(lower)
    }
}

fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        // A text field runs from a line that starts with `;` to the next.
        if let Some(first) = line.strip_prefix(';') {
            let mut field = vec![first.to_owned()];
            for next in lines.by_ref() {
                if next.starts_with(';') {
                    break;
                }
                field.push(next.to_owned());
            }
            tokens.push(Token::Quoted(field.join("\n").trim().to_owned()));
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            let ch = chars[at];
            if ch.is_whitespace() {
                at += 1;
                continue;
            }
            if ch == '#' {
                break;
            }
            if ch == '\'' || ch == '"' {
                // A quote closes only when whitespace or the line's end
                // follows it, so `'it's'` reads as `it's`.
                let mut end = at + 1;
                while end < chars.len()
                    && !(chars[end] == ch
                        && chars.get(end + 1).is_none_or(|next| next.is_whitespace()))
                {
                    end += 1;
                }
                tokens.push(Token::Quoted(
                    chars[at + 1..end.min(chars.len())].iter().collect(),
                ));
                at = end + 1;
                continue;
            }
            let start = at;
            while at < chars.len() && !chars[at].is_whitespace() {
                at += 1;
            }
            tokens.push(Token::Bare(chars[start..at].iter().collect()));
        }
    }
    tokens
}

/// The first data block of `text`.
pub fn read(text: &str) -> Cif {
    let mut cif = Cif::default();
    let mut tokens = tokens(text).into_iter().peekable();
    let mut blocks = 0;
    while let Some(token) = tokens.next() {
        if let Some(keyword) = token.keyword() {
            if keyword.starts_with("data_") {
                blocks += 1;
                if blocks > 1 {
                    break;
                }
                continue;
            }
            if keyword != "loop_" {
                continue;
            }
            let mut table = Loop::default();
            while let Some(name) = tokens.peek().and_then(Token::tag) {
                table.names.push(name);
                tokens.next();
            }
            let mut values = Vec::new();
            while let Some(next) = tokens.peek() {
                if next.tag().is_some() || next.keyword().is_some() {
                    break;
                }
                values.push(tokens.next().unwrap().text());
            }
            if !table.names.is_empty() {
                // A last row cut short is not a row.
                table.rows = values
                    .chunks_exact(table.names.len())
                    .map(<[String]>::to_vec)
                    .collect();
                cif.loops.push(table);
            }
            continue;
        }
        if let Some(name) = token.tag() {
            let value = tokens
                .next_if(|next| next.tag().is_none() && next.keyword().is_none())
                .map(Token::text)
                .unwrap_or_default();
            cif.items.insert(name, value);
        }
    }
    cif
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_loops_quotes_and_text_fields_are_read() {
        let text = "data_first\n\
            _cell.length_a   10.123(4)\n\
            _chemical_name 'it's quoted'  # a comment\n\
            _publ_section_title\n;\nA long\ntitle\n;\n\
            loop_\n_atom_site_label\n_atom_site_fract_x\n_atom_site_note\n\
            C1 0.5 \"one two\"\nO1 0.25 ?\nN1 0.1\n\
            _after 1\n\
            data_second\n_cell_length_a 99\n";
        let cif = read(text);
        assert_eq!(cif.number("_cell_length_a"), Some(10.123));
        assert_eq!(cif.items["_chemical_name"], "it's quoted");
        assert_eq!(cif.items["_publ_section_title"], "A long\ntitle");
        assert_eq!(cif.items["_after"], "1");
        let sites = cif.loop_with("_atom_site_fract_x").unwrap();
        assert_eq!(sites.column("_atom_site_note"), Some(2));
        // The third row is cut short, so it is not read.
        assert_eq!(
            sites.rows,
            vec![
                vec!["C1".to_string(), "0.5".into(), "one two".into()],
                vec!["O1".to_string(), "0.25".into(), "?".into()],
            ]
        );
        assert_eq!(number("?"), None);
    }
}
