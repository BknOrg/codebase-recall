use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

use crate::cli::{GraphQuery, ImpactArgs};
use crate::commands::graph::build_graph;
use crate::graph::{Edge, Node};

#[derive(Debug, Serialize)]
pub struct ImpactReport {
    pub target_symbol: String,
    pub target_id: String,
    pub target_kind: String,
    pub target_path: Option<String>,
    pub direct_callers_count: usize,
    pub total_affected_count: usize,
    pub total_affected_files: usize,
    pub max_depth_reached: u32,
    pub callers: Vec<ImpactItem>,
}

#[derive(Debug, Serialize)]
pub struct ImpactItem {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    pub edge_kind: String,
    pub depth: u32,
    pub callers: Vec<ImpactItem>,
}

pub fn run(args: ImpactArgs) -> Result<()> {
    let query = GraphQuery {
        project: args.project.clone(),
        scope: "both".to_string(),
        kinds: args.kinds.clone(),
        path: None,
        focus: None,
        depth: 2,
        min_confidence: 0.0,
        include_external: false,
        max_nodes: 0,
        no_sync: args.no_sync,
        precise: args.precise.clone(),
    };

    let graph = build_graph(&query).context("failed to build code graph for impact analysis")?;

    // Match candidate target nodes
    let targets: Vec<&Node> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.label == args.symbol
                || n.id == args.symbol
                || n.id.ends_with(&format!("::{}", args.symbol))
                || n.path.as_deref() == Some(&args.symbol)
                || (n.path.as_deref().is_some_and(|p| p.ends_with(&args.symbol)))
        })
        .collect();

    if targets.is_empty() {
        anyhow::bail!(
            "No symbol or file matching '{}' found in graph. Run `code-rcl graph` to inspect available symbols.",
            args.symbol
        );
    }

    // Build reverse adjacency list: target -> incoming edges (callers/importers)
    let mut incoming: HashMap<&str, Vec<&Edge>> = HashMap::new();
    for edge in &graph.edges {
        if args.kinds.is_empty() || args.kinds.iter().any(|k| k == &edge.kind) {
            incoming.entry(&edge.target).or_default().push(edge);
        }
    }

    let node_by_id: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    for target in targets {
        let mut ancestors = HashSet::new();
        let mut total_affected = HashSet::new();
        ancestors.insert(target.id.clone());

        let callers = traverse_impact(
            &target.id,
            0,
            args.depth,
            &incoming,
            &node_by_id,
            &mut ancestors,
            &mut total_affected,
        );

        let direct_callers_count = callers.len();
        let total_affected_count = total_affected.len();

        let affected_files: HashSet<&str> = total_affected
            .iter()
            .filter_map(|id| node_by_id.get(id.as_str()).and_then(|n| n.path.as_deref()))
            .collect();
        let total_affected_files = affected_files.len();

        let mut max_depth_reached = 0;
        calculate_max_depth(&callers, &mut max_depth_reached);

        let report = ImpactReport {
            target_symbol: target.label.clone(),
            target_id: target.id.clone(),
            target_kind: target.kind.clone(),
            target_path: target.path.clone(),
            direct_callers_count,
            total_affected_count,
            total_affected_files,
            max_depth_reached,
            callers,
        };

        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print_ascii_report(&report);
        }
    }

    Ok(())
}

fn traverse_impact(
    current_id: &str,
    depth: u32,
    max_depth: u32,
    incoming: &HashMap<&str, Vec<&Edge>>,
    node_by_id: &HashMap<&str, &Node>,
    ancestors: &mut HashSet<String>,
    total_affected: &mut HashSet<String>,
) -> Vec<ImpactItem> {
    if depth >= max_depth {
        return Vec::new();
    }

    let mut items = Vec::new();
    if let Some(edges) = incoming.get(current_id) {
        for edge in edges {
            let caller_id = edge.source.as_str();
            if ancestors.contains(caller_id) {
                // Cycle detected, avoid infinite loop
                continue;
            }

            if let Some(caller_node) = node_by_id.get(caller_id) {
                total_affected.insert(caller_node.id.clone());
                ancestors.insert(caller_id.to_string());

                let sub_callers = traverse_impact(
                    caller_id,
                    depth + 1,
                    max_depth,
                    incoming,
                    node_by_id,
                    ancestors,
                    total_affected,
                );

                ancestors.remove(caller_id);

                items.push(ImpactItem {
                    id: caller_node.id.clone(),
                    label: caller_node.label.clone(),
                    kind: caller_node.kind.clone(),
                    path: caller_node.path.clone(),
                    line: edge.line,
                    edge_kind: edge.kind.clone(),
                    depth: depth + 1,
                    callers: sub_callers,
                });
            }
        }
    }

    items
}

fn calculate_max_depth(items: &[ImpactItem], max_d: &mut u32) {
    for item in items {
        if item.depth > *max_d {
            *max_d = item.depth;
        }
        calculate_max_depth(&item.callers, max_d);
    }
}

fn print_ascii_report(report: &ImpactReport) {
    let loc = report
        .target_path
        .as_deref()
        .map(|p| format!(" ({p})"))
        .unwrap_or_default();

    println!(
        "Impact Analysis for: {} [{}]",
        report.target_symbol, report.target_kind
    );
    println!(
        "Direct callers: {} | Total affected: {} symbols across {} files | Max depth: {}\n",
        report.direct_callers_count,
        report.total_affected_count,
        report.total_affected_files,
        report.max_depth_reached
    );

    println!("{}{loc}", report.target_symbol);
    if report.callers.is_empty() {
        println!("└── (no incoming callers or references found)");
    } else {
        for (i, caller) in report.callers.iter().enumerate() {
            let is_last = i + 1 == report.callers.len();
            print_tree(caller, "", is_last);
        }
    }
    println!();
}

fn print_tree(item: &ImpactItem, prefix: &str, is_last: bool) {
    let branch = if is_last { "└── " } else { "├── " };
    let loc = match (&item.path, item.line) {
        (Some(p), Some(l)) => format!(" ({p}:{l})"),
        (Some(p), None) => format!(" ({p})"),
        _ => String::new(),
    };

    println!(
        "{prefix}{branch}{} [{}] - {}{loc}",
        item.label, item.edge_kind, item.kind
    );

    let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });
    for (i, child) in item.callers.iter().enumerate() {
        let last = i + 1 == item.callers.len();
        print_tree(child, &child_prefix, last);
    }
}
