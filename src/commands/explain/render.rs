use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Write as _;

use crate::commands::explain::models::ExplainReport;
use crate::commands::impact::write_tree;

const RULE_HEAVY: &str = "══════════════════════════════════════════════════════════════════";
const RULE_LIGHT: &str = "──────────────────────────────────────────────────────────────────";

pub fn render_json(reports: &[ExplainReport]) -> Result<String> {
    Ok(serde_json::to_string_pretty(reports)?)
}

pub fn render_ascii(reports: &[ExplainReport]) -> String {
    let mut out = String::new();
    for report in reports {
        write_report(&mut out, report);
    }
    out
}

fn write_report(out: &mut String, r: &ExplainReport) {
    let _ = writeln!(out, "{RULE_HEAVY}");
    let _ = writeln!(out, "  EXPLAIN: {}  [{}]", r.symbol, r.kind);
    if let Some(path) = r.path.as_deref() {
        match (r.start_line, r.end_line) {
            (Some(s), Some(e)) if s != e => {
                let _ = writeln!(out, "  {path}:{s}-{e}");
            }
            (Some(s), _) => {
                let _ = writeln!(out, "  {path}:{s}");
            }
            _ => {
                let _ = writeln!(out, "  {path}");
            }
        }
    }
    let _ = writeln!(out, "{RULE_LIGHT}");
    let visibility = if r.is_exported { "public" } else { "private" };
    let _ = writeln!(out, "  Visibility     : {visibility}");
    if let Some(label) = r.community.as_deref() {
        let _ = writeln!(out, "  Community      : {label}");
    }
    if let Some(lang) = r.language.as_deref() {
        let _ = writeln!(out, "  Language       : {lang}");
    }
    if let Some(sig) = r.signature.as_deref() {
        let _ = writeln!(out, "  Signature      : {sig}");
    }
    let _ = writeln!(
        out,
        "  Direct callers : {}      Direct callees : {}",
        r.relations.direct_callers_count, r.relations.direct_callees_count
    );
    if r.relations.risky_affected_count > 0 {
        let _ = writeln!(
            out,
            "  Risk flags     : {} item(s) touch public API, low-confidence edges, or cross module boundaries",
            r.relations.risky_affected_count
        );
    }
    let _ = writeln!(out, "{RULE_HEAVY}\n");

    if let Some(doc) = r.doc.as_deref() {
        let _ = writeln!(out, "DOCS");
        for line in doc.lines() {
            let _ = writeln!(out, "  {line}");
        }
        let _ = writeln!(out);
    }

    if !r.members.is_empty() {
        let _ = writeln!(out, "MEMBERS ({})", r.members.len());
        for m in &r.members {
            let vis = if m.is_exported { "pub " } else { "    " };
            let line = m.line.map(|l| format!("  line {l}")).unwrap_or_default();
            let _ = writeln!(out, "  {vis}{}  [{}]{line}", m.name, m.kind);
        }
        let _ = writeln!(out);
    }

    // A type only ever named in a signature or a field has references but no
    // calls, so a "CALLED BY" header would misdescribe its whole listing. Each
    // row still states its own edge kind.
    write_section(
        out,
        &section_title("▲", &r.relations.callers, "CALLED BY", "USED BY"),
        "(no incoming callers or references found)",
        &r.relations.callers,
    );
    write_section(
        out,
        &section_title("▼", &r.relations.callees, "CALLS", "USES"),
        "(no outgoing calls or references found)",
        &r.relations.callees,
    );
}

/// `calls_title` while every edge really is a call, `mixed_title` as soon as a
/// non-call relation (a type reference, an import) shows up.
fn section_title(
    arrow: &str,
    items: &[crate::commands::impact::ImpactItem],
    calls_title: &str,
    mixed_title: &str,
) -> String {
    let all_calls = items.iter().all(|i| i.edge_kind == "calls");
    let word = if all_calls { calls_title } else { mixed_title };
    format!("{arrow} {word}")
}

fn write_section(
    out: &mut String,
    title: &str,
    empty: &str,
    items: &[crate::commands::impact::ImpactItem],
) {
    let _ = writeln!(out, "{title}");
    if items.is_empty() {
        let _ = writeln!(out, "  {empty}");
    } else {
        let mut visited = HashSet::new();
        for (i, item) in items.iter().enumerate() {
            write_tree(out, item, "  ", i + 1 == items.len(), &mut visited);
        }
    }
    let _ = writeln!(out);
}
