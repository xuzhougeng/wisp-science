use crate::app_support::compose_icon;
use crate::bindings::invoke;
use crate::dto::research_graph::{self as shared, metadata_summary};
use crate::dto::{ResearchEdge, ResearchGraph, ResearchNode};
use crate::i18n::{t, tf, Locale};
use crate::window_capture_escape;
use leptos::*;
use std::collections::HashMap;
use wasm_bindgen::JsValue;

pub(super) use crate::dto::research_graph::metadata_pairs;

const GRAPH_NODE_WIDTH: i32 = shared::NODE_WIDTH;
const GRAPH_NODE_HEIGHT: i32 = shared::NODE_HEIGHT;
const GRAPH_HEADER_Y: i32 = shared::HEADER_Y;

/// A node plus its outgoing edges and their resolved target labels.
pub(super) struct GraphRow {
    pub(super) node: ResearchNode,
    pub(super) links: Vec<(ResearchEdge, String)>,
}

/// Bucket nodes by kind and hang each node's outgoing edges off it, in the
/// order and grouping both desktops share (`wisp_dto::research_graph`).
///
/// Both endpoints are guaranteed to name a node in the same project — the store
/// counts them before inserting an edge (`Store::save_research_edge`) and every
/// delete path drops edges before their nodes — so no edge goes unrendered.
pub(super) fn group_graph(graph: &ResearchGraph) -> Vec<(&'static str, Vec<GraphRow>)> {
    shared::group_by_kind(graph)
        .into_iter()
        .map(|(kind, rows)| {
            (
                kind_label_key(kind),
                rows.into_iter()
                    .map(|row| GraphRow {
                        node: row.node.clone(),
                        links: row
                            .links
                            .into_iter()
                            .map(|(edge, target)| (edge.clone(), target))
                            .collect(),
                    })
                    .collect(),
            )
        })
        .collect()
}

#[derive(Debug)]
struct GraphColumnLayout {
    label_key: &'static str,
    x: i32,
}

#[derive(Debug)]
struct GraphNodeLayout {
    node: ResearchNode,
    x: i32,
    y: i32,
}

#[derive(Debug)]
struct GraphEdgeLayout {
    edge: ResearchEdge,
    title: String,
    path: String,
    label_x: i32,
    label_y: i32,
}

#[derive(Debug)]
struct GraphLayout {
    width: i32,
    height: i32,
    columns: Vec<GraphColumnLayout>,
    nodes: Vec<GraphNodeLayout>,
    edges: Vec<GraphEdgeLayout>,
}

/// The shared fixed-column layout (`wisp_dto::research_graph::layout`),
/// with each route written as an SVG path.
fn layout_graph(graph: &ResearchGraph) -> GraphLayout {
    let shared = shared::layout(graph);
    GraphLayout {
        width: shared.width,
        height: shared.height,
        columns: shared
            .columns
            .into_iter()
            .map(|column| GraphColumnLayout {
                label_key: kind_label_key(column.kind),
                x: column.x,
            })
            .collect(),
        nodes: shared
            .nodes
            .into_iter()
            .map(|place| GraphNodeLayout {
                node: graph.nodes[place.node].clone(),
                x: place.x,
                y: place.y,
            })
            .collect(),
        edges: shared
            .edges
            .into_iter()
            .map(|place| {
                let edge = &graph.edges[place.edge];
                GraphEdgeLayout {
                    edge: edge.clone(),
                    title: shared::edge_title(graph, edge),
                    path: place.route.svg_path(),
                    label_x: place.label.x,
                    label_y: place.label.y,
                }
            })
            .collect(),
    }
}

fn truncate_label(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars()
        .take(max_chars.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

/// The i18n key of a node kind's name. The kinds are `shared::KINDS`.
fn kind_label_key(kind: &str) -> &'static str {
    match kind {
        "decision" => "graph.kind.decision",
        "paper" => "graph.kind.paper",
        "data_asset" => "graph.kind.data_asset",
        "run" => "graph.kind.run",
        "artifact" => "graph.kind.artifact",
        _ => "graph.title",
    }
}

fn empty_graph(locale: Locale) -> View {
    view! {
        <div class="rp-empty research-graph-empty">
            <span class="rp-empty-icon brand" aria-hidden="true"></span>
            <div class="rp-empty-title">{t(locale, "graph.empty.title")}</div>
            <p>{t(locale, "graph.empty.body")}</p>
        </div>
    }
    .into_view()
}

pub(super) fn research_graph_list(
    locale: Locale,
    graph: &ResearchGraph,
    selected_edge: RwSignal<Option<ResearchEdge>>,
) -> View {
    let sections = group_graph(graph);
    view! {
        <div class="research-graph-list" data-testid="research-graph-list">
            {sections.into_iter().map(|(label_key, rows)| view! {
                <section class="control-section">
                    <div class="control-section-head">
                        <span>{t(locale, label_key)}</span>
                        <span class="control-count">{rows.len().to_string()}</span>
                    </div>
                    {rows.into_iter().map(|row| view! {
                        <article class="graph-node">
                            <div class="graph-node-title">{row.node.title}</div>
                            {row.node.ref_id
                                .filter(|id| !id.trim().is_empty())
                                .map(|id| view! { <div class="graph-node-ref">{id}</div> })}
                            {{
                                let meta = metadata_summary(&row.node.metadata_json);
                                (!meta.is_empty()).then(|| view! {
                                    <div class="graph-node-ref graph-node-meta">{meta}</div>
                                })
                            }}
                            {(!row.links.is_empty()).then(|| view! {
                                <ul class="graph-node-links">
                                    {row.links.into_iter().map(|(edge, target)| {
                                        let metadata = metadata_summary(&edge.metadata_json);
                                        let edge_label = format!("{}: {target}", edge.relation);
                                        let selected = edge.clone();
                                        view! {
                                            <li>
                                                <button type="button" class="graph-edge-summary"
                                                    aria-label=edge_label
                                                    on:click=move |_| selected_edge.set(Some(selected.clone()))>
                                                    <span class="graph-rel">{edge.relation}{compose_icon("arrow-right")}</span>
                                                    <span class="graph-target">{target}</span>
                                                </button>
                                                {(!metadata.is_empty()).then(|| view! {
                                                    <div class="graph-edge-meta">{metadata}</div>
                                                })}
                                            </li>
                                        }
                                    }).collect_view()}
                                </ul>
                            })}
                        </article>
                    }).collect_view()}
                </section>
            }).collect_view()}
        </div>
    }
    .into_view()
}

pub(super) fn research_graph_canvas(
    locale: Locale,
    graph: &ResearchGraph,
    selected_edge: RwSignal<Option<ResearchEdge>>,
) -> View {
    let layout = layout_graph(graph);
    let view_box = format!("0 0 {} {}", layout.width, layout.height);
    let style = format!("width:{}px;height:{}px", layout.width, layout.height);
    view! {
        <div class="research-graph-canvas" data-testid="research-graph-canvas">
            <svg viewBox=view_box style=style role="img"
                aria-labelledby="research-graph-canvas-title research-graph-canvas-desc">
                <title id="research-graph-canvas-title">{t(locale, "graph.canvas.title")}</title>
                <desc id="research-graph-canvas-desc">{t(locale, "graph.canvas.description")}</desc>
                <defs>
                    <marker id="research-graph-arrow" markerWidth="8" markerHeight="8"
                        refX="7" refY="4" orient="auto" markerUnits="strokeWidth">
                        <path d="M 0 0 L 8 4 L 0 8 z"></path>
                    </marker>
                </defs>
                <g class="research-graph-edges">
                    {layout.edges.into_iter().map(|edge| {
                        let label = truncate_label(&edge.edge.relation, 18);
                        let selected_for_click = edge.edge.clone();
                        let selected_for_key = edge.edge.clone();
                        let selected_for_class = edge.edge.clone();
                        view! {
                            <g class="research-graph-edge-group" role="button" tabindex="0"
                                class:selected=move || selected_edge.get().as_ref() == Some(&selected_for_class)
                                aria-label=edge.title.clone()
                                on:click=move |_| selected_edge.set(Some(selected_for_click.clone()))
                                on:keydown=move |event| {
                                    if matches!(event.key().as_str(), "Enter" | " ") {
                                        event.prevent_default();
                                        selected_edge.set(Some(selected_for_key.clone()));
                                    }
                                }>
                                <title>{edge.title}</title>
                                <path class="research-graph-edge" d=edge.path.clone()
                                    marker-end="url(#research-graph-arrow)"></path>
                                <path class="research-graph-edge-hit" d=edge.path></path>
                                <text class="research-graph-edge-label" x=edge.label_x y=edge.label_y
                                    text-anchor="middle">{label}</text>
                            </g>
                        }
                    }).collect_view()}
                </g>
                <g class="research-graph-column-labels">
                    {layout.columns.into_iter().map(|column| view! {
                        <text x=column.x + GRAPH_NODE_WIDTH / 2 y=GRAPH_HEADER_Y
                            text-anchor="middle">{t(locale, column.label_key)}</text>
                    }).collect_view()}
                </g>
                <g class="research-graph-nodes">
                    {layout.nodes.into_iter().map(|position| {
                        let kind = position.node.kind.clone();
                        let kind_label = t(locale, kind_label_key(&kind));
                        let title = position.node.title.clone();
                        let short_title = truncate_label(&title, 24);
                        view! {
                            <g class=format!("research-graph-node {kind}")
                                data-node-id=position.node.id
                                transform=format!("translate({} {})", position.x, position.y)>
                                <title>{format!("{kind_label}: {title}")}</title>
                                <rect width=GRAPH_NODE_WIDTH height=GRAPH_NODE_HEIGHT rx="10"></rect>
                                <text class="research-graph-node-kind" x="12" y="19">{kind_label}</text>
                                <text class="research-graph-node-title" x="12" y="42">{short_title}</text>
                            </g>
                        }
                    }).collect_view()}
                </g>
            </svg>
        </div>
    }
    .into_view()
}

pub(super) fn research_edge_detail(
    locale: Locale,
    graph: &ResearchGraph,
    edge: ResearchEdge,
    selected_edge: RwSignal<Option<ResearchEdge>>,
) -> View {
    let node_titles = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.title.as_str()))
        .collect::<HashMap<_, _>>();
    let source = node_titles
        .get(edge.source_id.as_str())
        .copied()
        .unwrap_or(edge.source_id.as_str())
        .to_string();
    let target = node_titles
        .get(edge.target_id.as_str())
        .copied()
        .unwrap_or(edge.target_id.as_str())
        .to_string();
    let metadata = metadata_pairs(&edge.metadata_json);
    view! {
        <aside class="research-edge-detail" data-testid="research-edge-detail"
            aria-label=t(locale, "graph.edge.details")>
            <div class="research-edge-detail-head">
                <div>
                    <span class="graph-rel">{edge.relation}{compose_icon("arrow-right")}</span>
                    <strong>{format!("{source} → {target}")}</strong>
                </div>
                <button type="button" class="ps-close"
                    title=t(locale, "graph.edge.close")
                    aria-label=t(locale, "graph.edge.close")
                    on:click=move |_| selected_edge.set(None)>{compose_icon("close")}</button>
            </div>
            {if metadata.is_empty() {
                view! { <p>{t(locale, "graph.edge.no_metadata")}</p> }.into_view()
            } else {
                view! {
                    <dl>
                        {metadata.into_iter().map(|(key, value)| view! {
                            <div><dt>{key}</dt><dd>{value}</dd></div>
                        }).collect_view()}
                    </dl>
                }.into_view()
            }}
        </aside>
    }
    .into_view()
}

/// Project-level research graph opened from the left sidebar. The modal keeps a
/// dense list for exact inspection and a dependency-free SVG overview.
#[component]
pub(super) fn ResearchGraphModal(
    locale: ReadSignal<Locale>,
    graph: ReadSignal<ResearchGraph>,
    on_close: Callback<()>,
) -> impl IntoView {
    let graph_view = create_rw_signal(false);
    let selected_edge = create_rw_signal::<Option<ResearchEdge>>(None);
    window_capture_escape(move || {
        if selected_edge.get_untracked().is_none() {
            return false;
        }
        selected_edge.set(None);
        true
    });
    view! {
        <div class="overlay research-graph-overlay" role="presentation"
            on:click=move |_| on_close.call(())>
            <section class="modal research-graph-modal" role="dialog" aria-modal="true"
                data-testid="research-graph-modal"
                aria-labelledby="research-graph-title" aria-describedby="research-graph-summary"
                tabindex="-1"
                on:click=|event| event.stop_propagation()>
                <header class="research-graph-head">
                    <div class="research-graph-heading">
                        <h2 id="research-graph-title">{move || t(locale.get(), "graph.title")}</h2>
                        <p id="research-graph-summary">{move || {
                            let current = graph.get();
                            let nodes = current.nodes.len().to_string();
                            let edges = current.edges.len().to_string();
                            tf(locale.get(), "graph.summary", &[("nodes", &nodes), ("edges", &edges)])
                        }}</p>
                    </div>
                    <div class="research-graph-head-actions">
                        <div class="research-graph-view-modes" role="tablist"
                            aria-label=move || t(locale.get(), "graph.view.label")>
                            <button type="button" role="tab"
                                class:active=move || !graph_view.get()
                                aria-selected=move || (!graph_view.get()).to_string()
                                aria-controls="research-graph-content"
                                on:click=move |_| graph_view.set(false)>
                                {compose_icon("list")}
                                <span>{move || t(locale.get(), "graph.view.list")}</span>
                            </button>
                            <button type="button" role="tab"
                                class:active=move || graph_view.get()
                                aria-selected=move || graph_view.get().to_string()
                                aria-controls="research-graph-content"
                                on:click=move |_| graph_view.set(true)>
                                {compose_icon("branch")}
                                <span>{move || t(locale.get(), "graph.view.graph")}</span>
                            </button>
                        </div>
                        <button type="button" class="ps-close"
                            title=move || t(locale.get(), "graph.close")
                            aria-label=move || t(locale.get(), "graph.close")
                            on:click=move |_| on_close.call(())>{compose_icon("close")}</button>
                    </div>
                </header>
                <div id="research-graph-content" class="research-graph-content">
                    {move || {
                        let current = graph.get();
                        let loc = locale.get();
                        if current.nodes.is_empty() {
                            empty_graph(loc)
                        } else if graph_view.get() {
                            research_graph_canvas(loc, &current, selected_edge)
                        } else {
                            research_graph_list(loc, &current, selected_edge)
                        }
                    }}
                </div>
                {move || selected_edge.get().map(|edge| {
                    research_edge_detail(locale.get(), &graph.get(), edge, selected_edge)
                })}
            </section>
        </div>
    }
}

pub(super) fn refresh_research_graph(graph: RwSignal<ResearchGraph>) {
    spawn_local(async move {
        let value = invoke("get_research_graph", JsValue::UNDEFINED).await;
        if let Ok(next) = serde_wasm_bindgen::from_value::<ResearchGraph>(value) {
            graph.set(next);
        }
    });
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
    fn groups_by_kind_and_resolves_link_targets() {
        let graph = ResearchGraph {
            nodes: vec![
                node("d1", "decision", "Use DESeq2"),
                node("a1", "data_asset", "counts.tsv"),
            ],
            edges: vec![edge("d1", "a1", "uses")],
        };
        let sections = group_graph(&graph);
        // Decision sorts before data_asset, matching KINDS order.
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].0, "graph.kind.decision");
        assert_eq!(sections[1].0, "graph.kind.data_asset");
        assert_eq!(
            sections[0].1[0].links,
            vec![(edge("d1", "a1", "uses"), "counts.tsv".to_string())]
        );
        // The link hangs off the source only; the target repeats nothing.
        assert!(sections[1].1[0].links.is_empty());
    }

    #[test]
    fn grouping_keeps_node_metadata() {
        let mut asset = node("a1", "data_asset", "counts.tsv");
        asset.metadata_json = r#"{"rows": 1204}"#.into();
        let graph = ResearchGraph {
            nodes: vec![asset],
            edges: vec![],
        };
        let sections = group_graph(&graph);
        assert_eq!(sections[0].1[0].node.metadata_json, r#"{"rows": 1204}"#);
    }

    #[test]
    fn metadata_pairs_formats_values_and_skips_non_objects() {
        assert_eq!(
            metadata_pairs(r#"{"doi": "10.1/x", "rows": 1204, "tags": ["a"]}"#),
            vec![
                ("doi".to_string(), "10.1/x".to_string()),
                ("rows".to_string(), "1204".to_string()),
                ("tags".to_string(), r#"["a"]"#.to_string()),
            ]
        );
        assert!(metadata_pairs("{}").is_empty());
        assert!(metadata_pairs("not json").is_empty());
        assert!(metadata_pairs(r#"["an", "array"]"#).is_empty());
    }

    #[test]
    fn graph_layout_groups_kinds_and_connects_known_nodes() {
        let graph = ResearchGraph {
            nodes: vec![
                node("d1", "decision", "Use DESeq2"),
                node("p1", "paper", "Love et al. 2014"),
                node("a1", "data_asset", "counts.tsv"),
                node("a2", "data_asset", "samples.tsv"),
                node("r1", "run", "Differential expression"),
            ],
            edges: vec![
                edge("d1", "a1", "applies to"),
                edge("missing", "r1", "ignored"),
            ],
        };
        let layout = layout_graph(&graph);
        let data_nodes = layout
            .nodes
            .iter()
            .filter(|node| node.node.kind == "data_asset")
            .collect::<Vec<_>>();

        assert_eq!(layout.columns.len(), 4);
        assert_eq!(data_nodes.len(), 2);
        assert_eq!(data_nodes[0].x, data_nodes[1].x);
        assert_ne!(data_nodes[0].y, data_nodes[1].y);
        assert_eq!(layout.edges.len(), 1);
        assert!(layout.edges[0].path.starts_with("M "));
        assert!(layout.edges[0].path.contains(" L "));
        assert_eq!(layout.edges[0].edge.relation, "applies to");
    }

    #[test]
    fn graph_layout_keeps_edge_metadata_in_its_tooltip() {
        let mut link = edge("d1", "p1", "cites");
        link.metadata_json = r#"{"confidence":"high"}"#.into();
        let graph = ResearchGraph {
            nodes: vec![
                node("d1", "decision", "Use DESeq2"),
                node("p1", "paper", "Love et al. 2014"),
            ],
            edges: vec![link.clone()],
        };
        let layout = layout_graph(&graph);

        assert_eq!(layout.edges[0].edge, link);
        assert!(layout.edges[0].title.contains("confidence: high"));
    }
}
