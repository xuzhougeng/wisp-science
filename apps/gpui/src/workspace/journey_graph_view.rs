//! The journey's Relationships view: the project's research graph, listed
//! by kind or drawn as the WebView draws it — a column per kind, a box per
//! node, and labelled arrows for the relations — with one relation's
//! details open beside it. The layout is the shared one in
//! `wisp_dto::research_graph`; this file only paints it.
use crate::i18n::{fill, tr};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Selectable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    tooltip::Tooltip,
};
use wisp_dto::{
    ResearchEdge, ResearchGraph,
    research_graph::{self, NODE_HEIGHT, NODE_WIDTH},
};

use super::{
    Workspace,
    shell::{error_banner, glyph, icon_button},
};
use crate::{journey, layers::Layer};

/// Chords a curve is drawn with; enough to look smooth at node scale.
const CURVE_PIECES: usize = 16;
/// The width of the place a relation's name is written in.
const LABEL_WIDTH: f32 = 128.;
/// Characters of a name before it is cut, as in the WebView.
const LABEL_CHARS: usize = 18;
const TITLE_CHARS: usize = 24;

impl Workspace {
    pub(super) fn render_journey_graph(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let mut body = div()
            .id("journey-graph")
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .gap_3();
        if let Some(error) = &self.error {
            body = body.child(error_banner(error, cx));
        }
        let Some(whole) = &self.journey_graph else {
            return body.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(if self.error.is_some() {
                        ""
                    } else {
                        tr("Loading…")
                    }),
            );
        };
        if whole.nodes.is_empty() {
            return body
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(tr("Nothing recorded yet")),
                )
                .child(div().text_xs().text_color(muted).child(tr(
                    "Decisions, papers and data assets the agent records for this project \
                     show up here.",
                )));
        }
        let filter = self.journey_search.read(cx).value().to_string();
        let graph = journey::filter_graph(whole, &filter);
        let mut summary = journey::graph_summary(&graph);
        if graph.nodes.len() != whole.nodes.len() {
            summary = fill(
                tr("{} · filtered from {} nodes"),
                &[&summary, &whole.nodes.len()],
            );
        }
        body = body.child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(muted)
                        .child(summary),
                )
                .child(
                    Button::new("journey-graph-list")
                        .icon(glyph("list"))
                        .label(tr("List"))
                        .xsmall()
                        .ghost()
                        .selected(!self.journey_graph_drawn)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.journey_graph_drawn = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("journey-graph-drawn")
                        .icon(glyph("branch"))
                        .label(tr("Graph"))
                        .xsmall()
                        .ghost()
                        .selected(self.journey_graph_drawn)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.journey_graph_drawn = true;
                            cx.notify();
                        })),
                ),
        );
        if graph.nodes.is_empty() {
            return body.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(tr("Nothing in the graph matches the filter.")),
            );
        }
        let content = if self.journey_graph_drawn {
            self.graph_canvas(&graph, cx).into_any_element()
        } else {
            self.graph_list(&graph, cx).into_any_element()
        };
        let mut row = div()
            .h_flex()
            .items_start()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(content);
        if let Some(edge) = self
            .journey_edge
            .as_ref()
            .filter(|_| self.layers.contains(Layer::JourneyEdge))
        {
            row = row.child(self.edge_details(whole, edge, cx));
        }
        body.child(row)
    }

    /// The graph as sections by kind, each node with the relations that
    /// leave it; a relation opens its details.
    fn graph_list(&self, graph: &ResearchGraph, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let mut list = div()
            .id("journey-graph-list-body")
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_3()
            .overflow_y_scroll();
        let mut link_index: usize = 0;
        for (kind, rows) in research_graph::group_by_kind(graph) {
            let mut section = div().v_flex().gap_1().child(
                div()
                    .h_flex()
                    .gap_2()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(journey::graph_kind_label(kind))
                    .child(rows.len().to_string()),
            );
            for row in rows {
                let mut card = div()
                    .v_flex()
                    .gap_0p5()
                    .p_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().child(row.node.title.clone()));
                if let Some(reference) = row
                    .node
                    .ref_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())
                {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(reference.to_owned()),
                    );
                }
                let metadata = research_graph::metadata_summary(&row.node.metadata_json);
                if !metadata.is_empty() {
                    card = card.child(div().text_xs().text_color(muted).child(metadata));
                }
                for (edge, target) in row.links {
                    let chosen = self.journey_edge.as_ref() == Some(edge);
                    let opened = edge.clone();
                    card = card.child(
                        Button::new(("journey-link", link_index))
                            .icon(glyph("arrow-right"))
                            .label(format!("{}: {target}", edge.relation))
                            .xsmall()
                            .ghost()
                            .selected(chosen)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_journey_edge(opened.clone(), cx)
                            })),
                    );
                    link_index += 1;
                }
                section = section.child(card);
            }
            list = list.child(section);
        }
        list
    }

    /// The graph drawn: columns by kind, boxes for nodes, arrows for the
    /// relations painted underneath, and each relation's name a button
    /// that opens its details. It scrolls both ways when it is larger than
    /// the page.
    fn graph_canvas(&self, graph: &ResearchGraph, cx: &mut Context<Self>) -> Stateful<Div> {
        let muted = cx.theme().muted_foreground;
        let laid = research_graph::layout(graph);
        let chosen_index = self
            .journey_edge
            .as_ref()
            .and_then(|chosen| graph.edges.iter().position(|edge| edge == chosen));
        let routes: Vec<(bool, Vec<(f32, f32)>)> = laid
            .edges
            .iter()
            .map(|edge| {
                (
                    Some(edge.edge) == chosen_index,
                    edge.route.polyline(CURVE_PIECES),
                )
            })
            .collect();
        let (line, strong) = (muted.opacity(0.7), cx.theme().primary);
        let lines = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let at =
                    |(x, y): (f32, f32)| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
                for (chosen, points) in &routes {
                    let mut path = PathBuilder::stroke(px(if *chosen { 2. } else { 1.25 }));
                    let Some((&first, rest)) = points.split_first() else {
                        continue;
                    };
                    path.move_to(at(first));
                    for &next in rest {
                        path.line_to(at(next));
                    }
                    // The arrow's head, two strokes back from the end.
                    for wing in journey::arrow_head(points) {
                        path.move_to(at(*points.last().unwrap()));
                        path.line_to(at(wing));
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, if *chosen { strong } else { line });
                    }
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let mut board = div()
            .relative()
            .flex_shrink_0()
            .w(px(laid.width as f32))
            .h(px(laid.height as f32))
            .child(lines);
        for column in &laid.columns {
            board = board.child(
                div()
                    .absolute()
                    .left(px(column.x as f32))
                    .top(px((research_graph::HEADER_Y - 16) as f32))
                    .w(px(NODE_WIDTH as f32))
                    .text_center()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(muted)
                    .child(journey::graph_kind_label(column.kind)),
            );
        }
        for place in &laid.nodes {
            let node = &graph.nodes[place.node];
            let kind = journey::graph_kind_label(&node.kind);
            let whole = format!("{kind}: {}", node.title);
            board = board.child(
                div()
                    .id(("journey-node", place.node))
                    .absolute()
                    .left(px(place.x as f32))
                    .top(px(place.y as f32))
                    .w(px(NODE_WIDTH as f32))
                    .h(px(NODE_HEIGHT as f32))
                    .v_flex()
                    .justify_center()
                    .px_3()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(div().text_xs().text_color(muted).child(kind))
                    .child(
                        div()
                            .text_sm()
                            .truncate()
                            .child(journey::shorten(&node.title, TITLE_CHARS)),
                    )
                    .tooltip(move |window, cx| Tooltip::new(whole.clone()).build(window, cx)),
            );
        }
        for place in &laid.edges {
            let edge = &graph.edges[place.edge];
            let opened = edge.clone();
            board = board.child(
                div()
                    .absolute()
                    .left(px(place.label.x as f32 - LABEL_WIDTH / 2.))
                    .top(px(place.label.y as f32 - 16.))
                    .w(px(LABEL_WIDTH))
                    .h_flex()
                    .justify_center()
                    .child(
                        Button::new(("journey-edge", place.edge))
                            .label(journey::shorten(&edge.relation, LABEL_CHARS))
                            .xsmall()
                            .ghost()
                            .bg(cx.theme().background)
                            .selected(Some(place.edge) == chosen_index)
                            .tooltip(research_graph::edge_title(graph, edge))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_journey_edge(opened.clone(), cx)
                            })),
                    ),
            );
        }
        div()
            .id("journey-graph-canvas")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_scroll()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(board)
    }

    /// One relation: which two nodes, and what its metadata says.
    fn edge_details(
        &self,
        graph: &ResearchGraph,
        edge: &ResearchEdge,
        cx: &mut Context<Self>,
    ) -> Div {
        let muted = cx.theme().muted_foreground;
        let title = |id: &str| {
            graph
                .nodes
                .iter()
                .find(|node| node.id == id)
                .map_or_else(|| id.to_owned(), |node| node.title.clone())
        };
        let mut details = div()
            .v_flex()
            .w(px(280.))
            .flex_shrink_0()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("Relationship details")),
                    )
                    .child(
                        icon_button(
                            "journey-edge-close",
                            "close",
                            tr("Close relationship details"),
                        )
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.layers.close(Layer::JourneyEdge);
                            this.journey_edge = None;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{} →", edge.relation)),
            )
            .child(div().text_sm().child(format!(
                "{} → {}",
                title(&edge.source_id),
                title(&edge.target_id)
            )));
        let metadata = research_graph::metadata_pairs(&edge.metadata_json);
        if metadata.is_empty() {
            return details.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(tr("No relationship metadata")),
            );
        }
        for (key, value) in metadata {
            details = details.child(
                div()
                    .v_flex()
                    .child(div().text_xs().text_color(muted).child(key))
                    .child(div().text_xs().child(value)),
            );
        }
        details
    }
}
