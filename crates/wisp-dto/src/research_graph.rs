//! How both desktops read a project's research graph: its node kinds in the
//! order they are shown, node and edge metadata as words, the nodes grouped
//! by kind with the links leaving them, and the fixed overview layout — a
//! column per kind that has nodes, a row per node in stored order, and each
//! edge's route as lines and cubic curves. Drawing stays with the client:
//! the WebView writes a route as an SVG path, the GPUI desktop paints it.
use crate::{ResearchEdge, ResearchGraph, ResearchNode};
use std::collections::HashMap;

/// Node kinds in display order. Closed set: it mirrors
/// `wisp_store::ResearchNodeKind`, so every stored node lands in a column.
pub const KINDS: [&str; 5] = ["decision", "paper", "data_asset", "run", "artifact"];

pub const NODE_WIDTH: i32 = 176;
pub const NODE_HEIGHT: i32 = 58;
pub const COLUMN_GAP: i32 = 64;
pub const ROW_GAP: i32 = 92;
/// Baseline of the column headings.
pub const HEADER_Y: i32 = 34;
pub const FIRST_ROW_Y: i32 = 64;

/// Raw `metadata_json` as displayable `key: value` pairs. String values read
/// bare; anything else as compact JSON. Empty, non-object or unparseable
/// metadata yields no pairs.
pub fn metadata_pairs(metadata_json: &str) -> Vec<(String, String)> {
    match serde_json::from_str::<serde_json::Value>(metadata_json) {
        Ok(serde_json::Value::Object(map)) => map
            .into_iter()
            .map(|(key, value)| match value {
                serde_json::Value::String(text) => (key, text),
                other => (key, other.to_string()),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The pairs on one line, `key: value · key: value`.
pub fn metadata_summary(metadata_json: &str) -> String {
    metadata_pairs(metadata_json)
        .into_iter()
        .map(|(key, value)| format!("{key}: {value}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// A node's title by id, or the id itself when no node has it.
fn titles(graph: &ResearchGraph) -> HashMap<&str, &str> {
    graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.title.as_str()))
        .collect()
}

/// One node of a kind section, with each edge that leaves it and the title
/// of where that edge goes.
#[derive(Debug, PartialEq, Eq)]
pub struct NodeRow<'a> {
    pub node: &'a ResearchNode,
    pub links: Vec<(&'a ResearchEdge, String)>,
}

/// The nodes bucketed by kind, in [`KINDS`] order, kinds without nodes left
/// out. An edge hangs off its source only.
pub fn group_by_kind(graph: &ResearchGraph) -> Vec<(&'static str, Vec<NodeRow<'_>>)> {
    let titles = titles(graph);
    let mut outgoing: HashMap<&str, Vec<&ResearchEdge>> = HashMap::new();
    for edge in &graph.edges {
        outgoing
            .entry(edge.source_id.as_str())
            .or_default()
            .push(edge);
    }
    KINDS
        .into_iter()
        .filter_map(|kind| {
            let rows = graph
                .nodes
                .iter()
                .filter(|node| node.kind == kind)
                .map(|node| NodeRow {
                    node,
                    links: outgoing
                        .get(node.id.as_str())
                        .map(|edges| {
                            edges
                                .iter()
                                .map(|edge| {
                                    let target = titles
                                        .get(edge.target_id.as_str())
                                        .copied()
                                        .unwrap_or(edge.target_id.as_str());
                                    (*edge, target.to_owned())
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect::<Vec<_>>();
            (!rows.is_empty()).then_some((kind, rows))
        })
        .collect()
}

/// `source —relation→ target`, with the edge's metadata after a dot when it
/// has any: what an edge says when pointed at.
pub fn edge_title(graph: &ResearchGraph, edge: &ResearchEdge) -> String {
    let titles = titles(graph);
    let name = |id: &str| titles.get(id).copied().unwrap_or(id).to_owned();
    let line = format!(
        "{} —{}→ {}",
        name(&edge.source_id),
        edge.relation,
        name(&edge.target_id)
    );
    match metadata_summary(&edge.metadata_json) {
        metadata if metadata.is_empty() => line,
        metadata => format!("{line} · {metadata}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

const fn at(x: i32, y: i32) -> Point {
    Point { x, y }
}

/// One piece of an edge's route, from where the previous one ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Line(Point),
    /// Two control points, then the end.
    Cubic(Point, Point, Point),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub start: Point,
    pub steps: Vec<Step>,
}

impl Route {
    /// The route as an SVG path's `d`.
    pub fn svg_path(&self) -> String {
        let mut path = format!("M {} {}", self.start.x, self.start.y);
        for step in &self.steps {
            match step {
                Step::Line(to) => path.push_str(&format!(" L {} {}", to.x, to.y)),
                Step::Cubic(a, b, to) => path.push_str(&format!(
                    " C {} {}, {} {}, {} {}",
                    a.x, a.y, b.x, b.y, to.x, to.y
                )),
            }
        }
        path
    }

    /// The route as straight pieces: a line as it is, a curve cut into
    /// `pieces` chords. The first point is the start; the last is the end.
    pub fn polyline(&self, pieces: usize) -> Vec<(f32, f32)> {
        let pieces = pieces.max(1);
        let mut points = vec![(self.start.x as f32, self.start.y as f32)];
        let mut from = self.start;
        for step in &self.steps {
            match *step {
                Step::Line(to) => {
                    points.push((to.x as f32, to.y as f32));
                    from = to;
                }
                Step::Cubic(a, b, to) => {
                    let p = |point: Point| (point.x as f32, point.y as f32);
                    let (p0, p1, p2, p3) = (p(from), p(a), p(b), p(to));
                    for piece in 1..=pieces {
                        let t = piece as f32 / pieces as f32;
                        let u = 1.0 - t;
                        let (w0, w1, w2, w3) =
                            (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                        points.push((
                            w0 * p0.0 + w1 * p1.0 + w2 * p2.0 + w3 * p3.0,
                            w0 * p0.1 + w1 * p1.1 + w2 * p2.1 + w3 * p3.1,
                        ));
                    }
                    from = to;
                }
            }
        }
        points
    }
}

/// A column: its kind and the left edge of its nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnLayout {
    pub kind: &'static str,
    pub x: i32,
}

/// A node's top-left corner; `node` indexes `ResearchGraph::nodes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeLayout {
    pub node: usize,
    pub x: i32,
    pub y: i32,
}

/// An edge's route and where its relation is written (the label's centre,
/// on its baseline); `edge` indexes `ResearchGraph::edges`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeLayout {
    pub edge: usize,
    pub route: Route,
    pub label: Point,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub columns: Vec<ColumnLayout>,
    pub nodes: Vec<NodeLayout>,
    pub edges: Vec<EdgeLayout>,
}

/// Fixed columns by kind, rows in stored order: deterministic, with no
/// simulation. An edge between two nodes of one column bows out to its
/// right; one to the next column is an S-curve; a longer one drops into the
/// gap under its rows, so it does not cross the nodes of the columns between.
/// An edge naming a node that is not in the graph is left out.
pub fn layout(graph: &ResearchGraph) -> Layout {
    let kinds = KINDS
        .into_iter()
        .filter(|kind| graph.nodes.iter().any(|node| node.kind == *kind))
        .collect::<Vec<_>>();
    let column_count = kinds.len().max(1) as i32;
    let used_width = column_count * NODE_WIDTH + (column_count - 1) * COLUMN_GAP;
    let width = (used_width + 80).max(720);
    let first_x = (width - used_width) / 2;
    let max_rows = kinds
        .iter()
        .map(|kind| graph.nodes.iter().filter(|node| node.kind == *kind).count())
        .max()
        .unwrap_or(0) as i32;
    let height = (FIRST_ROW_Y + max_rows * ROW_GAP + 28).max(360);

    let mut positions = HashMap::new();
    let mut nodes = Vec::with_capacity(graph.nodes.len());
    let columns = kinds
        .into_iter()
        .enumerate()
        .map(|(column, kind)| {
            let x = first_x + column as i32 * (NODE_WIDTH + COLUMN_GAP);
            for (row, (index, node)) in graph
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| node.kind == kind)
                .enumerate()
            {
                let y = FIRST_ROW_Y + row as i32 * ROW_GAP;
                positions.insert(node.id.as_str(), (x, y));
                nodes.push(NodeLayout { node: index, x, y });
            }
            ColumnLayout { kind, x }
        })
        .collect::<Vec<_>>();

    let edges = graph
        .edges
        .iter()
        .enumerate()
        .filter_map(|(index, edge)| {
            let &(source_x, source_y) = positions.get(edge.source_id.as_str())?;
            let &(target_x, target_y) = positions.get(edge.target_id.as_str())?;
            let (route, label) = route(source_x, source_y, target_x, target_y);
            Some(EdgeLayout {
                edge: index,
                route,
                label,
            })
        })
        .collect();

    Layout {
        width,
        height,
        columns,
        nodes,
        edges,
    }
}

fn route(source_x: i32, source_y: i32, target_x: i32, target_y: i32) -> (Route, Point) {
    if source_x == target_x {
        let downward = source_y < target_y;
        let start_y = if downward {
            source_y + NODE_HEIGHT
        } else {
            source_y
        };
        let end_y = if downward {
            target_y
        } else {
            target_y + NODE_HEIGHT
        };
        let center_x = source_x + NODE_WIDTH / 2;
        let side_x = source_x + NODE_WIDTH + 34;
        return (
            Route {
                start: at(center_x, start_y),
                steps: vec![Step::Cubic(
                    at(side_x, start_y),
                    at(side_x, end_y),
                    at(center_x, end_y),
                )],
            },
            at(side_x, (start_y + end_y) / 2 - 5),
        );
    }
    let direction = if source_x < target_x { 1 } else { -1 };
    let start_x = if direction > 0 {
        source_x + NODE_WIDTH
    } else {
        source_x
    };
    let end_x = if direction > 0 {
        target_x
    } else {
        target_x + NODE_WIDTH
    };
    let start_y = source_y + NODE_HEIGHT / 2;
    let end_y = target_y + NODE_HEIGHT / 2;
    let label_x = (start_x + end_x) / 2;
    if (source_x - target_x).abs() > NODE_WIDTH + COLUMN_GAP {
        // A direct long edge would pass through nodes in intervening
        // columns, so use the whitespace between graph rows.
        let lane_y = start_y.max(end_y) + NODE_HEIGHT / 2 + 18;
        let lane_start = start_x + direction * 42;
        let lane_end = end_x - direction * 42;
        let out_x = start_x + direction * 24;
        let in_x = end_x - direction * 24;
        return (
            Route {
                start: at(start_x, start_y),
                steps: vec![
                    Step::Cubic(
                        at(out_x, start_y),
                        at(out_x, lane_y),
                        at(lane_start, lane_y),
                    ),
                    Step::Line(at(lane_end, lane_y)),
                    Step::Cubic(at(in_x, lane_y), at(in_x, end_y), at(end_x, end_y)),
                ],
            },
            at(label_x, lane_y - 5),
        );
    }
    let control_x = (start_x + end_x) / 2;
    (
        Route {
            start: at(start_x, start_y),
            steps: vec![Step::Cubic(
                at(control_x, start_y),
                at(control_x, end_y),
                at(end_x, end_y),
            )],
        },
        at(label_x, (start_y + end_y) / 2 - 5),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: &str, title: &str) -> ResearchNode {
        ResearchNode {
            id: id.into(),
            kind: kind.into(),
            title: title.into(),
            ref_id: None,
            metadata_json: "{}".into(),
        }
    }

    fn edge(source: &str, target: &str, relation: &str) -> ResearchEdge {
        ResearchEdge {
            source_id: source.into(),
            target_id: target.into(),
            relation: relation.into(),
            metadata_json: "{}".into(),
        }
    }

    #[test]
    fn nodes_group_by_kind_with_the_links_that_leave_them() {
        let graph = ResearchGraph {
            nodes: vec![
                node("a1", "data_asset", "counts.tsv"),
                node("d1", "decision", "Use DESeq2"),
            ],
            edges: vec![edge("d1", "a1", "uses"), edge("d1", "gone", "cites")],
        };
        let groups = group_by_kind(&graph);
        assert_eq!(
            groups.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            ["decision", "data_asset"]
        );
        let links = &groups[0].1[0].links;
        assert_eq!(links[0].1, "counts.tsv");
        // A target that is not a node is named by its id.
        assert_eq!(links[1].1, "gone");
        assert!(groups[1].1[0].links.is_empty());
        assert_eq!(
            edge_title(&graph, &graph.edges[0]),
            "Use DESeq2 —uses→ counts.tsv"
        );
    }

    #[test]
    fn metadata_reads_as_pairs_and_an_edge_title_carries_it() {
        assert_eq!(
            metadata_pairs(r#"{"doi": "10.1/x", "rows": 1204}"#),
            vec![
                ("doi".to_owned(), "10.1/x".to_owned()),
                ("rows".to_owned(), "1204".to_owned()),
            ]
        );
        assert!(metadata_pairs("[1]").is_empty() && metadata_pairs("nope").is_empty());
        let mut link = edge("d1", "p1", "cites");
        link.metadata_json = r#"{"confidence":"high"}"#.into();
        let graph = ResearchGraph {
            nodes: vec![node("d1", "decision", "D"), node("p1", "paper", "P")],
            edges: vec![link],
        };
        assert_eq!(
            edge_title(&graph, &graph.edges[0]),
            "D —cites→ P · confidence: high"
        );
    }

    #[test]
    fn the_layout_is_columns_by_kind_and_rows_in_stored_order() {
        let graph = ResearchGraph {
            nodes: vec![
                node("r1", "run", "Run"),
                node("a1", "data_asset", "counts"),
                node("d1", "decision", "Decide"),
                node("a2", "data_asset", "samples"),
            ],
            edges: vec![],
        };
        let first = layout(&graph);
        assert_eq!(first, layout(&graph));
        assert_eq!(
            first.columns.iter().map(|c| c.kind).collect::<Vec<_>>(),
            ["decision", "data_asset", "run"]
        );
        let data = first
            .nodes
            .iter()
            .filter(|n| graph.nodes[n.node].kind == "data_asset")
            .collect::<Vec<_>>();
        assert_eq!(data[0].node, 1);
        assert_eq!(data[1].node, 3);
        assert_eq!(data[0].x, data[1].x);
        assert_eq!(data[1].y - data[0].y, ROW_GAP);
        // No two nodes share a place, and every node is inside the canvas.
        for (i, a) in first.nodes.iter().enumerate() {
            assert!(a.x >= 0 && a.x + NODE_WIDTH <= first.width);
            assert!(a.y + NODE_HEIGHT <= first.height);
            for b in &first.nodes[i + 1..] {
                assert!(
                    (a.x - b.x).abs() >= NODE_WIDTH || (a.y - b.y).abs() >= NODE_HEIGHT,
                    "{a:?} overlaps {b:?}"
                );
            }
        }
    }

    #[test]
    fn edges_join_their_nodes_and_long_ones_take_the_gap_between_rows() {
        let graph = ResearchGraph {
            nodes: vec![
                node("d1", "decision", "Decide"),
                node("p1", "paper", "Paper"),
                node("a1", "data_asset", "counts"),
                node("d2", "decision", "Later"),
            ],
            edges: vec![
                edge("d1", "p1", "cites"),
                edge("d1", "a1", "applies to"),
                edge("d1", "d2", "leads to"),
                edge("missing", "p1", "ignored"),
            ],
        };
        let laid = layout(&graph);
        assert_eq!(laid.edges.len(), 3);
        let place = |id: &str| {
            let index = graph.nodes.iter().position(|n| n.id == id).unwrap();
            let n = laid.nodes.iter().find(|n| n.node == index).unwrap();
            (n.x, n.y)
        };
        let end = |route: &Route| match route.steps.last().unwrap() {
            Step::Line(to) | Step::Cubic(_, _, to) => *to,
        };
        // Next column: right side of the source to the left side of the target.
        let (dx, dy) = place("d1");
        let (px, py) = place("p1");
        let cites = &laid.edges[0].route;
        assert_eq!(cites.start, at(dx + NODE_WIDTH, dy + NODE_HEIGHT / 2));
        assert_eq!(end(cites), at(px, py + NODE_HEIGHT / 2));
        assert_eq!(cites.svg_path().matches(" C ").count(), 1);
        // Two columns over: down into the lane and along it.
        let applies = &laid.edges[1].route;
        assert!(applies.svg_path().contains(" L "));
        let lane = match applies.steps[1] {
            Step::Line(to) => to.y,
            _ => unreachable!(),
        };
        assert!(lane > dy + NODE_HEIGHT && lane < dy + ROW_GAP);
        // Same column: from under the upper node to over the lower one.
        let (_, d2y) = place("d2");
        let leads = &laid.edges[2].route;
        assert_eq!(leads.start.y, dy + NODE_HEIGHT);
        assert_eq!(end(leads).y, d2y);
        // Flattened, a route keeps its ends.
        let points = cites.polyline(8);
        assert_eq!(points.len(), 9);
        assert_eq!(
            points[0],
            ((dx + NODE_WIDTH) as f32, (dy + NODE_HEIGHT / 2) as f32)
        );
        assert_eq!(
            *points.last().unwrap(),
            (px as f32, (py + NODE_HEIGHT / 2) as f32)
        );
    }
}
