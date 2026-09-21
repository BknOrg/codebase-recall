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
    /// Where the start node is declared. Several symbols can share a name, so
    /// the rendered header needs this to say *which* one it walked from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_loc: Option<String>,
    pub to: String,
    pub to_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_loc: Option<String>,
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

    // A name is rarely unique, and reporting on an unrelated same-named symbol
    // reads as a wrong answer. Pair the candidates nearest each other first, so
    // the truncated list keeps the pairs the caller most likely meant.
    let mut pairs: Vec<(&Node, &Node)> = Vec::new();
    for f in &froms {
        for t in &tos {
            if f.id != t.id {
                pairs.push((f, t));
            }
        }
    }
    if pairs.is_empty() {
        anyhow::bail!("'{}' and '{}' resolve to the same node", args.from, args.to);
    }
    pairs.sort_by_key(|(f, t)| std::cmp::Reverse(proximity(f, t)));
    pairs.truncate(MAX_PAIRS);

    let adjacency = build_adjacency(&graph, &args.kinds);
    let node_by_id: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut file_lines_cache: HashMap<String, Vec<String>> = HashMap::new();

    let mut reports = Vec::with_capacity(pairs.len());
    for (from, to) in pairs {
        let steps = shortest_path(&adjacency, from, to, direction, args.max_depth);
        let mut report = PathReport {
            from: from.label.clone(),
            from_id: from.id.clone(),
            from_loc: location_of(from),
            to: to.label.clone(),
            to_id: to.id.clone(),
            to_loc: location_of(to),
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

/// `path/to/file.rs:12` for a symbol node, the path alone for a file node.
fn location_of(node: &Node) -> Option<String> {
    let path = node.path.as_deref()?;
    Some(match node.lines {
        Some([start, _]) if start > 0 => format!("{path}:{start}"),
        _ => path.to_string(),
    })
}

/// How closely two candidates sit together: same file beats same directory,
/// which beats anywhere else.
fn proximity(from: &Node, to: &Node) -> u8 {
    let (Some(a), Some(b)) = (from.path.as_deref(), to.path.as_deref()) else {
        return 0;
    };
    if a == b {
        2
    } else if parent_dir(a) == parent_dir(b) {
        1
    } else {
        0
    }
}

fn parent_dir(path: &str) -> &str {
    path.rfind('/').map(|i| &path[..i]).unwrap_or("")
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
    // With one report the header name is unambiguous; with several, each is
    // about a different pair of same-named symbols and has to say which.
    let label_endpoints = reports.len() > 1;
    for (i, report) in reports.iter().enumerate() {
        if label_endpoints {
            let _ = writeln!(out, "  [candidate {} of {}]", i + 1, reports.len());
        }
        write_report(&mut out, report, label_endpoints);
    }
    out
}

fn write_report(out: &mut String, report: &PathReport, label_endpoints: bool) {
    let _ = writeln!(out, "{RULE_HEAVY}");
    let _ = writeln!(out, "  PATH: {} → {}", report.from, report.to);
    if label_endpoints {
        let _ = writeln!(out, "{RULE_LIGHT}");
        let _ = writeln!(
            out,
            "  From : {}\n  To   : {}",
            report.from_loc.as_deref().unwrap_or("(unknown location)"),
            report.to_loc.as_deref().unwrap_or("(unknown location)"),
        );
    }
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
