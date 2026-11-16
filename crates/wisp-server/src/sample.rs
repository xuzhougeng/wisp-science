//! A sample project to try Wisp on: a small folder with made-up data, a
//! script and a note on what to ask. It is an ordinary project in a folder
//! the user chooses; nothing in it is real data.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

const NAME: &str = "Wisp sample project";

const README: &str = "\
# Wisp sample project

A small, made-up experiment to try Wisp on. Nothing here is real data.

Twelve samples of a pretend RNA-seq run are described in
`data/samples.csv`, and `data/counts.csv` has read counts for eight genes
in each of them. Half the samples are `control`, half `treated`.

Things to ask:

- Which genes differ most between control and treated? Show the numbers.
- Plot the counts of the top gene by condition and save the figure.
- Is any sample an outlier? How did you decide?
- Run `scripts/summary.py` and explain what it prints.

Wisp asks before it writes files or runs commands. This folder is yours:
change or delete anything in it.
";

const SAMPLES: &str = "\
sample,condition,batch
S01,control,A
S02,control,A
S03,control,A
S04,control,B
S05,control,B
S06,control,B
S07,treated,A
S08,treated,A
S09,treated,A
S10,treated,B
S11,treated,B
S12,treated,B
";

const COUNTS: &str = "\
gene,S01,S02,S03,S04,S05,S06,S07,S08,S09,S10,S11,S12
GENE_A,120,131,118,125,140,122,410,395,430,402,388,415
GENE_B,980,1010,995,1002,970,1015,990,1005,985,1012,998,1001
GENE_C,45,52,48,50,47,49,12,15,11,14,13,10
GENE_D,300,310,295,305,298,312,305,299,308,301,296,3100
GENE_E,15,18,14,16,17,15,16,14,18,15,17,16
GENE_F,640,655,630,648,660,642,820,835,810,828,815,840
GENE_G,210,205,215,208,212,207,150,145,155,148,152,149
GENE_H,75,80,72,78,74,77,76,79,73,78,75,77
";

const SCRIPT: &str = "\
\"\"\"Mean count per gene in each condition, from the sample data.\"\"\"
import csv
from collections import defaultdict
from pathlib import Path

root = Path(__file__).resolve().parent.parent
conditions = {}
with open(root / \"data\" / \"samples.csv\", newline=\"\") as handle:
    for row in csv.DictReader(handle):
        conditions[row[\"sample\"]] = row[\"condition\"]

with open(root / \"data\" / \"counts.csv\", newline=\"\") as handle:
    for row in csv.DictReader(handle):
        totals = defaultdict(list)
        for sample, condition in conditions.items():
            totals[condition].append(int(row[sample]))
        means = {name: sum(values) / len(values) for name, values in totals.items()}
        print(row[\"gene\"], *(f\"{name}={mean:.1f}\" for name, mean in sorted(means.items())))
";

/// The files of the sample, by their path in its folder.
const FILES: [(&str, &str); 4] = [
    ("README.md", README),
    ("data/samples.csv", SAMPLES),
    ("data/counts.csv", COUNTS),
    ("scripts/summary.py", SCRIPT),
];

/// A folder under `parent` that is not there yet: the sample's name, or
/// that name with a number.
fn free_folder(parent: &Path) -> Result<PathBuf> {
    for number in 1..=99 {
        let name = match number {
            1 => NAME.to_owned(),
            n => format!("{NAME} {n}"),
        };
        let folder = parent.join(name);
        if !folder.exists() {
            return Ok(folder);
        }
    }
    bail!("There are already many sample projects in that folder; choose another")
}

/// Writes the sample into a new folder under `parent` and returns the
/// folder. Nothing that exists is written over.
pub(crate) fn write(parent: &Path) -> Result<PathBuf> {
    let folder = free_folder(parent)?;
    for (path, text) in FILES {
        let file = folder.join(path);
        let into = file.parent().context("a sample file has a folder")?;
        std::fs::create_dir_all(into)
            .with_context(|| format!("Could not make {}", into.display()))?;
        std::fs::write(&file, text)
            .with_context(|| format!("Could not write {}", file.display()))?;
    }
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_is_written_whole_into_a_folder_of_its_own() {
        let temp = tempfile::tempdir().unwrap();
        let first = write(temp.path()).unwrap();
        assert_eq!(first, temp.path().join(NAME));
        for (path, text) in FILES {
            assert_eq!(std::fs::read_to_string(first.join(path)).unwrap(), text);
        }
        // What is there is never written over: a second one goes beside it.
        std::fs::write(first.join("README.md"), "mine now").unwrap();
        let second = write(temp.path()).unwrap();
        assert_eq!(second, temp.path().join(format!("{NAME} 2")));
        assert_eq!(
            std::fs::read_to_string(first.join("README.md")).unwrap(),
            "mine now"
        );
    }

    #[test]
    fn the_made_up_data_is_consistent_with_itself() {
        let samples: Vec<&str> = SAMPLES
            .lines()
            .skip(1)
            .map(|line| line.split(',').next().unwrap())
            .collect();
        assert_eq!(samples.len(), 12);
        let mut rows = COUNTS.lines();
        let header: Vec<&str> = rows.next().unwrap().split(',').skip(1).collect();
        // Every sample has a column, in the same order.
        assert_eq!(header, samples);
        for row in rows {
            let cells: Vec<&str> = row.split(',').collect();
            assert_eq!(cells.len(), 13, "{row}");
            assert!(
                cells[1..].iter().all(|cell| cell.parse::<u32>().is_ok()),
                "{row}"
            );
        }
        assert!(README.contains("Nothing here is real data"));
    }
}
