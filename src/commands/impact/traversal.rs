use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::commands::impact::models::ImpactItem;
use crate::graph::{Edge, Node};

const LOW_CONFIDENCE_THRESHOLD: f32 = 0.7;

/// First `/`-separated component of a path's *directory* (e.g. `src` for
/// `src/commands/impact/mod.rs`, `""` for a root-level file like `main.rs`).
/// Used as a coarse "module" grouping for the `crosses_module` risk signal.
pub fn module_root(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((dir, _file)) => dir.split('/').next().unwrap_or(""),
        None => "",
    }
}

/// What a traversal compares each item against to decide the risk flags.
pub struct RiskContext<'a> {
    /// Top-level directory of the target (fallback when communities are unavailable).
    pub module: &'a str,
    pub community: Option<u32>,
    /// Community id -> label.
    pub labels: &'a HashMap<u32, String>,
}

pub fn get_snippet(
    project_root: &Path,
    file_lines_cache: &mut HashMap<String, Vec<String>>,
    rel_path: Option<&str>,
    line: Option<i64>,
) -> Option<String> {
    let p = rel_path?;
    let l = line?;
    if l <= 0 {
        return None;
    }
    let lines = file_lines_cache.entry(p.to_string()).or_insert_with(|| {
        let abs = project_root.join(p);
        fs::read_to_string(&abs)
            .map(|c| c.lines().map(String::from).collect())
            .unwrap_or_default()
    });
    let idx = (l - 1) as usize;
    lines.get(idx).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn calculate_max_depth(items: &[ImpactItem], max_d: &mut u32) {
    for item in items {
        if item.depth > *max_d {
            *max_d = item.depth;
        }
        calculate_max_depth(&item.callers, max_d);
    }
}

/// Count distinct affected items (by id) that are exported, low-confidence,
/// or cross into a different module than the traversal target.
pub fn count_risky(items: &[ImpactItem], seen: &mut HashSet<String>, count: &mut usize) {
    for item in items {
        if seen.insert(item.id.clone())
            && (item.is_exported || item.low_confidence || item.crosses_module)
        {
            *count += 1;
        }
        count_risky(&item.callers, seen, count);
    }
}

pub fn traverse_tree(
    current_id: &str,
    depth: u32,
    max_depth: u32,
    adj: &HashMap<&str, Vec<&Edge>>,
    node_by_id: &HashMap<&str, &Node>,
    ancestors: &mut HashSet<String>,
    total_affected: &mut HashSet<String>,
    is_outgoing: bool,
    project_root: &Path,
    file_lines_cache: &mut HashMap<String, Vec<String>>,
    ctx: &RiskContext,
) -> Vec<ImpactItem> {
    if depth >= max_depth {
        return Vec::new();
    }

    let mut items = Vec::new();
    if let Some(edges) = adj.get(current_id) {
        for edge in edges {
            let next_id = if is_outgoing {
                edge.target.as_str()
            } else {
                edge.source.as_str()
            };

            if ancestors.contains(next_id) {
                continue;
            }

            if let Some(next_node) = node_by_id.get(next_id) {
                total_affected.insert(next_node.id.clone());
                ancestors.insert(next_id.to_string());

                let sub = traverse_tree(
                    next_id,
                    depth + 1,
                    max_depth,
                    adj,
                    node_by_id,
                    ancestors,
                    total_affected,
                    is_outgoing,
                    project_root,
                    file_lines_cache,
                    ctx,
                );

                ancestors.remove(next_id);

                let snippet_path = if is_outgoing {
                    node_by_id.get(current_id).and_then(|n| n.path.as_deref())
                } else {
                    next_node.path.as_deref()
                };

                let snippet = get_snippet(project_root, file_lines_cache, snippet_path, edge.line);

                items.push(ImpactItem {
                    id: next_node.id.clone(),
                    label: next_node.label.clone(),
                    kind: next_node.kind.clone(),
                    path: next_node.path.clone(),
                    line: edge.line,
                    site_path: snippet_path.map(str::to_string),
                    edge_kind: edge.kind.clone(),
                    confidence: edge.confidence,
                    snippet,
                    depth: depth + 1,
                    is_exported: next_node.exported,
                    low_confidence: edge.confidence <= LOW_CONFIDENCE_THRESHOLD,
                    crosses_module: match (ctx.community, next_node.community) {
                        (Some(target), Some(item)) => target != item,
                        _ => next_node
                            .path
                            .as_deref()
                            .map(|p| module_root(p) != ctx.module)
                            .unwrap_or(false),
                    },
                    community: next_node
                        .community
                        .and_then(|c| ctx.labels.get(&c).cloned()),
                    callers: sub,
                });
            }
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_root_groups_by_top_level_directory() {
        assert_eq!(module_root("src/commands/impact/mod.rs"), "src");
        assert_eq!(module_root("src/graph/mod.rs"), "src");
        assert_eq!(module_root("main.rs"), "");
        assert_eq!(module_root("util.rs"), "");
    }
}
