//! A SMILES file as the WebView's molecule viewer shows it: the first
//! molecule's atoms and bonds read from the line notation and laid out
//! flat, for the structure preview to draw. Hydrogens the notation leaves
//! implicit are not drawn. Pure, so it is tested here.
use std::collections::{HashMap, VecDeque};

use crate::molecules::{Atom, Structure};

/// The most atoms laid out; a larger molecule is shown as text.
pub const MAX_ATOMS: usize = 300;

/// The two-letter elements a bracket atom may name.
const TWO_LETTER: &[&str] = &[
    "He", "Li", "Be", "Ne", "Na", "Mg", "Al", "Si", "Cl", "Ar", "Ca", "Sc", "Ti", "Cr", "Mn", "Fe",
    "Co", "Ni", "Cu", "Zn", "Ga", "Ge", "As", "Se", "Br", "Kr", "Rb", "Sr", "Zr", "Nb", "Mo", "Tc",
    "Ru", "Rh", "Pd", "Ag", "Cd", "In", "Sn", "Sb", "Te", "Xe", "Cs", "Ba", "La", "Ce", "Pr", "Nd",
    "Pm", "Sm", "Eu", "Gd", "Tb", "Dy", "Ho", "Er", "Tm", "Yb", "Lu", "Hf", "Ta", "Re", "Os", "Ir",
    "Pt", "Au", "Hg", "Tl", "Pb", "Bi", "Po", "At", "Rn", "Fr", "Ra", "Ac", "Th", "Pa", "Np", "Pu",
    "Am", "Cm",
];

#[derive(Debug, Default, PartialEq)]
struct Graph {
    elements: Vec<String>,
    aromatic: Vec<bool>,
    /// Pairs of atoms with the bond's order: 1, 2, 3, or 4 for aromatic.
    bonds: Vec<(usize, usize, u8)>,
    /// The bonds that close a ring.
    closures: Vec<(usize, usize)>,
}

impl Graph {
    fn order(&self, first: usize, second: usize) -> u8 {
        if self.aromatic[first] && self.aromatic[second] {
            4
        } else {
            1
        }
    }

    fn add(
        &mut self,
        element: String,
        aromatic: bool,
        previous: &mut Option<usize>,
        bond: &mut Option<u8>,
    ) {
        let index = self.elements.len();
        self.elements.push(element);
        self.aromatic.push(aromatic);
        if let Some(before) = *previous {
            let order = bond.take().unwrap_or_else(|| self.order(before, index));
            self.bonds.push((before, index, order));
        }
        *bond = None;
        *previous = Some(index);
    }
}

/// The element inside a bracket atom (`[13CH4]`, `[nH]`, `[Fe+2]`), and
/// whether it is aromatic.
fn bracket(inside: &str) -> Option<(String, bool)> {
    let rest = inside.trim_start_matches(|ch: char| ch.is_ascii_digit());
    let mut chars = rest.chars();
    let first = chars.next()?;
    let second = chars.next();
    if first.is_ascii_lowercase() {
        // Aromatic: `se` and `as` besides the organic ones.
        if let Some(second) = second {
            let pair = format!("{first}{second}");
            if pair == "se" || pair == "as" {
                return Some((format!("{}{second}", first.to_ascii_uppercase()), true));
            }
        }
        return "bcnops"
            .contains(first)
            .then(|| (first.to_ascii_uppercase().to_string(), true));
    }
    if !first.is_ascii_uppercase() {
        // `*` is any atom.
        return (first == '*').then(|| ("*".into(), false));
    }
    if let Some(second) = second.filter(char::is_ascii_lowercase) {
        let pair = format!("{first}{second}");
        if TWO_LETTER.contains(&pair.as_str()) {
            return Some((pair, false));
        }
    }
    Some((first.to_string(), false))
}

/// The atoms and bonds `smiles` writes, or `None` when it is not SMILES.
fn read(smiles: &str) -> Option<Graph> {
    let chars: Vec<char> = smiles.chars().collect();
    let mut graph = Graph::default();
    let mut previous: Option<usize> = None;
    let mut branches: Vec<Option<usize>> = Vec::new();
    let mut bond: Option<u8> = None;
    let mut rings: HashMap<u32, (usize, Option<u8>)> = HashMap::new();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        match ch {
            '(' => branches.push(previous),
            ')' => previous = branches.pop()?,
            '.' => {
                previous = None;
                bond = None;
            }
            '-' | '/' | '\\' => bond = Some(1),
            '=' => bond = Some(2),
            // A quadruple bond is drawn as a triple one.
            '#' | '$' => bond = Some(3),
            ':' => bond = Some(4),
            '0'..='9' | '%' => {
                let number = if ch == '%' {
                    let digits: String = chars.get(at + 1..at + 3)?.iter().collect();
                    at += 2;
                    digits.parse().ok()?
                } else {
                    ch.to_digit(10)?
                };
                let current = previous?;
                match rings.remove(&number) {
                    Some((other, opened)) => {
                        if other == current {
                            return None;
                        }
                        let order = bond
                            .or(opened)
                            .unwrap_or_else(|| graph.order(other, current));
                        graph.bonds.push((other, current, order));
                        graph.closures.push((other, current));
                    }
                    None => {
                        rings.insert(number, (current, bond));
                    }
                }
                bond = None;
            }
            '[' => {
                let end = at + chars[at..].iter().position(|ch| *ch == ']')?;
                let inside: String = chars[at + 1..end].iter().collect();
                let (element, aromatic) = bracket(&inside)?;
                graph.add(element, aromatic, &mut previous, &mut bond);
                at = end;
            }
            '*' => graph.add("*".into(), false, &mut previous, &mut bond),
            'C' | 'B' if matches!(chars.get(at + 1), Some('l' | 'r')) => {
                let pair = format!("{ch}{}", chars[at + 1]);
                if pair != "Cl" && pair != "Br" {
                    return None;
                }
                graph.add(pair, false, &mut previous, &mut bond);
                at += 1;
            }
            'B' | 'C' | 'N' | 'O' | 'P' | 'S' | 'F' | 'I' => {
                graph.add(ch.to_string(), false, &mut previous, &mut bond)
            }
            'b' | 'c' | 'n' | 'o' | 'p' | 's' => graph.add(
                ch.to_ascii_uppercase().to_string(),
                true,
                &mut previous,
                &mut bond,
            ),
            _ => return None,
        }
        at += 1;
    }
    // An open branch or ring is not a whole molecule.
    (branches.is_empty() && rings.is_empty() && !graph.elements.is_empty()).then_some(graph)
}

/// How far apart two atoms `steps` bonds away lie along a zigzag chain of
/// unit bonds at 120°.
fn zigzag(steps: u32) -> f32 {
    let along = steps as f32 * 0.866;
    if steps % 2 == 0 {
        along
    } else {
        (along * along + 0.25).sqrt()
    }
}

/// The atoms on the shortest path from `from` to `to` that does not take
/// the bond between them.
fn ring_through(neighbours: &[Vec<usize>], from: usize, to: usize) -> Option<Vec<usize>> {
    let mut before = vec![usize::MAX; neighbours.len()];
    before[from] = from;
    let mut queue = VecDeque::from([from]);
    while let Some(atom) = queue.pop_front() {
        for &next in &neighbours[atom] {
            if before[next] != usize::MAX || (atom == from && next == to) {
                continue;
            }
            before[next] = atom;
            if next == to {
                let mut path = vec![to];
                let mut at = to;
                while at != from {
                    at = before[at];
                    path.push(at);
                }
                return Some(path);
            }
            queue.push_back(next);
        }
    }
    None
}

/// Flat positions for the atoms, about a unit bond apart: each connected
/// piece by classical scaling of the distances its bonds imply (rings as
/// regular polygons), refined by stress majorization, the pieces side by
/// side.
fn layout(graph: &Graph) -> Vec<[f32; 2]> {
    let count = graph.elements.len();
    let mut neighbours = vec![Vec::new(); count];
    for &(first, second, _) in &graph.bonds {
        neighbours[first].push(second);
        neighbours[second].push(first);
    }
    // Bonds apart, between every pair.
    let steps: Vec<Vec<u32>> = (0..count)
        .map(|start| {
            let mut seen = vec![u32::MAX; count];
            seen[start] = 0;
            let mut queue = VecDeque::from([start]);
            while let Some(atom) = queue.pop_front() {
                for &next in &neighbours[atom] {
                    if seen[next] == u32::MAX {
                        seen[next] = seen[atom] + 1;
                        queue.push_back(next);
                    }
                }
            }
            seen
        })
        .collect();
    let mut target: Vec<Vec<f32>> = steps
        .iter()
        .map(|row| row.iter().map(|&steps| zigzag(steps)).collect())
        .collect();
    let mut in_ring = vec![vec![false; count]; count];
    for &(first, second) in &graph.closures {
        let Some(ring) = ring_through(&neighbours, first, second) else {
            continue;
        };
        let size = ring.len();
        if !(3..=12).contains(&size) {
            continue;
        }
        for i in 0..size {
            for j in i + 1..size {
                let apart = (j - i).min(size - (j - i)) as f32;
                let chord = (std::f32::consts::PI * apart / size as f32).sin()
                    / (std::f32::consts::PI / size as f32).sin();
                let (a, b) = (ring[i], ring[j]);
                if !in_ring[a][b] || chord < target[a][b] {
                    target[a][b] = chord;
                    target[b][a] = chord;
                }
                in_ring[a][b] = true;
                in_ring[b][a] = true;
            }
        }
    }

    let mut positions = vec![[0f32; 2]; count];
    let mut placed = vec![false; count];
    let mut left = 0f32;
    for start in 0..count {
        if placed[start] {
            continue;
        }
        let piece: Vec<usize> = (0..count)
            .filter(|&atom| steps[start][atom] != u32::MAX)
            .collect();
        for &atom in &piece {
            placed[atom] = true;
        }
        let mut local = classical(&piece, &target);
        majorize(&piece, &target, &mut local);
        let min_x = local.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let max_x = local.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        let mid_y = {
            let low = local.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
            let high = local.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
            (low + high) / 2.0
        };
        for (slot, &atom) in piece.iter().enumerate() {
            positions[atom] = [local[slot][0] - min_x + left, local[slot][1] - mid_y];
        }
        left += max_x - min_x + 1.5;
    }
    positions
}

/// Classical scaling: the two strongest directions of the doubly centred
/// squared distances, found by power iteration.
fn classical(piece: &[usize], target: &[Vec<f32>]) -> Vec<[f32; 2]> {
    let n = piece.len();
    if n == 1 {
        return vec![[0.0, 0.0]];
    }
    let squared: Vec<Vec<f32>> = piece
        .iter()
        .map(|&a| piece.iter().map(|&b| target[a][b].powi(2)).collect())
        .collect();
    let row: Vec<f32> = squared
        .iter()
        .map(|values| values.iter().sum::<f32>() / n as f32)
        .collect();
    let all = row.iter().sum::<f32>() / n as f32;
    let mut centred: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| -0.5 * (squared[i][j] - row[i] - row[j] + all))
                .collect()
        })
        .collect();
    let mut axes = Vec::new();
    for axis in 0..2 {
        // A start that is not symmetric, so it is not orthogonal to the
        // answer by chance; the same every time.
        let mut vector: Vec<f32> = (0..n)
            .map(|i| ((i * 7 + axis * 3) % 11) as f32 + 1.0)
            .collect();
        let mut value = 0.0;
        for _ in 0..100 {
            let next: Vec<f32> = centred
                .iter()
                .map(|row| row.iter().zip(&vector).map(|(a, b)| a * b).sum())
                .collect();
            let norm = next.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm < 1e-9 {
                break;
            }
            value = norm;
            vector = next.into_iter().map(|v| v / norm).collect();
        }
        // Take this direction out before looking for the next.
        for i in 0..n {
            for j in 0..n {
                centred[i][j] -= value * vector[i] * vector[j];
            }
        }
        axes.push((value.sqrt(), vector));
    }
    (0..n)
        .map(|i| {
            // Atoms the scaling puts in one place are spread a little, so
            // majorization can move them apart.
            let nudge = (i as f32 * 2.399).sin() * 0.05;
            [
                axes[0].0 * axes[0].1[i] + nudge,
                axes[1].0 * axes[1].1[i] - nudge,
            ]
        })
        .collect()
}

/// Moves each atom to where its distances to the others best match the
/// targets, nearer atoms counting more, until the layout settles.
fn majorize(piece: &[usize], target: &[Vec<f32>], local: &mut [[f32; 2]]) {
    let n = piece.len();
    if n < 2 {
        return;
    }
    for _ in 0..300 {
        let mut moved = 0f32;
        for i in 0..n {
            let mut sum = [0f32; 2];
            let mut weights = 0f32;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let wanted = target[piece[i]][piece[j]];
                let weight = 1.0 / (wanted * wanted);
                let dx = local[i][0] - local[j][0];
                let dy = local[i][1] - local[j][1];
                let distance = (dx * dx + dy * dy).sqrt().max(1e-4);
                sum[0] += weight * (local[j][0] + wanted * dx / distance);
                sum[1] += weight * (local[j][1] + wanted * dy / distance);
                weights += weight;
            }
            let next = [sum[0] / weights, sum[1] / weights];
            moved = moved.max((next[0] - local[i][0]).abs() + (next[1] - local[i][1]).abs());
            local[i] = next;
        }
        if moved < 1e-4 {
            break;
        }
    }
}

/// The first molecule in a SMILES file (one per line, a name after it; a
/// header line is passed over), laid out flat.
pub fn parse(text: &str) -> Option<Structure> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let (at, graph) = lines.iter().take(5).enumerate().find_map(|(at, line)| {
        let smiles = line.split_whitespace().next()?;
        read(smiles).map(|graph| (at, graph))
    })?;
    if graph.elements.len() > MAX_ATOMS {
        return None;
    }
    let positions = layout(&graph);
    let name = lines[at]
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ");
    let atoms = graph
        .elements
        .iter()
        .zip(&positions)
        .map(|(element, [x, y])| Atom {
            element: element.clone(),
            position: [*x, *y, 0.0],
            chain: String::new(),
        })
        .collect::<Vec<_>>();
    Some(Structure {
        total_atoms: atoms.len(),
        atoms,
        bonds: graph.bonds.iter().map(|&(a, b, _)| (a, b)).collect(),
        orders: graph.bonds.iter().map(|&(_, _, order)| order).collect(),
        trace: false,
        name: (!name.is_empty()).then_some(name),
        records: lines.len() - at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance(read: &Structure, a: usize, b: usize) -> f32 {
        let (p, q) = (read.atoms[a].position, read.atoms[b].position);
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
    }

    #[test]
    fn atoms_bonds_branches_and_orders_are_read() {
        let acetic = read("CC(=O)O").unwrap();
        assert_eq!(acetic.elements, ["C", "C", "O", "O"]);
        assert_eq!(acetic.bonds, [(0, 1, 1), (1, 2, 2), (1, 3, 1)]);
        assert_eq!(read("C#N").unwrap().bonds, [(0, 1, 3)]);
        let salt = read("[NH4+].[Cl-]").unwrap();
        assert_eq!(salt.elements, ["N", "Cl"]);
        assert!(salt.bonds.is_empty());
        let odd = read("[13CH3][nH]Br[Se][se]").unwrap();
        assert_eq!(odd.elements, ["C", "N", "Br", "Se", "Se"]);
        assert_eq!(odd.aromatic, [false, true, false, false, true]);
        // A ring closure with a two-digit number keeps the order it was
        // opened with.
        let ring = read("C=%12CCC%12").unwrap();
        assert_eq!(ring.closures, [(0, 3)]);
        assert_eq!(ring.bonds.last(), Some(&(0, 3, 2)));
        for broken in ["C(C", "C1CC", "Xx", "C)", "", "C11"] {
            assert_eq!(read(broken), None, "{broken}");
        }
    }

    #[test]
    fn benzene_is_a_regular_hexagon_of_aromatic_bonds() {
        let read = parse("c1ccccc1 benzene\n").unwrap();
        assert_eq!(read.atoms.len(), 6);
        assert_eq!(read.orders, [4; 6]);
        assert_eq!(read.name.as_deref(), Some("benzene"));
        for &(a, b) in &read.bonds {
            assert!((distance(&read, a, b) - 1.0).abs() < 0.05, "{a}-{b}");
        }
        for (a, b) in [(0, 3), (1, 4), (2, 5)] {
            assert!((distance(&read, a, b) - 2.0).abs() < 0.08, "{a}-{b}");
        }
    }

    #[test]
    fn a_chain_zigzags_and_pieces_lie_side_by_side() {
        let read = parse("CCCC").unwrap();
        assert!((distance(&read, 0, 1) - 1.0).abs() < 0.05);
        assert!((distance(&read, 0, 2) - 1.732).abs() < 0.08);
        let salt = parse("[Na+].[Cl-]").unwrap();
        assert!(distance(&salt, 0, 1) >= 1.4);
        assert!(salt.atoms.iter().all(|atom| atom.position[2] == 0.0));
    }

    #[test]
    fn a_file_shows_its_first_molecule_after_any_header() {
        let read = parse("SMILES Name\nCCO ethanol\nC=O formaldehyde\n\n").unwrap();
        assert_eq!(read.atoms.len(), 3);
        assert_eq!(read.records, 2);
        assert_eq!(read.name.as_deref(), Some("ethanol"));
        assert_eq!(parse("not smiles at all"), None);
        let big = "C".repeat(MAX_ATOMS + 1);
        assert_eq!(parse(&big), None);
    }
}
