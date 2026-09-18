use crate::assets::{self, Delivery};
use crate::graph::CodeGraph;

/// A fully self-contained interactive page: inlined data + vendored d3-force
/// layout. No network requests, works offline, theme-aware.
///
/// `code-rcl serve` renders the same view from [`assets::graph_page`] with
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

#[cfg(test)]
mod tests {
    use super::render;
    use crate::graph::CodeGraph;

    /// `render()` promises a self-contained page ("no network requests,
    /// works offline"); it must never emit a reference to a sibling file —
    /// that shape (`Delivery::Separated`) is what `code-rcl graph
    /// --format html` uses instead, and it writes the sibling files that
    /// mode expects. This function does not, so it must stay `Inline`.
    #[test]
    fn output_is_fully_self_contained() {
        let graph = CodeGraph {
            version: 2,
            root: ".".to_string(),
            generated_at: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
            communities: Vec::new(),
        };
        let html = render(&graph);
        assert!(!html.contains("href=\"./"), "must not link a sibling file:\n{html}");
        assert!(!html.contains("src=\"./"), "must not source a sibling file:\n{html}");
        assert!(html.contains("<style>"), "CSS must be inlined");
        assert!(html.contains("<script>"), "JS must be inlined");
    }

    /// The subsystem colouring needs both its control and the community list in the page.
    #[test]
    fn community_colouring_is_wired_in() {
        use crate::graph::CommunityInfo;

        let graph = CodeGraph {
            version: 2,
            root: ".".to_string(),
            generated_at: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
            communities: vec![CommunityInfo { id: 0, label: "src/billing".into(), size: 3 }],
        };
        let html = render(&graph);
        assert!(html.contains("id=\"colorByCommunity\""), "toggle missing");
        assert!(html.contains("function communityColor"), "colour helper missing");
        assert!(html.contains("src/billing"), "communities must be embedded in the page data");
    }
}
