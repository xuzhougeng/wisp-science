//! Sequence files as the WebView's sequence viewers show them: an
//! alignment (Clustal, Stockholm, aligned FASTA) as a grid of residues
//! coloured by kind, and a FASTA file the same way, one record per row.
//! Reading is pure, so it is tested here; the Files preview draws it.
use std::ops::Range;

use crate::i18n::{fill, tr};

/// The most rows and columns drawn; a larger file says what is shown.
pub const MAX_ROWS: usize = 400;
pub const MAX_COLUMNS: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alignment {
    pub names: Vec<String>,
    /// Each row's residues, gaps included, cut at [`MAX_COLUMNS`].
    pub rows: Vec<String>,
    /// Records in the file, and the longest row before it was cut.
    pub total_rows: usize,
    pub width: usize,
    /// Nucleotides rather than amino acids.
    pub nucleic: bool,
}

/// What a residue is, for its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Residue {
    Gap,
    Hydrophobic,
    Aromatic,
    Positive,
    Negative,
    Polar,
    Glycine,
    Proline,
    Cysteine,
    Adenine,
    Cytosine,
    Guanine,
    Thymine,
    Other,
}

fn extension(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, kind)| kind.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Alignment formats, as the WebView's `file_kind` names them.
pub fn is_alignment_file(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "aln" | "clustal" | "clustalw" | "sto" | "stockholm" | "stk" | "afa" | "mfa"
    )
}

/// Plain FASTA files.
pub fn is_fasta_file(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "fasta" | "fa" | "fas" | "fna" | "faa" | "ffn" | "frn"
    )
}

/// The sequences of an alignment or FASTA file, or `None` when the text
/// is not one.
pub fn parse(path: &str, text: &str) -> Option<Alignment> {
    if !is_alignment_file(path) && !is_fasta_file(path) {
        return None;
    }
    let first = text.lines().find(|line| !line.trim().is_empty())?.trim();
    let records = if first.starts_with('>') {
        fasta(text)
    } else if first.starts_with("# STOCKHOLM") {
        blocks(text, |line| line.starts_with('#') || line.starts_with("//"))
    } else if first.to_ascii_uppercase().starts_with("CLUSTAL") || first.starts_with("MUSCLE") {
        blocks(text, |line| {
            line.to_ascii_uppercase().starts_with("CLUSTAL") || line.starts_with("MUSCLE")
        })
    } else {
        return None;
    };
    if records.is_empty() {
        return None;
    }
    let total_rows = records.len();
    let width = records.iter().map(|(_, row)| row.len()).max().unwrap_or(0);
    let shown: Vec<_> = records.into_iter().take(MAX_ROWS).collect();
    let nucleic = nucleic(shown.iter().map(|(_, row)| row.as_str()));
    let (names, rows) = shown
        .into_iter()
        .map(|(name, row)| {
            let mut row = row;
            row.truncate(MAX_COLUMNS);
            (name, row)
        })
        .unzip();
    Some(Alignment {
        names,
        rows,
        total_rows,
        width,
        nucleic,
    })
}

/// `>name description` records with their sequence lines joined; anything
/// but residue letters, gaps and stops is dropped.
fn fasta(text: &str) -> Vec<(String, String)> {
    let mut records: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('>') {
            let name = header.split_whitespace().next().unwrap_or_default();
            records.push((name.to_owned(), String::new()));
        } else if let Some((_, row)) = records.last_mut() {
            row.extend(
                line.chars()
                    .filter(|c| c.is_ascii_alphabetic() || matches!(c, '-' | '.' | '*')),
            );
        }
    }
    records
}

/// Interleaved `name residues` lines, in blocks; a name seen again carries
/// on its row. `skip` marks header and annotation lines; a line that starts
/// with a space is a consensus line.
fn blocks(text: &str, skip: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut records: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with(char::is_whitespace) || skip(line) {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(residues)) = (parts.next(), parts.next()) else {
            continue;
        };
        match records.iter_mut().find(|(kept, _)| kept == name) {
            Some((_, row)) => row.push_str(residues),
            None => records.push((name.to_owned(), residues.to_owned())),
        }
    }
    records
}

/// Whether the rows read as DNA or RNA: nearly all their letters are
/// A, C, G, T, U or N.
fn nucleic<'a>(rows: impl Iterator<Item = &'a str>) -> bool {
    let (mut bases, mut letters) = (0usize, 0usize);
    for c in rows.flat_map(str::chars).filter(char::is_ascii_alphabetic) {
        letters += 1;
        if matches!(c.to_ascii_uppercase(), 'A' | 'C' | 'G' | 'T' | 'U' | 'N') {
            bases += 1;
        }
    }
    letters > 0 && bases * 10 >= letters * 9
}

/// What one residue is.
pub fn classify(residue: char, nucleic: bool) -> Residue {
    let c = residue.to_ascii_uppercase();
    if matches!(c, '-' | '.') {
        return Residue::Gap;
    }
    if nucleic {
        return match c {
            'A' => Residue::Adenine,
            'C' => Residue::Cytosine,
            'G' => Residue::Guanine,
            'T' | 'U' => Residue::Thymine,
            _ => Residue::Other,
        };
    }
    match c {
        'A' | 'I' | 'L' | 'M' | 'V' => Residue::Hydrophobic,
        'F' | 'W' | 'Y' | 'H' => Residue::Aromatic,
        'K' | 'R' => Residue::Positive,
        'D' | 'E' => Residue::Negative,
        'S' | 'T' | 'N' | 'Q' => Residue::Polar,
        'G' => Residue::Glycine,
        'P' => Residue::Proline,
        'C' => Residue::Cysteine,
        _ => Residue::Other,
    }
}

/// A row as runs of one kind of residue, for colouring.
pub fn runs(row: &str, nucleic: bool) -> Vec<(Range<usize>, Residue)> {
    let mut runs: Vec<(Range<usize>, Residue)> = Vec::new();
    for (at, residue) in row.char_indices() {
        let kind = classify(residue, nucleic);
        let end = at + residue.len_utf8();
        match runs.last_mut() {
            Some((range, last)) if *last == kind && range.end == at => range.end = end,
            _ => runs.push((at..end, kind)),
        }
    }
    runs
}

/// The line over the grid: what was read, and what of it is shown.
pub fn note(alignment: &Alignment) -> String {
    let kind = if alignment.nucleic {
        tr("nucleotides")
    } else {
        tr("amino acids")
    };
    let mut line = fill(
        tr("{} sequences · {} columns · {}"),
        &[&alignment.total_rows, &alignment.width, &kind],
    );
    if alignment.total_rows > alignment.rows.len() || alignment.width > MAX_COLUMNS {
        line.push_str(&fill(
            tr(" · showing the first {} rows and {} columns"),
            &[&alignment.rows.len(), &alignment.width.min(MAX_COLUMNS)],
        ));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_alignment_formats_are_read_into_rows() {
        let clustal = "CLUSTAL W (1.83) multiple sequence alignment\n\n\
            seq1      MKV-LA  6\n\
            seq2      MRVQLA  6\n\
            \x20         *:* **\n\n\
            seq1      GE\n\
            seq2      GD\n";
        let read = parse("x.aln", clustal).unwrap();
        assert_eq!(read.names, ["seq1", "seq2"]);
        assert_eq!(read.rows, ["MKV-LAGE", "MRVQLAGD"]);
        assert_eq!((read.total_rows, read.width), (2, 8));
        assert!(!read.nucleic);

        let stockholm = "# STOCKHOLM 1.0\n#=GF ID test\nA1 ACGU\nB2 ACG-\n#=GC SS_cons ....\n//\n";
        let read = parse("x.sto", stockholm).unwrap();
        assert_eq!(read.rows, ["ACGU", "ACG-"]);
        assert!(read.nucleic);

        let fasta = ">one first record\nACGT\nAC\n>two\nAC-TAC\n";
        let read = parse("reads.fa", fasta).unwrap();
        assert_eq!(read.names, ["one", "two"]);
        assert_eq!(read.rows, ["ACGTAC", "AC-TAC"]);
        assert!(read.nucleic);
        assert_eq!(note(&read), "2 sequences · 6 columns · nucleotides");

        // Not a sequence file, or not what its name says.
        assert_eq!(parse("notes.txt", fasta), None);
        assert_eq!(parse("x.aln", "just words"), None);
    }

    #[test]
    fn a_large_file_is_cut_and_says_so() {
        let mut text = String::new();
        for index in 0..MAX_ROWS + 5 {
            text.push_str(&format!(">s{index}\n{}\n", "M".repeat(MAX_COLUMNS + 10)));
        }
        let read = parse("big.faa", &text).unwrap();
        assert_eq!(read.rows.len(), MAX_ROWS);
        assert_eq!(read.rows[0].len(), MAX_COLUMNS);
        assert_eq!(read.total_rows, MAX_ROWS + 5);
        assert!(note(&read).contains("showing the first 400 rows and 5000 columns"));
    }

    #[test]
    fn residues_are_coloured_by_kind_in_runs() {
        assert_eq!(classify('-', false), Residue::Gap);
        assert_eq!(classify('k', false), Residue::Positive);
        assert_eq!(classify('W', false), Residue::Aromatic);
        assert_eq!(classify('C', false), Residue::Cysteine);
        assert_eq!(classify('C', true), Residue::Cytosine);
        assert_eq!(classify('U', true), Residue::Thymine);
        assert_eq!(
            runs("KR-DE", false),
            vec![
                (0..2, Residue::Positive),
                (2..3, Residue::Gap),
                (3..5, Residue::Negative),
            ]
        );
    }
}
