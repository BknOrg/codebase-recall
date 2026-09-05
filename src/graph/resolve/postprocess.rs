//! Passes that run over the resolved node/edge lists after the graph is built:
//! file collapse, focus neighbourhood, external pruning, degree, node cap, and
//! the directory roll-up.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::graph::{dir_id, Edge, Node};

use super::parent_dir;

pub(super) fn collapse_to_files(
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    sym_node: &HashMap<i64, (String, String)>,
) {
    let sym_to_file: HashMap<&str, &str> = sym_node
        .values()
        .map(|(s, f)| (s.as_str(), f.as_str()))
        .collect();
    nodes.retain(|n| n.kind == "file" || n.kind == "external");

    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut out = Vec::new();
    for e in edges.drain(..) {
        if e.kind == "contains" {
            continue;
        }
        let s = sym_to_file.get(e.source.as_str()).map(|s| s.to_string()).unwrap_or(e.source);
        let t = sym_to_file.get(e.target.as_str()).map(|s| s.to_string()).unwrap_or(e.target);
        if s == t {
            continue;
        }
        if seen.insert((s.clone(), t.clone(), e.kind.clone())) {
            out.push(Edge {
                source: s,
                target: t,
                kind: e.kind,
                confidence: e.confidence,
                external: e.external,
            });
        }
    }
    *edges = out;
}

pub(super) fn apply_focus(nodes: &mut Vec<Node>, edges: &mut Vec<Edge>, focus: &str, depth: u32) {
    let seeds: HashSet<&str> = nodes
        .iter()
        .filter(|n| n.label == focus || n.id.contains(focus))
        .map(|n| n.id.as_str())
        .collect();
    if seeds.is_empty() {
        return;
    }

    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in edges.iter() {
        adj.entry(&e.source).or_default().push(&e.target);
        adj.entry(&e.target).or_default().push(&e.source);
    }

    let mut keep: HashSet<String> = seeds.iter().map(|s| s.to_string()).collect();
    let mut frontier: VecDeque<(String, u32)> =
        seeds.iter().map(|s| (s.to_string(), 0)).collect();
    while let Some((id, d)) = frontier.pop_front() {
        if d >= depth {
            continue;
        }
        if let Some(neigh) = adj.get(id.as_str()) {
            for n in neigh {
                if keep.insert(n.to_string()) {
                    frontier.push_back((n.to_string(), d + 1));
                }
            }
        }
    }

    nodes.retain(|n| keep.contains(&n.id));
    edges.retain(|e| keep.contains(&e.source) && keep.contains(&e.target));
}

pub(super) fn prune_unreferenced_externals(
    nodes: &mut Vec<Node>,
    edges: &[Edge],
    include_external: bool,
) {
    if !include_external {
        nodes.retain(|n| n.kind != "external");
        return;
    }
    let used: HashSet<&str> = edges
        .iter()
        .flat_map(|e| [e.source.as_str(), e.target.as_str()])
        .collect();
    nodes.retain(|n| n.kind != "external" || used.contains(n.id.as_str()));
}

/// Incident relation-edge count per node id. `contains` is structural, so it
/// doesn't count toward a node's "how connected is this" degree.
pub(super) fn degree_map(edges: &[Edge]) -> HashMap<String, u32> {
    let mut m: HashMap<String, u32> = HashMap::new();
    for e in edges {
        if e.kind == "contains" {
            continue;
        }
        *m.entry(e.source.clone()).or_default() += 1;
        *m.entry(e.target.clone()).or_default() += 1;
    }
    m
}

/// Drop the lowest-degree symbol nodes until the graph fits in `max_nodes`.
/// Files, dirs and externals are always kept; edges that lose an endpoint go
/// with them. `max_nodes == 0` disables the cap.
pub(super) fn enforce_max_nodes(
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    degree: &HashMap<String, u32>,
    max_nodes: usize,
) {
    if max_nodes == 0 || nodes.len() <= max_nodes {
        return;
    }
    let mut syms: Vec<&Node> = nodes.iter().filter(|n| n.id.starts_with("sym:")).collect();
    let structural = nodes.len() - syms.len();

    // How many symbols to drop: everything past the leftover budget, and all of
    // them if the structural nodes alone already blow the cap.
    let drop_count = match max_nodes.checked_sub(structural) {
        Some(budget) => syms.len().saturating_sub(budget),
        None => syms.len(),
    };
    syms.sort_by_key(|n| degree.get(n.id.as_str()).copied().unwrap_or(0));
    let doomed: HashSet<String> = syms
        .into_iter()
        .take(drop_count)
        .map(|n| n.id.clone())
        .collect();

    nodes.retain(|n| !doomed.contains(&n.id));
    edges.retain(|e| !doomed.contains(&e.source) && !doomed.contains(&e.target));
    if !doomed.is_empty() {
        eprintln!(
            "graph: dropped {} low-degree symbol nodes to stay under --max-nodes {max_nodes}",
            doomed.len()
        );
    }
}

/// Add a `dir:` node for every directory that holds a kept file, plus `contains`
/// edges dir→dir and dir→file. This is what lets the viewer open on a
/// folder-level graph and expand down into files and symbols.
pub(super) fn rollup_directories(nodes: &mut Vec<Node>, edges: &mut Vec<Edge>) {
    let mut dirs: HashSet<String> = HashSet::new();
    let mut contains: Vec<(String, String)> = Vec::new();

    let file_dirs: Vec<(String, String)> = nodes
        .iter()
        .filter(|n| n.kind == "file")
        .filter_map(|n| {
            n.path
                .as_deref()
                .and_then(parent_dir)
                .map(|d| (n.id.clone(), d))
        })
        .collect();

    for (fid, dir) in &file_dirs {
        contains.push((dir_id(dir), fid.clone()));
        let mut cur = dir.clone();
        // Walk the ancestor chain: "a/b/c" -> "a/b" -> "a".
        while dirs.insert(cur.clone()) {
            match parent_dir(&cur) {
                Some(parent) => {
                    contains.push((dir_id(&parent), dir_id(&cur)));
                    cur = parent;
                }
                None => break,
            }
        }
    }

    for d in &dirs {
        nodes.push(Node {
            id: dir_id(d),
            kind: "dir".into(),
            label: d.clone(),
            path: Some(d.clone()),
            dir: parent_dir(d),
            language: None,
            exported: true,
            lines: None,
            degree: None,
        });
    }

    let mut seen: HashSet<(String, String)> = HashSet::new();
    for (parent, child) in contains {
        if seen.insert((parent.clone(), child.clone())) {
            edges.push(Edge {
                source: parent,
                target: child,
                kind: "contains".into(),
                confidence: 1.0,
                external: None,
            });
        }
    }
}
