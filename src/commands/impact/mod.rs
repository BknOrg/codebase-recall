pub mod diffscan;
pub mod models;
pub mod render;
pub mod traversal;

pub use diffscan::*;
pub use models::*;
pub use render::*;
pub use traversal::*;

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};

use crate::cache::CacheDb;
use crate::cli::ImpactArgs;
use crate::commands::graph::build_graph;
use crate::graph::query::{Adjacency, build_adjacency, find_targets};
use crate::graph::{Node, symbol_id};

/// Most targets a `--diff` run reports; a large working tree can touch hundreds
/// of symbols, far more than anyone (or any MCP client) can read.
pub const DIFF_TARGET_LIMIT: usize = 10;

/// What `trim_diff_reports` left out.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DiffTrim {
    /// Targets with no callers or callees: changing them reaches nothing.
    pub isolated: usize,
    /// Connected targets beyond the limit, least affected first to go.
    pub over_limit: usize,
}

impl DiffTrim {
    /// A one-line note for the reader, or `None` when nothing was dropped.
    pub fn note(&self) -> Option<String> {
        if self.isolated == 0 && self.over_limit == 0 {
            return None;
        }
        Some(format!(
            "note: showing the {} most-affected changed symbols; omitted {} with no callers or callees and {} more beyond the limit (run `impact <SYMBOL>` to inspect any of them).",
            DIFF_TARGET_LIMIT, self.isolated, self.over_limit
        ))
    }
}

/// Shrink a `--diff` result to what is worth reading: drop targets that reach
/// nothing, put the widest blast radius first, and keep at most `limit`.
pub fn trim_diff_reports(reports: &mut Vec<ImpactReport>, limit: usize) -> DiffTrim {
    let before = reports.len();
    reports.retain(|r| r.total_affected_count > 0);
    let isolated = before - reports.len();

    reports.sort_by(|a, b| {
        b.total_affected_count
            .cmp(&a.total_affected_count)
            .then_with(|| a.target_symbol.cmp(&b.target_symbol))
    });
    let over_limit = reports.len().saturating_sub(limit);
    reports.truncate(limit);
    DiffTrim { isolated, over_limit }
}

pub fn run(args: ImpactArgs) -> Result<()> {
    let mut reports = generate_reports(&args)?;
    let trim = if args.diff {
        trim_diff_reports(&mut reports, DIFF_TARGET_LIMIT)
    } else {
        DiffTrim::default()
    };
    if args.json {
        println!("{}", render_json(&reports)?);
        // Keep stdout a valid JSON array; the note goes to the side channel.
        if let Some(note) = trim.note() {
            eprintln!("{note}");
        }
    } else {
        print!("{}", render_ascii(&reports));
        if let Some(note) = trim.note() {
            println!("
{note}");
        }
    }
    Ok(())
}

/// Build graph + traverse callers/callees for every node matching `args.symbol`
/// (or, with `--diff`, every symbol touched by uncommitted git changes).
pub fn generate_reports(args: &ImpactArgs) -> Result<Vec<ImpactReport>> {
    if !args.diff && args.symbol.is_none() {
        anyhow::bail!(
            "impact requires either a <SYMBOL> argument or --diff to auto-detect changed symbols from git"
        );
    }
    if args.diff && args.symbol.is_some() {
        anyhow::bail!("--diff cannot be combined with an explicit <SYMBOL>; pass one or the other");
    }

    let query = crate::service::analysis_query(
        &args.project,
        args.kinds.clone(),
        args.no_sync,
        args.precise.clone(),
    );


    let graph = build_graph(&query).context("failed to build code graph for impact analysis")?;
    let node_by_id: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let targets: Vec<&Node> = if args.diff {
        let db = CacheDb::open(&args.project)?;
        let ranges = scan_diff(&args.project)?;
        if ranges.is_empty() {
            if args.json {
                return Ok(Vec::new());
            }
            println!("No uncommitted changes detected.");
            return Ok(Vec::new());
        }

        let symbols = resolve_changed_symbols(&db, &ranges)?;
        if symbols.is_empty() {
            if args.json {
                return Ok(Vec::new());
            }
            println!(
                "No symbols affected by the current diff (only whitespace/deletions, or changed files are outside the synced project)."
            );
            return Ok(Vec::new());
        }

        let files = db.all_files()?;
        let file_path_by_id: HashMap<i64, String> =
            files.into_iter().map(|f| (f.id, f.path)).collect();

        let target_ids: HashSet<String> = symbols
            .iter()
            .filter_map(|s| {
                let path = file_path_by_id.get(&s.file_id)?;
                Some(symbol_id(path, &s.name, s.start_line.unwrap_or(0)))
            })
            .collect();

        graph.nodes.iter().filter(|n| target_ids.contains(&n.id)).collect()
    } else {
        let symbol = args.symbol.as_deref().expect("validated non-empty above");
        find_targets(&graph, symbol)
    };

    if targets.is_empty() {
        if args.json {
            return Ok(Vec::new());
        }
        anyhow::bail!(
            "No symbol or file matching '{}' found in graph. Run `code-rcl graph` to inspect available symbols.",
            args.symbol.as_deref().unwrap_or("")
        );
    }

    let dir_norm = args.direction.to_ascii_lowercase();
    let do_upstream = matches!(dir_norm.as_str(), "both" | "reverse" | "upstream" | "callers");
    let do_downstream = matches!(dir_norm.as_str(), "both" | "forward" | "downstream" | "callees");

    let Adjacency { incoming, outgoing } = build_adjacency(&graph, &args.kinds);

    let mut file_lines_cache: HashMap<String, Vec<String>> = HashMap::new();
    let community_labels: HashMap<u32, String> = graph
        .communities
        .iter()
        .map(|c| (c.id, c.label.clone()))
        .collect();

    let mut reports = Vec::with_capacity(targets.len());
    for target in targets {
        let ctx = RiskContext {
            module: target.path.as_deref().map(module_root).unwrap_or(""),
            community: target.community,
            labels: &community_labels,
        };

        let mut ancestors_up = HashSet::new();
        let mut total_affected_up = HashSet::new();
        ancestors_up.insert(target.id.clone());

        let callers = if do_upstream {
            traverse_tree(
                &target.id,
                0,
                args.depth,
                &incoming,
                &node_by_id,
                &mut ancestors_up,
                &mut total_affected_up,
                false,
                &args.project,
                &mut file_lines_cache,
                &ctx,
            )
        } else {
            Vec::new()
        };

        let mut ancestors_down = HashSet::new();
        let mut total_affected_down = HashSet::new();
        ancestors_down.insert(target.id.clone());

        let callees = if do_downstream {
            traverse_tree(
                &target.id,
                0,
                args.depth,
                &outgoing,
                &node_by_id,
                &mut ancestors_down,
                &mut total_affected_down,
                true,
                &args.project,
                &mut file_lines_cache,
                &ctx,
            )
        } else {
            Vec::new()
        };

        let direct_callers_count = callers.len();
        let total_callers_count = total_affected_up.len();

        let direct_callees_count = callees.len();
        let total_callees_count = total_affected_down.len();

        let mut all_affected = total_affected_up;
        all_affected.extend(total_affected_down);

        let affected_files: HashSet<&str> = all_affected
            .iter()
            .filter_map(|id| node_by_id.get(id.as_str()).and_then(|n| n.path.as_deref()))
            .collect();
        let total_affected_files = affected_files.len();

        let mut max_depth_reached = 0;
        calculate_max_depth(&callers, &mut max_depth_reached);
        calculate_max_depth(&callees, &mut max_depth_reached);

        let mut risky_seen = HashSet::new();
        let mut risky_affected_count = 0;
        count_risky(&callers, &mut risky_seen, &mut risky_affected_count);
        count_risky(&callees, &mut risky_seen, &mut risky_affected_count);

        let report = ImpactReport {
            target_symbol: target.label.clone(),
            target_id: target.id.clone(),
            target_kind: target.kind.clone(),
            target_path: target.path.clone(),
            direction: args.direction.clone(),
            direct_callers_count,
            total_callers_count,
            callers,
            direct_callees_count,
            total_callees_count,
            callees,
            total_affected_count: all_affected.len(),
            total_affected_files,
            risky_affected_count,
            target_community: target.community.and_then(|c| community_labels.get(&c).cloned()),
            max_depth_reached,
        };

        reports.push(report);
    }

    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(name: &str, affected: usize) -> ImpactReport {
        ImpactReport {
            target_symbol: name.to_string(),
            target_id: name.to_string(),
            target_kind: "function".to_string(),
            target_path: None,
            direction: "both".to_string(),
            direct_callers_count: 0,
            total_callers_count: 0,
            callers: Vec::new(),
            direct_callees_count: 0,
            total_callees_count: 0,
            callees: Vec::new(),
            total_affected_count: affected,
            total_affected_files: 0,
            risky_affected_count: 0,
            target_community: None,
            max_depth_reached: 0,
        }
    }

    #[test]
    fn diff_trim_drops_isolated_sorts_and_caps() {
        let mut reports = vec![report("lone", 0), report("small", 1), report("big", 9), report("mid", 4)];
        let trim = trim_diff_reports(&mut reports, 2);
        let names: Vec<_> = reports.iter().map(|r| r.target_symbol.as_str()).collect();
        assert_eq!(names, ["big", "mid"]);
        assert_eq!(trim, DiffTrim { isolated: 1, over_limit: 1 });
        assert!(trim.note().is_some());
    }

    #[test]
    fn diff_trim_is_silent_when_nothing_is_dropped() {
        let mut reports = vec![report("a", 3)];
        let trim = trim_diff_reports(&mut reports, 20);
        assert_eq!(trim, DiffTrim::default());
        assert!(trim.note().is_none());
    }
}
