//! Molecules and structures as the WebView's 3Dmol viewer shows them,
//! drawn flat: a small molecule (MOL, SDF, MOL2, or a crystal's CIF) as its
//! atoms and bonds, and a protein or nucleic acid (PDB or mmCIF) as the
//! trace of its chains, turned by dragging. Reading and projecting are pure, so they are tested here;
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
        "cif" | "mmcif" => cif(text),
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
    Some(assemble(all, backbone))
}

/// A polymer as the trace through its backbone atoms (with their residue
/// numbers), or, without a backbone, all its atoms bonded by distance.
fn assemble(all: Vec<Atom>, backbone: Vec<(Atom, i64)>) -> Structure {
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
        return Structure {
            atoms,
            bonds,
            trace: true,
            total_atoms,
        };
    }
    let atoms: Vec<Atom> = all.into_iter().take(BONDED_BY_DISTANCE).collect();
    Structure {
        bonds: by_distance(&atoms),
        atoms,
        trace: false,
        total_atoms,
    }
}

/// The most atoms whose bonds are worked out from their distances.
const BONDED_BY_DISTANCE: usize = 2000;

/// A CIF file: an mmCIF structure as a PDB file is read, from its
/// `_atom_site` table's Cartesian coordinates; a small-molecule CIF from its
/// fractional coordinates in the unit cell (the asymmetric unit only, as
/// the file lists it).
fn cif(text: &str) -> Option<Structure> {
    let read = crate::cif::read(text);
    let sites = read
        .loop_with("_atom_site_cartn_x")
        .or_else(|| read.loop_with("_atom_site_fract_x"))?;
    let column = |names: &[&str]| names.iter().find_map(|name| sites.column(name));
    let value = |row: &[String], at: Option<usize>| -> Option<String> {
        at.and_then(|at| row.get(at))
            .filter(|value| !matches!(value.as_str(), "?" | "."))
            .cloned()
    };
    let element_at = column(&["_atom_site_type_symbol"]);
    let label_at = column(&[
        "_atom_site_label_atom_id",
        "_atom_site_auth_atom_id",
        "_atom_site_label",
    ]);
    if let Some(x_at) = sites.column("_atom_site_cartn_x") {
        let (y_at, z_at) = (
            sites.column("_atom_site_cartn_y")?,
            sites.column("_atom_site_cartn_z")?,
        );
        let group_at = column(&["_atom_site_group_pdb"]);
        let chain_at = column(&["_atom_site_auth_asym_id", "_atom_site_label_asym_id"]);
        let residue_at = column(&["_atom_site_auth_seq_id", "_atom_site_label_seq_id"]);
        let model_at = column(&["_atom_site_pdbx_pdb_model_num"]);
        let mut model = None;
        let mut all = Vec::new();
        let mut backbone = Vec::new();
        for row in &sites.rows {
            // The first model only.
            let this_model = value(row, model_at);
            if model.is_none() {
                model = this_model.clone();
            } else if this_model != model {
                break;
            }
            let coordinate = |at: usize| row.get(at).and_then(|value| crate::cif::number(value));
            let (Some(x), Some(y), Some(z)) =
                (coordinate(x_at), coordinate(y_at), coordinate(z_at))
            else {
                continue;
            };
            let name = value(row, label_at).unwrap_or_default();
            let atom = Atom {
                element: value(row, element_at)
                    .map(|symbol| type_symbol(&symbol))
                    .unwrap_or_else(|| element_symbol(&name)),
                position: [x as f32, y as f32, z as f32],
                chain: value(row, chain_at).unwrap_or_default(),
            };
            let polymer = value(row, group_at).is_none_or(|group| group == "ATOM");
            if polymer && matches!(name.as_str(), "CA" | "P") {
                let residue = value(row, residue_at)
                    .and_then(|residue| residue.parse::<i64>().ok())
                    .unwrap_or_default();
                backbone.push((atom.clone(), residue));
            }
            all.push(atom);
        }
        return Some(assemble(all, backbone));
    }
    // A small molecule's fractional coordinates, turned Cartesian with the
    // cell's edges and angles.
    let cell = |name: &str| read.number(name);
    let (a, b, c) = (
        cell("_cell_length_a")?,
        cell("_cell_length_b")?,
        cell("_cell_length_c")?,
    );
    let angle = |name: &str| cell(name).unwrap_or(90.0).to_radians();
    let (alpha, beta, gamma) = (
        angle("_cell_angle_alpha"),
        angle("_cell_angle_beta"),
        angle("_cell_angle_gamma"),
    );
    let cy = (alpha.cos() - beta.cos() * gamma.cos()) / gamma.sin();
    let cz = (1.0 - beta.cos().powi(2) - cy.powi(2)).max(0.0).sqrt();
    let axes = [
        [a, 0.0, 0.0],
        [b * gamma.cos(), b * gamma.sin(), 0.0],
        [c * beta.cos(), c * cy, c * cz],
    ];
    let fractions = [
        sites.column("_atom_site_fract_x")?,
        sites.column("_atom_site_fract_y")?,
        sites.column("_atom_site_fract_z")?,
    ];
    let mut atoms = Vec::new();
    'rows: for row in &sites.rows {
        let mut fraction = [0.0; 3];
        for (axis, at) in fractions.iter().enumerate() {
            match row.get(*at).and_then(|value| crate::cif::number(value)) {
                Some(value) => fraction[axis] = value,
                // A site without a position is not drawn.
                None => continue 'rows,
            }
        }
        let position: [f32; 3] = std::array::from_fn(|out| {
            (0..3)
                .map(|axis| fraction[axis] * axes[axis][out])
                .sum::<f64>() as f32
        });
        let label = value(row, label_at).unwrap_or_default();
        atoms.push(Atom {
            element: value(row, element_at)
                .map(|symbol| type_symbol(&symbol))
                .unwrap_or_else(|| element_symbol(&label)),
            position,
            chain: String::new(),
        });
    }
    Some(assemble(atoms, Vec::new()))
}

/// The element a CIF type symbol names, in any case: `O2-` is oxygen and
/// `FE3+` or `Fe3+` iron.
fn type_symbol(text: &str) -> String {
    let letters: String = text
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .take(2)
        .collect();
    let mut chars = letters.chars();
    match chars.next() {
        Some(first) => format!(
            "{}{}",
            first.to_ascii_uppercase(),
            chars.as_str().to_ascii_lowercase()
        ),
        None => String::new(),
    }
}

/// The element an atom label names: `C12` is carbon and `Cl1` chlorine,
/// while `CA` (a Cα) is carbon.
fn element_symbol(text: &str) -> String {
    let letters: Vec<char> = text
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .take(2)
        .collect();
    match letters.as_slice() {
        [first, second] if second.is_ascii_lowercase() => {
            format!("{}{}", first.to_ascii_uppercase(), second)
        }
        [first, ..] => first.to_ascii_uppercase().to_string(),
        [] => String::new(),
    }
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
    fn an_mmcif_structure_is_traced_like_a_pdb_file() {
        let text = "data_1ABC\n#\nloop_\n\
            _atom_site.group_PDB\n_atom_site.id\n_atom_site.type_symbol\n\
            _atom_site.label_atom_id\n_atom_site.label_seq_id\n_atom_site.Cartn_x\n\
            _atom_site.Cartn_y\n_atom_site.Cartn_z\n_atom_site.auth_seq_id\n\
            _atom_site.auth_asym_id\n_atom_site.pdbx_PDB_model_num\n\
            ATOM 1 N N 1 0.0 0.0 0.0 1 A 1\n\
            ATOM 2 C CA 1 1.0 0.0 0.0 1 A 1\n\
            ATOM 3 C CA 2 4.8 0.0 0.0 2 A 1\n\
            HETATM 4 CA CA . 9.0 0.0 0.0 . B 1\n\
            ATOM 5 C CA 1 99.0 0.0 0.0 1 A 2\n#\n";
        let read = parse("1abc.cif", text).unwrap();
        assert!(read.trace);
        // Four atoms in the first model; a calcium ion is not a Cα.
        assert_eq!(read.total_atoms, 4);
        assert_eq!(read.atoms.len(), 2);
        assert_eq!(read.bonds, [(0, 1)]);
        assert_eq!(read.atoms[1].position, [4.8, 0.0, 0.0]);
        // Without Cα atoms every atom is drawn, the ion as calcium.
        let ions = text.replace(" C CA ", " C CB ");
        let read = parse("1abc.cif", &ions).unwrap();
        assert!(!read.trace);
        assert_eq!(read.atoms[3].element, "Ca");
        assert_eq!(type_symbol("FE3+"), "Fe");
    }

    #[test]
    fn a_small_molecule_cif_is_placed_in_its_cell() {
        let text = "data_water\n\
            _cell_length_a 10.0(2)\n_cell_length_b 10\n_cell_length_c 10\n\
            _cell_angle_alpha 90\n_cell_angle_beta 90\n_cell_angle_gamma 90\n\
            loop_\n_atom_site_label\n_atom_site_type_symbol\n\
            _atom_site_fract_x\n_atom_site_fract_y\n_atom_site_fract_z\n\
            O1 O2- 0.5 0.5 0.5\nH1 H 0.596 0.5 0.5\nH2 H 0.476 0.593 0.5\nCl1 ? ? 0.1 0.1\n";
        let read = parse("water.cif", text).unwrap();
        assert!(!read.trace);
        let elements: Vec<_> = read
            .atoms
            .iter()
            .map(|atom| atom.element.as_str())
            .collect();
        // The site without a position is left out.
        assert_eq!(elements, ["O", "H", "H"]);
        assert!((read.atoms[1].position[0] - 5.96).abs() < 1e-4);
        assert_eq!(read.bonds, [(0, 1), (0, 2)]);
        // Without a type symbol the label names the element.
        assert_eq!(element_symbol("Cl1"), "Cl");
        assert_eq!(element_symbol("Fe3+"), "Fe");
        assert_eq!(element_symbol("C12"), "C");
        // A monoclinic cell leans its third edge.
        let leaning = text.replace("_cell_angle_beta 90", "_cell_angle_beta 120");
        let read = parse("water.cif", &leaning).unwrap();
        assert!((read.atoms[0].position[0] - 2.5).abs() < 1e-3);
        assert_eq!(parse("water.cif", "data_x\n_cell_length_a 1\n"), None);
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
