//! File-level community detection (Louvain) over the relation graph.
//!
//! Files are the unit: an `imports`/`calls`/`references` edge between two symbols
//! counts as a link between their files. Symbols later inherit their file's
//! community. Everything is deterministic (sorted ids, fixed iteration order,
//! lowest-id tie-breaks) because the graph's edge order is not.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{CodeGraph, Edge, Node, file_id};

/// Detect communities over a whole built graph.
pub fn detect_graph(graph: &CodeGraph) -> Communities {
    detect(&graph.nodes, &graph.edges)
}

const MAX_PASSES: usize = 20;
const MAX_LEVELS: usize = 10;
const EPSILON: f64 = 1e-9;
/// Louvain resolution: above 1 favours smaller subsystems. 1.0 lumps everything that depends on a
/// shared hub (e.g. every command on the cache) into one blob; values much above 2 fragment.
const RESOLUTION: f64 = 1.5;
const RELATION_KINDS: [&str; 3] = ["imports", "calls", "references"];

/// One detected subsystem.
#[derive(Debug, Clone)]
pub struct Community {
    pub id: u32,
    pub label: String,
    pub size: usize,
    /// Most connected files (weighted degree), at most three.
    pub top_files: Vec<String>,
    /// Links between files of this community.
    pub internal_edges: usize,
    /// Links from this community's files to files of other communities.
    pub external_edges: usize,
}

#[derive(Debug, Default)]
pub struct Communities {
    /// File node id -> community id. Files without one are absent.
    pub assignment: BTreeMap<String, u32>,
    pub list: Vec<Community>,
    /// Links between two different communities, keyed `(low_id, high_id)`.
    pub links: BTreeMap<(u32, u32), usize>,
}

/// The file a graph node belongs to, as a `file:<path>` id. `None` for nodes that are
/// neither files nor path-carrying symbols (directories, externals).
pub fn file_of(node_id: &str, path: Option<&str>) -> Option<String> {
    if node_id.starts_with("file:") {
        Some(node_id.to_string())
    } else if node_id.starts_with("sym:") {
        path.map(file_id)
    } else {
        None
    }
}

/// Undirected weighted file-to-file links, keyed `(low, high)` by id. Self links dropped.
pub fn file_graph(nodes: &[Node], edges: &[Edge]) -> BTreeMap<(String, String), usize> {
    let node_file: HashMap<&str, String> = nodes
        .iter()
        .filter_map(|n| file_of(&n.id, n.path.as_deref()).map(|f| (n.id.as_str(), f)))
        .collect();

    let mut links: BTreeMap<(String, String), usize> = BTreeMap::new();
    for edge in edges {
        if !RELATION_KINDS.contains(&edge.kind.as_str()) {
            continue;
        }
        let (Some(a), Some(b)) = (
            node_file.get(edge.source.as_str()),
            node_file.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if a == b {
            continue;
        }
        let key = if a < b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
        *links.entry(key).or_insert(0) += 1;
    }
    links
}

pub fn detect(nodes: &[Node], edges: &[Edge]) -> Communities {
    let links = file_graph(nodes, edges);
    if links.is_empty() {
        return Communities::default();
    }

    let ids: Vec<String> = links
        .keys()
        .flat_map(|(a, b)| [a.clone(), b.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();

    let mut adj: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); ids.len()];
    for ((a, b), w) in &links {
        let (i, j) = (index[a.as_str()], index[b.as_str()]);
        *adj[i].entry(j).or_insert(0.0) += *w as f64;
        *adj[j].entry(i).or_insert(0.0) += *w as f64;
    }

    let raw = louvain(adj);
    finalize(&ids, &raw, &links)
}

/// Louvain modularity optimisation. Returns a community index per input node.
fn louvain(adj: Vec<BTreeMap<usize, f64>>) -> Vec<usize> {
    let n0 = adj.len();
    let mut membership: Vec<usize> = (0..n0).collect();
    let mut adj = adj;
    let mut self_w = vec![0.0_f64; n0];

    for _ in 0..MAX_LEVELS {
        let n = adj.len();
        let degree: Vec<f64> = (0..n)
            .map(|i| adj[i].values().sum::<f64>() + 2.0 * self_w[i])
            .collect();
        let m2: f64 = degree.iter().sum();
        if m2 <= 0.0 {
            break;
        }

        let mut comm: Vec<usize> = (0..n).collect();
        let mut total: Vec<f64> = degree.clone();

        for _ in 0..MAX_PASSES {
            let mut moved = false;
            for i in 0..n {
                let current = comm[i];
                let mut to_comm: BTreeMap<usize, f64> = BTreeMap::new();
                for (&j, &w) in &adj[i] {
                    *to_comm.entry(comm[j]).or_insert(0.0) += w;
                }

                total[current] -= degree[i];
                let gain = |c: usize, w: f64, total: &[f64]| w - RESOLUTION * total[c] * degree[i] / m2;

                let mut best = current;
                let mut best_gain = gain(current, to_comm.get(&current).copied().unwrap_or(0.0), &total);
                for (&c, &w) in &to_comm {
                    let g = gain(c, w, &total);
                    if g > best_gain + EPSILON {
                        best = c;
                        best_gain = g;
                    }
                }

                total[best] += degree[i];
                if best != current {
                    comm[i] = best;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }

        // Relabel compactly by first appearance so the numbering is deterministic.
        let mut relabel: HashMap<usize, usize> = HashMap::new();
        for &c in &comm {
            let next = relabel.len();
            relabel.entry(c).or_insert(next);
        }
        let k = relabel.len();
        if k == n {
            break; // nothing merged: converged
        }
        let comm: Vec<usize> = comm.iter().map(|c| relabel[c]).collect();
        for m in &mut membership {
            *m = comm[*m];
        }

        // Aggregate communities into super-nodes.
        let mut next_adj: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); k];
        let mut next_self = vec![0.0_f64; k];
        for i in 0..n {
            next_self[comm[i]] += self_w[i];
            for (&j, &w) in &adj[i] {
                if comm[i] == comm[j] {
                    if i < j {
                        next_self[comm[i]] += w;
                    }
                } else {
                    *next_adj[comm[i]].entry(comm[j]).or_insert(0.0) += w;
                }
            }
        }
        adj = next_adj;
        self_w = next_self;
    }

    membership
}

fn dir_of(file_node_id: &str) -> String {
    let path = file_node_id.strip_prefix("file:").unwrap_or(file_node_id);
    path.rfind('/').map(|i| path[..i].to_string()).unwrap_or_default()
}

fn common_dir_prefix(dirs: &[String]) -> String {
    let mut parts: Vec<&str> = match dirs.first() {
        Some(d) if !d.is_empty() => d.split('/').collect(),
        _ => return String::new(),
    };
    for d in &dirs[1..] {
        let other: Vec<&str> = d.split('/').collect();
        let keep = parts.iter().zip(&other).take_while(|(a, b)| a == b).count();
        parts.truncate(keep);
        if parts.is_empty() {
            break;
        }
    }
    parts.join("/")
}

/// Name a community after where its files live: the (up to two) directories holding the most
/// files, ignoring any that is an ancestor or descendant of one already chosen and any holding
/// under a fifth of the files. A shared prefix like `src` would name everything the same, so it
/// is only the fallback.
fn label_from_dirs(dirs: &[String]) -> String {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for d in dirs.iter().filter(|d| !d.is_empty()) {
        *count.entry(d.as_str()).or_insert(0) += 1;
    }
    let mut ranked: Vec<(&str, usize)> = count.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let related = |a: &str, b: &str| {
        a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
    };
    let mut chosen: Vec<&str> = Vec::new();
    for (dir, n) in ranked {
        if chosen.len() == 2 {
            break;
        }
        if n * 5 < dirs.len() || chosen.iter().any(|c| related(c, dir)) {
            continue;
        }
        chosen.push(dir);
    }
    if chosen.is_empty() {
        return common_dir_prefix(dirs);
    }
    chosen.sort();
    chosen.join(" + ")
}

fn finalize(
    ids: &[String],
    raw: &[usize],
    links: &BTreeMap<(String, String), usize>,
) -> Communities {
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, &c) in raw.iter().enumerate() {
        groups.entry(c).or_default().push(i);
    }

    // Communities of one file are noise: leave those files unclustered.
    let mut kept: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() >= 2).collect();
    kept.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| ids[a[0]].cmp(&ids[b[0]])));

    let mut assignment: BTreeMap<String, u32> = BTreeMap::new();
    for (cid, members) in kept.iter().enumerate() {
        for &m in members {
            assignment.insert(ids[m].clone(), cid as u32);
        }
    }

    let mut weighted_degree: HashMap<&str, usize> = HashMap::new();
    for ((a, b), w) in links {
        *weighted_degree.entry(a.as_str()).or_insert(0) += w;
        *weighted_degree.entry(b.as_str()).or_insert(0) += w;
    }

    let mut internal = vec![0usize; kept.len()];
    let mut external = vec![0usize; kept.len()];
    let mut between: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for ((a, b), w) in links {
        match (assignment.get(a), assignment.get(b)) {
            (Some(&ca), Some(&cb)) if ca == cb => internal[ca as usize] += w,
            (Some(&ca), Some(&cb)) => {
                external[ca as usize] += w;
                external[cb as usize] += w;
                *between.entry((ca.min(cb), ca.max(cb))).or_insert(0) += w;
            }
            (Some(&ca), None) => external[ca as usize] += w,
            (None, Some(&cb)) => external[cb as usize] += w,
            (None, None) => {}
        }
    }

    let mut used_labels: HashMap<String, usize> = HashMap::new();
    let mut list = Vec::with_capacity(kept.len());
    for (cid, members) in kept.iter().enumerate() {
        let mut files: Vec<String> = members.iter().map(|&m| ids[m].clone()).collect();
        files.sort();

        let mut top = files.clone();
        top.sort_by(|a, b| {
            let da = weighted_degree.get(a.as_str()).copied().unwrap_or(0);
            let db = weighted_degree.get(b.as_str()).copied().unwrap_or(0);
            db.cmp(&da).then_with(|| a.cmp(b))
        });
        top.truncate(3);

        let dirs: Vec<String> = files.iter().map(|f| dir_of(f)).collect();
        let mut label = label_from_dirs(&dirs);
        if label.is_empty() {
            label = top
                .first()
                .map(|f| f.strip_prefix("file:").unwrap_or(f).to_string())
                .unwrap_or_else(|| format!("community {cid}"));
        }
        let seen = used_labels.entry(label.clone()).or_insert(0);
        *seen += 1;
        if *seen > 1 {
            label = format!("{label} ({seen})");
        }

        list.push(Community {
            id: cid as u32,
            label,
            size: files.len(),
            top_files: top,
            internal_edges: internal[cid],
            external_edges: external[cid],
        });
    }

    Communities { assignment, list, links: between }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_node(path: &str) -> Node {
        Node {
            id: file_id(path),
            kind: "file".into(),
            label: path.into(),
            path: Some(path.into()),
            dir: None,
            language: None,
            exported: true,
            lines: None,
            degree: None,
            community: None,
        }
    }

    fn import(from: &str, to: &str) -> Edge {
        Edge {
            source: file_id(from),
            target: file_id(to),
            kind: "imports".into(),
            confidence: 1.0,
            external: None,
            line: None,
        }
    }

    fn graph(files: &[&str], edges: Vec<Edge>) -> CodeGraph {
        CodeGraph {
            version: 2,
            root: ".".into(),
            generated_at: 0,
            nodes: files.iter().map(|p| file_node(p)).collect(),
            edges,
            communities: Vec::new(),
        }
    }

    fn two_cliques() -> (Vec<&'static str>, Vec<Edge>) {
        let files = vec!["a/1.rs", "a/2.rs", "a/3.rs", "b/1.rs", "b/2.rs", "b/3.rs"];
        let mut edges = Vec::new();
        for grp in [["a/1.rs", "a/2.rs", "a/3.rs"], ["b/1.rs", "b/2.rs", "b/3.rs"]] {
            edges.push(import(grp[0], grp[1]));
            edges.push(import(grp[1], grp[2]));
            edges.push(import(grp[2], grp[0]));
        }
        edges.push(import("a/1.rs", "b/1.rs"));
        (files, edges)
    }

    #[test]
    fn two_cliques_joined_by_one_edge_split_in_two() {
        let (files, edges) = two_cliques();
        let c = detect_graph(&graph(&files, edges));

        assert_eq!(c.list.len(), 2);
        let a = c.assignment[&file_id("a/1.rs")];
        assert_eq!(a, c.assignment[&file_id("a/2.rs")]);
        assert_eq!(a, c.assignment[&file_id("a/3.rs")]);
        let b = c.assignment[&file_id("b/1.rs")];
        assert_ne!(a, b);
        assert_eq!(b, c.assignment[&file_id("b/3.rs")]);

        let labels: Vec<&str> = c.list.iter().map(|x| x.label.as_str()).collect();
        assert!(labels.contains(&"a") && labels.contains(&"b"), "{labels:?}");
        assert_eq!(c.links.values().sum::<usize>(), 1);
    }

    #[test]
    fn result_does_not_depend_on_edge_order() {
        let (files, edges) = two_cliques();
        let baseline = detect_graph(&graph(&files, edges.clone())).assignment;

        for shift in 1..edges.len() {
            let mut rotated = edges.clone();
            rotated.rotate_left(shift);
            let mut reversed = rotated.clone();
            reversed.reverse();
            assert_eq!(detect_graph(&graph(&files, rotated)).assignment, baseline);
            assert_eq!(detect_graph(&graph(&files, reversed)).assignment, baseline);
        }
    }

    #[test]
    fn isolated_files_and_empty_graphs_get_no_community() {
        let empty = detect_graph(&graph(&[], Vec::new()));
        assert!(empty.assignment.is_empty() && empty.list.is_empty());

        let g = graph(&["x.rs", "y.rs", "lonely.rs"], vec![import("x.rs", "y.rs")]);
        let c = detect_graph(&g);
        assert!(!c.assignment.contains_key(&file_id("lonely.rs")));
        assert_eq!(c.assignment.len(), 2);
    }

    #[test]
    fn duplicate_labels_are_made_unique() {
        let files = vec!["src/a.rs", "src/b.rs", "src/c.rs", "src/d.rs"];
        let edges = vec![import("src/a.rs", "src/b.rs"), import("src/c.rs", "src/d.rs")];
        let c = detect_graph(&graph(&files, edges));
        assert_eq!(c.list.len(), 2);
        let mut labels: Vec<&str> = c.list.iter().map(|x| x.label.as_str()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 2, "labels must be unique");
    }

    #[test]
    fn symbols_map_to_their_file() {
        assert_eq!(
            file_of("sym:a/1.rs#foo@3", Some("a/1.rs")),
            Some("file:a/1.rs".to_string())
        );
        assert_eq!(file_of("file:a/1.rs", Some("a/1.rs")), Some("file:a/1.rs".to_string()));
        assert_eq!(file_of("ext:serde", None), None);
        assert_eq!(file_of("dir:a", None), None);
    }
}
