//! Lookups over a built [`CodeGraph`], shared by `impact`, `path` and `explain`.

use std::collections::HashMap;

use super::{CodeGraph, Edge, Node};

/// Adjacency lists keyed by node id: edges arriving at, and leaving, each node.
pub struct Adjacency<'a> {
    pub incoming: HashMap<&'a str, Vec<&'a Edge>>,
    pub outgoing: HashMap<&'a str, Vec<&'a Edge>>,
}

/// Every node whose label, id, or path matches `query`. Several nodes can match
/// (e.g. a `run` function in many files); callers decide how to treat that.
pub fn find_targets<'a>(graph: &'a CodeGraph, query: &str) -> Vec<&'a Node> {
    graph
        .nodes
        .iter()
        .filter(|n| {
            n.label == query
                || n.id == query
                || n.id.ends_with(&format!("::{query}"))
                || n.path.as_deref() == Some(query)
                || n.path.as_deref().is_some_and(|p| p.ends_with(query))
        })
        .collect()
}

/// Build incoming/outgoing adjacency over the edges whose kind is in `kinds`
/// (all edges when `kinds` is empty).
pub fn build_adjacency<'a>(graph: &'a CodeGraph, kinds: &[String]) -> Adjacency<'a> {
    let mut incoming: HashMap<&str, Vec<&Edge>> = HashMap::new();
    let mut outgoing: HashMap<&str, Vec<&Edge>> = HashMap::new();
    for edge in &graph.edges {
        if kinds.is_empty() || kinds.iter().any(|k| k == &edge.kind) {
            incoming.entry(&edge.target).or_default().push(edge);
            outgoing.entry(&edge.source).or_default().push(edge);
        }
    }
    Adjacency { incoming, outgoing }
}
