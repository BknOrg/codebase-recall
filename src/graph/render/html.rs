use crate::assets::{self, Delivery};
use crate::graph::CodeGraph;

/// A fully self-contained interactive page: inlined data + vendored d3-force
/// layout. No network requests, works offline, theme-aware.
///
/// `code-ctx serve` renders the same view from [`assets::graph_page`] with
/// [`Delivery::Server`] instead, so the two stay in lockstep.
pub fn render(graph: &CodeGraph) -> String {
    let data = serde_json::to_string(graph).unwrap_or_else(|_| "{}".to_string());
    let stat = format!(
        "{} nodes \u{00b7} {} edges",
        graph.nodes.len(),
        graph.edges.len()
    );
    assets::graph_page(&data, &stat, Delivery::Inline)
}
