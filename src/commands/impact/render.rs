use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Write as _;

use crate::commands::impact::models::{ImpactItem, ImpactReport};

pub fn render_ascii(reports: &[ImpactReport]) -> String {
    let mut out = String::new();
    for report in reports {
        write_ascii_report(&mut out, report);
    }
    out
}

pub fn render_json(reports: &[ImpactReport]) -> Result<String> {
    Ok(serde_json::to_string_pretty(reports)?)
}

const RULE_HEAVY: &str = "══════════════════════════════════════════════════════════════════";
const RULE_LIGHT: &str = "──────────────────────────────────────────────────────────────────";

pub fn write_ascii_report(out: &mut String, report: &ImpactReport) {
    let dir_norm = report.direction.to_ascii_lowercase();
    let is_both = matches!(dir_norm.as_str(), "both" | "");
    let is_rev = matches!(dir_norm.as_str(), "reverse" | "upstream" | "callers");
    let is_fwd = matches!(dir_norm.as_str(), "forward" | "downstream" | "callees");

    let _ = writeln!(out, "{RULE_HEAVY}");
    let _ = writeln!(
        out,
        "  IMPACT ANALYSIS: {}  [{}]",
        report.target_symbol, report.target_kind
    );
    if let Some(path) = report.target_path.as_deref() {
        let _ = writeln!(out, "  {path}");
    }
    let _ = writeln!(out, "{RULE_LIGHT}");
    let _ = writeln!(
        out,
        "  Direct callers : {}      Direct callees : {}",
        report.direct_callers_count, report.direct_callees_count
    );
    let _ = writeln!(
        out,
        "  Total affected : {} symbols across {} file(s)",
        report.total_affected_count, report.total_affected_files
    );
    let _ = writeln!(out, "  Max depth      : {}", report.max_depth_reached);
    if let Some(label) = report.target_community.as_deref() {
        let _ = writeln!(out, "  Community      : {label}");
    }
    if report.risky_affected_count > 0 {
        let _ = writeln!(
            out,
            "  Risk flags     : {} item(s) touch public API, low-confidence edges, or cross module boundaries",
            report.risky_affected_count
        );
    }
    let _ = writeln!(out, "{RULE_HEAVY}\n");

    if is_both || is_rev {
        let _ = writeln!(out, "▲ UPSTREAM CALLERS");
        if report.callers.is_empty() {
            let _ = writeln!(out, "  (no incoming callers or references found)");
        } else {
            let mut visited = HashSet::new();
            for (i, caller) in report.callers.iter().enumerate() {
                let is_last = i + 1 == report.callers.len();
                write_tree(out, caller, "  ", is_last, &mut visited);
            }
        }
        let _ = writeln!(out);
    }

    if is_both || is_fwd {
        let _ = writeln!(out, "▼ DOWNSTREAM CALLEES");
        if report.callees.is_empty() {
            let _ = writeln!(out, "  (no outgoing calls found)");
        } else {
            let mut visited = HashSet::new();
            for (i, callee) in report.callees.iter().enumerate() {
                let is_last = i + 1 == report.callees.len();
                write_tree(out, callee, "  ", is_last, &mut visited);
            }
        }
        let _ = writeln!(out);
    }
}

/// Where an item is defined and, when the relation's line lives in another
/// file (a callee is called from its caller's file), where that line is.
fn item_location(item: &ImpactItem) -> String {
    match (&item.path, item.line, &item.site_path) {
        (Some(p), Some(l), Some(site)) if site != p => format!("  {p}  (called at {site}:{l})"),
        (Some(p), Some(l), _) => format!("  {p}:{l}"),
        (Some(p), None, _) => format!("  {p}"),
        _ => String::new(),
    }
}

pub fn write_tree(
    out: &mut String,
    item: &ImpactItem,
    prefix: &str,
    is_last: bool,
    visited: &mut HashSet<String>,
) {
    let branch = if is_last { "└── " } else { "├── " };
    let loc = item_location(item);

    let conf_str = format!("(conf {:.2})", item.confidence);

    let mut badges = Vec::new();
    if item.is_exported {
        badges.push("⚠ public");
    }
    if item.low_confidence {
        badges.push("⚠ low-conf");
    }
    let cross_module = match item.community.as_deref() {
        Some(label) => format!("⚠ cross-module ({label})"),
        None => "⚠ cross-module".to_string(),
    };
    if item.crosses_module {
        badges.push(cross_module.as_str());
    }
    let badge_str = if badges.is_empty() {
        String::new()
    } else {
        format!("  {}", badges.join(" "))
    };

    let is_leaf = item.callers.is_empty();
    let glyph = if is_leaf { "◆" } else { "○" };
    let child_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });

    if !visited.insert(item.id.clone()) {
        let _ = writeln!(
            out,
            "{prefix}{branch}{glyph} {}  [{} → {}]{loc}  {conf_str}{badge_str}",
            item.label, item.edge_kind, item.kind
        );
        if let Some(snippet) = &item.snippet {
            let _ = writeln!(out, "{child_prefix}│ {snippet}");
        }
        let _ = writeln!(out, "{child_prefix}└── (already traced above)");
        return;
    }

    let _ = writeln!(
        out,
        "{prefix}{branch}{glyph} {}  [{} → {}]{loc}  {conf_str}{badge_str}",
        item.label, item.edge_kind, item.kind
    );

    if let Some(snippet) = &item.snippet {
        let _ = writeln!(out, "{child_prefix}│ {snippet}");
    }

    for (i, child) in item.callers.iter().enumerate() {
        let last = i + 1 == item.callers.len();
        write_tree(out, child, &child_prefix, last, visited);
    }
}

#[cfg(test)]
mod location_tests {
    use super::*;

    fn item(path: &str, site: Option<&str>, line: Option<i64>) -> ImpactItem {
        ImpactItem {
            id: "x".to_string(),
            label: "x".to_string(),
            kind: "function".to_string(),
            path: Some(path.to_string()),
            line,
            site_path: site.map(str::to_string),
            edge_kind: "calls".to_string(),
            confidence: 0.9,
            snippet: None,
            depth: 1,
            is_exported: false,
            low_confidence: false,
            crosses_module: false,
            community: None,
            callers: Vec::new(),
        }
    }

    #[test]
    fn caller_shows_its_own_file_and_line() {
        let it = item("src/a.rs", Some("src/a.rs"), Some(7));
        assert_eq!(item_location(&it), "  src/a.rs:7");
    }

    #[test]
    fn callee_shows_definition_and_the_call_site_separately() {
        let it = item("src/service.rs", Some("src/impact/mod.rs"), Some(98));
        assert_eq!(
            item_location(&it),
            "  src/service.rs  (called at src/impact/mod.rs:98)"
        );
    }
}
