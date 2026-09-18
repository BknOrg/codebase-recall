use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

use crate::cli::PathArgs;
use crate::commands::graph::build_graph;
use crate::commands::impact::get_snippet;
use crate::graph::query::{Adjacency, build_adjacency, find_targets};
use crate::graph::{Edge, Node};

const MAX_PAIRS: usize = 5;

const RULE_HEAVY: &str = "══════════════════════════════════════════════════════════════════";
const RULE_LIGHT: &str = "──────────────────────────────────────────────────────────────────";

#[derive(Debug, Serialize)]
pub struct PathHop {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub path: Option<String>,
    /// How this node was reached from the previous hop; `None` for the start node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_kind: Option<String>,
    /// True when the edge was followed against its direction (`--direction reverse|any`).
    pub reversed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Line of the relation in the edge's source file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PathReport {
    pub from: String,
    pub from_id: String,
    pub to: String,
    pub to_id: String,
    pub direction: String,
    pub found: bool,
    /// Number of edges on the path (0 when not found).
    pub length: usize,
    pub hops: Vec<PathHop>,
}

#[derive(Clone, Copy, PartialEq)]
enum Direction {
    Forward,
    Reverse,
    Any,
}

fn parse_direction(raw: &str) -> Result<Direction> {
    match raw.to_ascii_lowercase().as_str() {
        "forward" => Ok(Direction::Forward),
        "reverse" => Ok(Direction::Reverse),
        "any" => Ok(Direction::Any),
        other => anyhow::bail!("unknown --direction '{other}'; expected forward, reverse, or any"),
    }
}

pub fn run(args: PathArgs) -> Result<()> {
    let reports = generate_reports(&args)?;
    if args.json {
        println!("{}", render_json(&reports)?);
    } else {
        print!("{}", render_ascii(&reports));
    }
    Ok(())
}

pub fn render_json(reports: &[PathReport]) -> Result<String> {
    Ok(serde_json::to_string_pretty(reports)?)
}

/// Build the graph, then BFS from every node matching `from` to every node matching `to`.
pub fn generate_reports(args: &PathArgs) -> Result<Vec<PathReport>> {
    let direction = parse_direction(&args.direction)?;

    let query = crate::service::analysis_query(
        &args.project,
        args.kinds.clone(),
        args.no_sync,
        args.precise.clone(),
    );

    let graph = build_graph(&query).context("failed to build code graph for path analysis")?;

    let froms = find_targets(&graph, &args.from);
    if froms.is_empty() {
        anyhow::bail!(
            "No symbol or file matching '{}' found in graph. Run `code-rcl graph` to inspect available symbols.",
            args.from
        );
    }
    let tos = find_targets(&graph, &args.to);
    if tos.is_empty() {
        anyhow::bail!(
            "No symbol or file matching '{}' found in graph. Run `code-rcl graph` to inspect available symbols.",
            args.to
        );
    }

    let mut pairs: Vec<(&Node, &Node)> = Vec::new();
    'outer: for f in &froms {
        for t in &tos {
            if f.id == t.id {
                continue;
            }
            pairs.push((f, t));
            if pairs.len() >= MAX_PAIRS {
                break 'outer;
            }
        }
    }
    if pairs.is_empty() {
        anyhow::bail!("'{}' and '{}' resolve to the same node", args.from, args.to);
    }

    let adjacency = build_adjacency(&graph, &args.kinds);
    let node_by_id: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut file_lines_cache: HashMap<String, Vec<String>> = HashMap::new();

    let mut reports = Vec::with_capacity(pairs.len());
    for (from, to) in pairs {
        let steps = shortest_path(&adjacency, from, to, direction, args.max_depth);
        let mut report = PathReport {
            from: from.label.clone(),
            from_id: from.id.clone(),
            to: to.label.clone(),
            to_id: to.id.clone(),
            direction: args.direction.clone(),
            found: steps.is_some(),
            length: 0,
            hops: Vec::new(),
        };

        if let Some(steps) = steps {
            report.length = steps.len();
            report.hops.push(hop_for_start(from));
            for step in steps {
                let Some(node) = node_by_id.get(step.node_id.as_str()) else {
                    continue;
                };
                let snippet_path = node_by_id
                    .get(step.edge.source.as_str())
                    .and_then(|n| n.path.as_deref());
                let snippet =
                    get_snippet(&args.project, &mut file_lines_cache, snippet_path, step.edge.line);
                report.hops.push(PathHop {
                    id: node.id.clone(),
                    label: node.label.clone(),
                    kind: node.kind.clone(),
                    path: node.path.clone(),
                    edge_kind: Some(step.edge.kind.clone()),
                    reversed: step.reversed,
                    confidence: Some(step.edge.confidence),
                    line: step.edge.line,
                    snippet,
                });
            }
        }
        reports.push(report);
    }

    Ok(reports)
}

fn hop_for_start(node: &Node) -> PathHop {
    PathHop {
        id: node.id.clone(),
        label: node.label.clone(),
        kind: node.kind.clone(),
        path: node.path.clone(),
        edge_kind: None,
        reversed: false,
        confidence: None,
        line: None,
        snippet: None,
    }
}

struct Step<'a> {
    node_id: String,
    edge: &'a Edge,
    reversed: bool,
}

/// Breadth-first search from `from` to `to`, at most `max_depth` edges long.
/// Returns the steps after the start node, or `None` if no path exists.
fn shortest_path<'a>(
    adjacency: &Adjacency<'a>,
    from: &Node,
    to: &Node,
    direction: Direction,
    max_depth: u32,
) -> Option<Vec<Step<'a>>> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut parent: HashMap<&'a str, (&'a str, &'a Edge, bool)> = HashMap::new();
    let mut queue: VecDeque<(&str, u32)> = VecDeque::new();

    let start: &str = from.id.as_str();
    visited.insert(start);
    queue.push_back((start, 0));

    while let Some((current, depth)) = queue.pop_front() {
        if current == to.id {
            return Some(rebuild(parent, current, start));
        }
        if depth >= max_depth {
            continue;
        }

        // (next node, node we came from, edge, followed against its direction)
        let mut neighbors: Vec<(&'a str, &'a str, &'a Edge, bool)> = Vec::new();
        if matches!(direction, Direction::Forward | Direction::Any) {
            if let Some(edges) = adjacency.outgoing.get(current) {
                neighbors.extend(
                    edges
                        .iter()
                        .map(|e| (e.target.as_str(), e.source.as_str(), *e, false)),
                );
            }
        }
        if matches!(direction, Direction::Reverse | Direction::Any) {
            if let Some(edges) = adjacency.incoming.get(current) {
                neighbors.extend(
                    edges
                        .iter()
                        .map(|e| (e.source.as_str(), e.target.as_str(), *e, true)),
                );
            }
        }

        for (next, prev, edge, reversed) in neighbors {
            if visited.insert(next) {
                parent.insert(next, (prev, edge, reversed));
                queue.push_back((next, depth + 1));
            }
        }
    }
    None
}

fn rebuild<'a>(
    parent: HashMap<&'a str, (&'a str, &'a Edge, bool)>,
    end: &str,
    start: &str,
) -> Vec<Step<'a>> {
    let mut steps = Vec::new();
    let mut cursor = end;
    while cursor != start {
        let Some((prev, edge, reversed)) = parent.get(cursor) else {
            break;
        };
        steps.push(Step {
            node_id: cursor.to_string(),
            edge,
            reversed: *reversed,
        });
        cursor = prev;
    }
    steps.reverse();
    steps
}

pub fn render_ascii(reports: &[PathReport]) -> String {
    let mut out = String::new();
    for report in reports {
        write_report(&mut out, report);
    }
    out
}

fn write_report(out: &mut String, report: &PathReport) {
    let _ = writeln!(out, "{RULE_HEAVY}");
    let _ = writeln!(out, "  PATH: {} → {}", report.from, report.to);
    let _ = writeln!(out, "{RULE_LIGHT}");
    if report.found {
        let _ = writeln!(
            out,
            "  Length : {} hop(s)   Direction : {}",
            report.length, report.direction
        );
    } else {
        let _ = writeln!(out, "  Direction : {}", report.direction);
    }
    let _ = writeln!(out, "{RULE_HEAVY}\n");

    if !report.found {
        let _ = writeln!(
            out,
            "  No path found from '{}' to '{}'.",
            report.from, report.to
        );
        if report.direction.eq_ignore_ascii_case("forward") {
            let _ = writeln!(
                out,
                "  Try --direction any to follow edges in either direction, or raise --max-depth.\n"
            );
        } else {
            let _ = writeln!(out, "  Try raising --max-depth.\n");
        }
        return;
    }

    let last = report.hops.len().saturating_sub(1);
    for (i, hop) in report.hops.iter().enumerate() {
        let glyph = if i == 0 || i == last { "◆" } else { "○" };
        let loc = hop
            .path
            .as_deref()
            .map(|p| format!("  {p}"))
            .unwrap_or_default();
        let _ = writeln!(out, "  {glyph} {}  [{}]{loc}", hop.label, hop.kind);

        if i < last {
            let next = &report.hops[i + 1];
            let verb = match (&next.edge_kind, next.reversed) {
                (Some(k), false) => k.clone(),
                (Some(k), true) => format!("{k} (reversed)"),
                (None, _) => String::new(),
            };
            let at = match (&hop.path, &next.line) {
                (_, Some(l)) => format!("  line {l}"),
                _ => String::new(),
            };
            let conf = next
                .confidence
                .map(|c| format!("  (conf {c:.2})"))
                .unwrap_or_default();
            let _ = writeln!(out, "  │  {verb}{at}{conf}");
            if let Some(snippet) = &next.snippet {
                let _ = writeln!(out, "  │  │ {snippet}");
            }
            let _ = writeln!(out, "  ▼");
        }
    }
    let _ = writeln!(out);
}
