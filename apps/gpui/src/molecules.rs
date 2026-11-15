//! Molecules and structures as the WebView's 3Dmol viewer shows them,
//! drawn flat: a small molecule (MOL, SDF, MOL2) as its atoms and bonds,
//! and a protein or nucleic acid (PDB) as the trace of its chains, turned
//! by dragging. Reading and projecting are pure, so they are tested here;
//! the Files preview draws the result.
use crate::i18n::{fill, tr};

/// The most atoms read; a larger file shows its first ones and says so.
pub const MAX_ATOMS: usize = 20_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Atom {
    pub element: String,
    pub position: [f32; 3],
    /// The chain a residue's atom belongs to; empty for a small molecule.
    pub chain: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Structure {
    pub atoms: Vec<Atom>,
    /// Pairs of atom indexes.
    pub bonds: Vec<(usize, usize)>,
    /// Drawn as a chain trace rather than as atoms and bonds.
    pub trace: bool,
    /// Atoms in the file before it was cut at [`MAX_ATOMS`].
    pub total_atoms: usize,
}

fn extension(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(_, kind)| kind.to_ascii_lowercase())
        .unwrap_or_default()
}

/// The molecule or structure in `text` (by the file's kind: MOL, SDF,
/// MOL2 or PDB, as the WebView's `file_kind` names them), or `None` when it
/// cannot be read.
pub fn parse(path: &str, text: &str) -> Option<Structure> {
    let read = match extension(path).as_str() {
        "sdf" | "mol" => molfile(text),
        "mol2" => mol2(text),
        "pdb" => pdb(text),
        _ => None,
    }?;
    (!read.atoms.is_empty()).then_some(read)
}

fn field(line: &str, range: std::ops::Range<usize>) -> &str {
    line.get(range.start.min(line.len())..range.end.min(line.len()))
        .unwrap_or_default()
        .trim()
}

/// The first record of an MDL molfile or SD file (V2000).
fn molfile(text: &str) -> Option<Structure> {
    let lines: Vec<&str> = text.lines().collect();
    let counts = lines.get(3)?;
    if counts.contains("V3000") {
        return None;
    }
    let atoms_count: usize = field(counts, 0..3).parse().ok()?;
    let bonds_count: usize = field(counts, 3..6).parse().ok()?;
    let mut atoms = Vec::new();
    for line in lines.get(4..4 + atoms_count)? {
        let number = |range| field(line, range).parse::<f32>().ok();
        atoms.push(Atom {
            element: field(line, 31..34).to_owned(),
            position: [number(0..10)?, number(10..20)?, number(20..30)?],
            chain: String::new(),
        });
    }
    let mut bonds = Vec::new();
    for line in lines.get(4 + atoms_count..4 + atoms_count + bonds_count)? {
        let index = |range| field(line, range).parse::<usize>().ok();
        let (first, second) = (index(0..3)?, index(3..6)?);
        if (1..=atoms.len()).contains(&first) && (1..=atoms.len()).contains(&second) {
            bonds.push((first - 1, second - 1));
        }
    }
    let total_atoms = atoms.len();
    Some(Structure {
        atoms,
        bonds,
        trace: false,
        total_atoms,
    })
}

/// A Tripos MOL2 file's atoms and bonds.
fn mol2(text: &str) -> Option<Structure> {
    let mut section = "";
    let mut atoms = Vec::new();
    let mut ids = std::collections::HashMap::new();
    let mut bonds = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("@<TRIPOS>") {
            section = match name {
                "ATOM" => "atom",
                "BOND" => "bond",
                _ => "",
            };
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match section {
            "atom" if parts.len() >= 6 => {
                let number = |at: usize| parts[at].parse::<f32>().ok();
                let (Some(x), Some(y), Some(z)) = (number(2), number(3), number(4)) else {
                    continue;
                };
                // "C.ar" is an aromatic carbon.
                let element = parts[5].split('.').next().unwrap_or_default();
                ids.insert(parts[0].to_owned(), atoms.len());
                atoms.push(Atom {
                    element: element.to_owned(),
                    position: [x, y, z],
                    chain: String::new(),
                });
            }
            "bond" if parts.len() >= 3 => {
                if let (Some(&first), Some(&second)) = (ids.get(parts[1]), ids.get(parts[2])) {
                    bonds.push((first, second));
                }
            }
            _ => {}
        }
    }
    let total_atoms = atoms.len();
    Some(Structure {
        atoms,
        bonds,
        trace: false,
        total_atoms,
    })
}

/// A PDB file: a polymer as the trace through each chain's Cα (or a
/// nucleic acid's P) atoms; without one, its atoms bonded by distance.
fn pdb(text: &str) -> Option<Structure> {
    let mut all = Vec::new();
    let mut backbone: Vec<(Atom, i64)> = Vec::new();
    for line in text.lines() {
        if line.starts_with("ENDMDL") {
            // The first model only.
            break;
        }
        if !(line.starts_with("ATOM") || line.starts_with("HETATM")) {
            continue;
        }
        let number = |range| field(line, range).parse::<f32>().ok();
        let (Some(x), Some(y), Some(z)) = (number(30..38), number(38..46), number(46..54)) else {
            continue;
        };
        let name = field(line, 12..16);
        let element = match field(line, 76..78) {
            "" => name
                .trim_start_matches(char::is_numeric)
                .chars()
                .take(1)
                .collect(),
            element => element.to_owned(),
        };
        let atom = Atom {
            element,
            position: [x, y, z],
            chain: field(line, 21..22).to_owned(),
        };
        if line.starts_with("ATOM") && matches!(name, "CA" | "P") {
            let residue = field(line, 22..26).parse::<i64>().unwrap_or_default();
            backbone.push((atom.clone(), residue));
        }
        all.push(atom);
    }
    let total_atoms = all.len();
    if backbone.len() >= 2 {
        let mut bonds = Vec::new();
        for (index, pair) in backbone.windows(2).enumerate() {
            let ((first, first_residue), (second, second_residue)) = (&pair[0], &pair[1]);
            // A chain break, or a gap in the numbering, is not a bond.
            if first.chain == second.chain && second_residue - first_residue <= 1 {
                bonds.push((index, index + 1));
            }
        }
        let atoms = backbone
            .into_iter()
            .take(MAX_ATOMS)
            .map(|(atom, _)| atom)
            .collect::<Vec<_>>();
        bonds.retain(|(first, second)| *first < atoms.len() && *second < atoms.len());
        return Some(Structure {
            atoms,
            bonds,
            trace: true,
            total_atoms,
        });
    }
    let atoms: Vec<Atom> = all.into_iter().take(2000).collect();
    Some(Structure {
        bonds: by_distance(&atoms),
        atoms,
        trace: false,
        total_atoms,
    })
}

/// Bonds between atoms closer than a covalent bond is long.
fn by_distance(atoms: &[Atom]) -> Vec<(usize, usize)> {
    let mut bonds = Vec::new();
    for (first, a) in atoms.iter().enumerate() {
        for (offset, b) in atoms[first + 1..].iter().enumerate() {
            let squared: f32 = (0..3)
                .map(|axis| (a.position[axis] - b.position[axis]).powi(2))
                .sum();
            let hydrogen = a.element == "H" || b.element == "H";
            let limit = if hydrogen { 1.3 } else { 1.9 };
            if squared > 0.01 && squared < limit * limit {
                bonds.push((first, first + 1 + offset));
            }
        }
    }
    bonds
}

/// Points on screen for the atoms, turned by `yaw` and `pitch` (radians)
/// and fitted into `width` × `height` with a margin; with their depth, for
/// drawing the far ones first.
pub fn project(
    structure: &Structure,
    yaw: f32,
    pitch: f32,
    width: f32,
    height: f32,
) -> Vec<(f32, f32, f32)> {
    let count = structure.atoms.len().max(1) as f32;
    let mut centre = [0f32; 3];
    for atom in &structure.atoms {
        for axis in 0..3 {
            centre[axis] += atom.position[axis] / count;
        }
    }
    let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
    let turned: Vec<[f32; 3]> = structure
        .atoms
        .iter()
        .map(|atom| {
            let [x, y, z] = [
                atom.position[0] - centre[0],
                atom.position[1] - centre[1],
                atom.position[2] - centre[2],
            ];
            // Yaw about the vertical axis, then pitch about the horizontal.
            let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
            let (y, z) = (y * cp - z * sp, y * sp + z * cp);
            [x, y, z]
        })
        .collect();
    let reach = turned
        .iter()
        .map(|[x, y, _]| x.abs().max(y.abs()))
        .fold(0f32, f32::max)
        .max(0.5);
    let margin = 16.;
    let scale = ((width.min(height) / 2. - margin) / reach).max(0.1);
    turned
        .into_iter()
        // Screen y grows downwards.
        .map(|[x, y, z]| (width / 2. + x * scale, height / 2. - y * scale, z))
        .collect()
}

/// An element's colour, as a hue and lightness (CPK-like); carbon and
/// unknown elements are left to the foreground.
pub fn element_colour(element: &str) -> Option<(f32, f32)> {
    match element.to_ascii_uppercase().as_str() {
        "N" => Some((0.62, 0.55)),
        "O" => Some((0.0, 0.5)),
        "S" => Some((0.15, 0.5)),
        "P" => Some((0.08, 0.5)),
        "F" | "CL" | "BR" | "I" => Some((0.33, 0.45)),
        "H" => Some((0.0, 0.85)),
        _ => None,
    }
}

/// The line over the drawing.
pub fn note(structure: &Structure) -> String {
    let chains = {
        let mut seen: Vec<&str> = structure
            .atoms
            .iter()
            .map(|atom| atom.chain.as_str())
            .collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    };
    if structure.trace {
        fill(
            tr("{} chains · {} residues traced · drag to turn"),
            &[&chains, &structure.atoms.len()],
        )
    } else if structure.total_atoms > structure.atoms.len() {
        fill(
            tr("The first {} of {} atoms · drag to turn"),
            &[&structure.atoms.len(), &structure.total_atoms],
        )
    } else {
        fill(
            tr("{} atoms · {} bonds · drag to turn"),
            &[&structure.atoms.len(), &structure.bonds.len()],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ETHANOL: &str = "ethanol\n  test\n\n  3  2  0  0  0  0  0  0  0  0999 V2000\n\
    \x20   0.0000    0.0000    0.0000 C   0  0  0  0  0  0  0  0  0  0  0  0\n\
    \x20   1.5000    0.0000    0.0000 C   0  0  0  0  0  0  0  0  0  0  0  0\n\
    \x20   2.0000    1.4000    0.0000 O   0  0  0  0  0  0  0  0  0  0  0  0\n\
    \x20 1  2  1  0\n\
    \x20 2  3  1  0\nM  END\n$$$$\n";

    #[test]
    fn a_molfile_is_read_into_atoms_and_bonds() {
        let read = parse("ethanol.sdf", ETHANOL).unwrap();
        let elements: Vec<_> = read
            .atoms
            .iter()
            .map(|atom| atom.element.as_str())
            .collect();
        assert_eq!(elements, ["C", "C", "O"]);
        assert_eq!(read.bonds, [(0, 1), (1, 2)]);
        assert_eq!(read.atoms[2].position, [2.0, 1.4, 0.0]);
        assert!(!read.trace);
        assert_eq!(note(&read), "3 atoms · 2 bonds · drag to turn");
        assert_eq!(parse("ethanol.sdf", "not a molfile"), None);
        assert_eq!(parse("ethanol.txt", ETHANOL), None);
    }

    #[test]
    fn a_mol2_file_is_read_with_its_atom_types() {
        let text = "@<TRIPOS>MOLECULE\nwater\n@<TRIPOS>ATOM\n\
            1 O1 0.0 0.0 0.0 O.3 1 HOH 0.0\n\
            2 H1 0.96 0.0 0.0 H 1 HOH 0.0\n\
            3 H2 -0.24 0.93 0.0 H 1 HOH 0.0\n\
            @<TRIPOS>BOND\n1 1 2 1\n2 1 3 1\n";
        let read = parse("water.mol2", text).unwrap();
        assert_eq!(read.atoms[0].element, "O");
        assert_eq!(read.bonds, [(0, 1), (0, 2)]);
    }

    #[test]
    fn a_protein_is_traced_through_each_chain() {
        let atom = |serial: usize, name: &str, chain: &str, residue: i64, x: f32| {
            format!(
                "ATOM  {serial:>5} {name:<4} ALA {chain}{residue:>4}    {x:>8.3}{:>8.3}{:>8.3}  1.00  0.00           C",
                0.0, 0.0
            )
        };
        let text = [
            atom(1, "N", "A", 1, 0.0),
            atom(2, "CA", "A", 1, 1.0),
            atom(3, "CA", "A", 2, 4.8),
            atom(4, "CA", "A", 3, 8.6),
            atom(5, "CA", "B", 1, 20.0),
            atom(6, "CA", "B", 5, 23.8),
            "ENDMDL".into(),
            atom(7, "CA", "A", 1, 99.0),
        ]
        .join("\n");
        let read = parse("1abc.pdb", &text).unwrap();
        assert!(read.trace);
        // Six atoms in the first model; five of them Cα.
        assert_eq!(read.total_atoms, 6);
        assert_eq!(read.atoms.len(), 5);
        // Within chain A, not across to B, and not over B's gap.
        assert_eq!(read.bonds, [(0, 1), (1, 2)]);
        assert_eq!(note(&read), "2 chains · 5 residues traced · drag to turn");
    }

    #[test]
    fn atoms_are_centred_turned_and_fitted() {
        let read = parse("ethanol.sdf", ETHANOL).unwrap();
        let points = project(&read, 0., 0., 200., 100.);
        for (x, y, _) in &points {
            assert!((0.0..=200.0).contains(x) && (0.0..=100.0).contains(y));
        }
        // The oxygen is up, so higher on screen than the carbons.
        assert!(points[2].1 < points[0].1);
        // A half turn about the vertical axis mirrors left and right.
        let turned = project(&read, std::f32::consts::PI, 0., 200., 100.);
        assert!((points[0].0 - 100.).signum() != (turned[0].0 - 100.).signum());
        assert_eq!(element_colour("o"), Some((0.0, 0.5)));
        assert_eq!(element_colour("C"), None);
    }
}
