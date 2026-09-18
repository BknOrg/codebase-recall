//! `code-rcl report`: a one-page architecture overview built from the relation graph —
//! core hubs, subsystems (communities), bridges between them, and questions worth asking.
//! `sync` keeps `.code-rcl/REPORT.md` fresh so an agent can read it without a tool call.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cache::ctx_dir;
use crate::cli::{PreciseArgs, ReportArgs};
use crate::commands::digest::{HubItem, ensure_extension, hubs_from_graph};
use crate::commands::graph::build_graph;
use crate::commands::sync::SyncStats;
use crate::graph::CodeGraph;
use crate::graph::community::{self, Communities, file_graph};

const REPORT_FILE: &str = "REPORT.md";
const MAX_SUBSYSTEMS: usize = 12;
const MAX_BRIDGES: usize = 5;

#[derive(Debug, Serialize)]
pub struct LinkInfo {
    pub subsystem: String,
    pub links: usize,
}

#[derive(Debug, Serialize)]
pub struct Subsystem {
    pub id: u32,
    pub label: String,
    pub files: usize,
    pub key_files: Vec<String>,
    /// Share of this subsystem's links that stay inside it (0.0 - 1.0).
    pub cohesion: f64,
    pub internal_links: usize,
    pub external_links: usize,
    pub depends_on: Vec<LinkInfo>,
}

#[derive(Debug, Serialize)]
pub struct Bridge {
    pub file: String,
    pub touches: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Question {
    pub question: String,
    pub command: String,
}

#[derive(Debug, Serialize)]
pub struct ReportData {
    pub project: String,
    pub total_files: usize,
    pub total_symbols: usize,
    pub public_symbols: usize,
    pub languages: BTreeMap<String, usize>,
    pub hubs: Vec<HubItem>,
    pub subsystems: Vec<Subsystem>,
    pub unclustered_files: usize,
    pub bridges: Vec<Bridge>,
    pub questions: Vec<Question>,
}

pub fn run(args: ReportArgs) -> Result<()> {
    let data = generate(&args.project, args.no_sync)?;
    let content = if args.json {
        serde_json::to_string_pretty(&data)?
    } else {
        render_markdown(&data)
    };

    let mut wrote = false;
    if args.write {
        let path = report_path(&args.project);
        write_file(&path, &content)?;
        eprintln!("Report written to {}", path.display());
        wrote = true;
    }
    if let Some(out) = &args.output {
        let path = ensure_extension(out, args.json);
        write_file(&path, &content)?;
        eprintln!("Report written to {}", path.display());
        wrote = true;
    }
    if !wrote {
        print!("{content}");
    }
    Ok(())
}

pub fn report_path(project: &Path) -> PathBuf {
    ctx_dir(project).join(REPORT_FILE)
}

fn write_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))
}

/// Regenerate `.code-rcl/REPORT.md` when a sync changed something (or the file is missing, or
/// language-server edges were added). Returns the path if it was written, `None` if it was
/// already current. Never fails the caller: problems come back as `Err` for them to downgrade.
pub fn refresh_after_sync(
    project: &Path,
    stats: &SyncStats,
    precise_ran: bool,
) -> Result<Option<PathBuf>> {
    let path = report_path(project);
    let changed = stats.added + stats.changed + stats.removed > 0;
    if !changed && !precise_ran && path.exists() {
        return Ok(None);
    }

    let content = render_markdown(&generate(project, true)?);
    if fs::read_to_string(&path).is_ok_and(|existing| existing == content) {
        return Ok(None);
    }
    write_file(&path, &content)?;
    Ok(Some(path))
}

pub fn generate(project: &Path, no_sync: bool) -> Result<ReportData> {
    let query = crate::service::analysis_query(
        project,
        crate::service::call_and_import_kinds(),
        no_sync,
        PreciseArgs::default(),
    );

    let graph = build_graph(&query).context("failed to build code graph for the report")?;
    Ok(analyze(project, &graph))
}

fn project_name(project: &Path) -> String {
    project
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "codebase".to_string())
}

fn path_of(file_node_id: &str) -> &str {
    file_node_id.strip_prefix("file:").unwrap_or(file_node_id)
}

fn analyze(project: &Path, graph: &CodeGraph) -> ReportData {
    let mut languages: BTreeMap<String, usize> = BTreeMap::new();
    let (mut total_files, mut total_symbols, mut public_symbols) = (0usize, 0usize, 0usize);
    for n in &graph.nodes {
        match n.kind.as_str() {
            "file" => {
                total_files += 1;
                if let Some(lang) = &n.language {
                    *languages.entry(lang.clone()).or_insert(0) += 1;
                }
            }
            // Same symbol kinds `digest` counts, so the two summaries agree.
            "struct" | "enum" | "class" | "interface" | "trait" | "type" | "method"
            | "function" => {
                total_symbols += 1;
                if n.exported {
                    public_symbols += 1;
                }
            }
            _ => {}
        }
    }

    let hubs = hubs_from_graph(graph);
    let communities = community::detect_graph(graph);
    let links = file_graph(&graph.nodes, &graph.edges);

    let label_of: HashMap<u32, &str> = communities
        .list
        .iter()
        .map(|c| (c.id, c.label.as_str()))
        .collect();

    let subsystems: Vec<Subsystem> = communities
        .list
        .iter()
        .take(MAX_SUBSYSTEMS)
        .map(|c| subsystem_of(c, &communities, &label_of))
        .collect();

    let clustered: usize = communities.list.iter().map(|c| c.size).sum();
    let unclustered_files = total_files.saturating_sub(clustered);

    let bridges = bridges_of(&communities, &links, &label_of);
    let questions = questions_of(&hubs, &subsystems, &communities, &bridges);

    ReportData {
        project: project_name(project),
        total_files,
        total_symbols,
        public_symbols,
        languages,
        hubs,
        subsystems,
        unclustered_files,
        bridges,
        questions,
    }
}

fn subsystem_of(
    c: &community::Community,
    all: &Communities,
    label_of: &HashMap<u32, &str>,
) -> Subsystem {
    let total = c.internal_edges + c.external_edges;
    let cohesion = if total == 0 {
        1.0
    } else {
        ((c.internal_edges as f64 / total as f64) * 100.0).round() / 100.0
    };

    let mut deps: Vec<(u32, usize)> = all
        .links
        .iter()
        .filter_map(|(&(a, b), &w)| {
            if a == c.id {
                Some((b, w))
            } else if b == c.id {
                Some((a, w))
            } else {
                None
            }
        })
        .collect();
    deps.sort_by(|x, y| y.1.cmp(&x.1).then_with(|| x.0.cmp(&y.0)));

    Subsystem {
        id: c.id,
        label: c.label.clone(),
        files: c.size,
        key_files: c.top_files.iter().map(|f| path_of(f).to_string()).collect(),
        cohesion,
        internal_links: c.internal_edges,
        external_links: c.external_edges,
        depends_on: deps
            .into_iter()
            .take(3)
            .filter_map(|(id, links)| {
                label_of
                    .get(&id)
                    .map(|l| LinkInfo { subsystem: (*l).to_string(), links })
            })
            .collect(),
    }
}

/// Files with links into other subsystems, most far-reaching first: the seams between
/// subsystems. Ranked by how many other subsystems they touch, then by link count.
fn bridges_of(
    communities: &Communities,
    links: &BTreeMap<(String, String), usize>,
    label_of: &HashMap<u32, &str>,
) -> Vec<Bridge> {
    // file -> (other subsystem -> links into it)
    let mut touches: BTreeMap<&str, BTreeMap<u32, usize>> = BTreeMap::new();
    for ((a, b), &w) in links {
        let (ca, cb) = (communities.assignment.get(a), communities.assignment.get(b));
        if ca == cb {
            continue;
        }
        if let Some(&c) = cb {
            *touches.entry(a.as_str()).or_default().entry(c).or_insert(0) += w;
        }
        if let Some(&c) = ca {
            *touches.entry(b.as_str()).or_default().entry(c).or_insert(0) += w;
        }
    }

    let mut found: Vec<(&str, BTreeMap<u32, usize>)> = touches.into_iter().collect();
    found.sort_by(|a, b| {
        let (wa, wb): (usize, usize) = (a.1.values().sum(), b.1.values().sum());
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| wb.cmp(&wa))
            .then_with(|| a.0.cmp(b.0))
    });

    found
        .into_iter()
        .take(MAX_BRIDGES)
        .map(|(file, reach)| Bridge {
            file: path_of(file).to_string(),
            touches: reach
                .keys()
                .filter_map(|id| label_of.get(id).map(|l| (*l).to_string()))
                .collect(),
        })
        .collect()
}

fn questions_of(
    hubs: &[HubItem],
    subsystems: &[Subsystem],
    communities: &Communities,
    bridges: &[Bridge],
) -> Vec<Question> {
    let mut out = Vec::new();

    if let Some(hub) = hubs.first() {
        out.push(Question {
            question: format!("What breaks if `{}` changes?", hub.label),
            command: format!("code-rcl impact {}", hub.label),
        });
    }
    if let [a, b, ..] = hubs {
        out.push(Question {
            question: format!("How do `{}` and `{}` connect?", a.label, b.label),
            command: format!("code-rcl path {} {}", a.label, b.label),
        });
    }
    if let Some(bridge) = bridges.first() {
        out.push(Question {
            question: format!(
                "Why does `{}` reach into {} other subsystem(s)?",
                bridge.file,
                bridge.touches.len()
            ),
            command: format!("code-rcl explain {}", bridge.file),
        });
    }
    if let Some((&(a, b), _)) = communities.links.iter().max_by_key(|(k, w)| (**w, std::cmp::Reverse(**k))) {
        let key = |id: u32| {
            subsystems
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.key_files.first().cloned())
        };
        let label = |id: u32| {
            communities
                .list
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.label.clone())
        };
        if let (Some(ka), Some(kb), Some(la), Some(lb)) = (key(a), key(b), label(a), label(b)) {
            out.push(Question {
                question: format!("How is `{la}` wired to `{lb}` (the most connected pair)?"),
                command: format!("code-rcl path {ka} {kb} --direction any"),
            });
        }
    }
    out.push(Question {
        question: "What do my uncommitted changes affect?".to_string(),
        command: "code-rcl impact --diff".to_string(),
    });
    out
}

pub fn render_markdown(d: &ReportData) -> String {
    let mut md = String::new();
    md.push_str(&format!("# Code Report: {}\n\n", d.project));
    md.push_str(
        "_Generated by code-rcl. Refresh with `code-rcl report --write`; `code-rcl sync` keeps it current._\n\n",
    );

    let langs = if d.languages.is_empty() {
        "none".to_string()
    } else {
        let mut v: Vec<_> = d.languages.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        v.iter()
            .map(|(l, n)| format!("{l} ({n})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    md.push_str(&format!(
        "**Summary**: {} files | {} symbols ({} public) | Languages: {}\n\n",
        d.total_files, d.total_symbols, d.public_symbols, langs
    ));

    if !d.hubs.is_empty() {
        md.push_str("## Core hubs\n\nMost connected nodes; changes here reach the furthest.\n\n");
        md.push_str("| Symbol | Kind | File | Degree |\n| :--- | :--- | :--- | :--- |\n");
        for h in &d.hubs {
            md.push_str(&format!(
                "| `{}` | {} | `{}` | {} |\n",
                h.label,
                h.kind,
                h.path.as_deref().unwrap_or("-"),
                h.degree
            ));
        }
        md.push('\n');
    }

    md.push_str("## Subsystems\n\n");
    if d.subsystems.is_empty() {
        md.push_str("_No subsystems detected: files have no cross-file imports or calls yet._\n\n");
    } else {
        md.push_str(
            "Groups of files that depend on each other far more than on the rest (detected from imports and calls).\n\n",
        );
        for (i, s) in d.subsystems.iter().enumerate() {
            md.push_str(&format!("### {}. {} ({} files)\n\n", i + 1, s.label, s.files));
            let keys: Vec<String> = s.key_files.iter().map(|f| format!("`{f}`")).collect();
            md.push_str(&format!("- Key files: {}\n", keys.join(", ")));
            md.push_str(&format!(
                "- Cohesion: {:.0}% ({} internal links, {} to other subsystems)\n",
                s.cohesion * 100.0,
                s.internal_links,
                s.external_links
            ));
            if !s.depends_on.is_empty() {
                let deps: Vec<String> = s
                    .depends_on
                    .iter()
                    .map(|l| format!("`{}` ({} links)", l.subsystem, l.links))
                    .collect();
                md.push_str(&format!("- Linked to: {}\n", deps.join(", ")));
            }
            md.push('\n');
        }
    }
    if d.unclustered_files > 0 {
        md.push_str(&format!(
            "_{} file(s) belong to no subsystem (no links to other files)._\n\n",
            d.unclustered_files
        ));
    }

    if !d.bridges.is_empty() {
        md.push_str("## Bridges\n\nFiles with links into other subsystems, most far-reaching first; the seams to be careful with.\n\n");
        md.push_str("| File | Reaches into |\n| :--- | :--- |\n");
        for b in &d.bridges {
            let t: Vec<String> = b.touches.iter().map(|l| format!("`{l}`")).collect();
            md.push_str(&format!("| `{}` | {} |\n", b.file, t.join(", ")));
        }
        md.push('\n');
    }

    md.push_str("## Suggested questions\n\n");
    for q in &d.questions {
        md.push_str(&format!("- {} `{}`\n", q.question, q.command));
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ReportData {
        ReportData {
            project: "demo".into(),
            total_files: 6,
            total_symbols: 9,
            public_symbols: 7,
            languages: BTreeMap::from([("rust".to_string(), 6)]),
            hubs: vec![HubItem {
                label: "apply".into(),
                kind: "function".into(),
                path: Some("tax.rs".into()),
                degree: 4,
            }],
            subsystems: vec![Subsystem {
                id: 0,
                label: "billing".into(),
                files: 3,
                key_files: vec!["invoice.rs".into()],
                cohesion: 0.86,
                internal_links: 6,
                external_links: 1,
                depends_on: vec![LinkInfo { subsystem: "shipping".into(), links: 1 }],
            }],
            unclustered_files: 0,
            bridges: vec![Bridge {
                file: "invoice.rs".into(),
                touches: vec!["billing".into(), "shipping".into()],
            }],
            questions: vec![Question {
                question: "What breaks if `apply` changes?".into(),
                command: "code-rcl impact apply".into(),
            }],
        }
    }

    #[test]
    fn markdown_has_every_section_and_is_stable() {
        let md = render_markdown(&sample());
        for needle in [
            "# Code Report: demo",
            "**Summary**: 6 files | 9 symbols (7 public) | Languages: rust (6)",
            "## Core hubs",
            "| `apply` | function | `tax.rs` | 4 |",
            "### 1. billing (3 files)",
            "Cohesion: 86%",
            "`shipping` (1 links)",
            "## Bridges",
            "## Suggested questions",
            "`code-rcl impact apply`",
        ] {
            assert!(md.contains(needle), "missing {needle:?} in:\n{md}");
        }
        assert_eq!(md, render_markdown(&sample()), "rendering must be deterministic");
    }

    #[test]
    fn empty_report_still_renders() {
        let mut d = sample();
        d.hubs.clear();
        d.subsystems.clear();
        d.bridges.clear();
        d.questions.clear();
        let md = render_markdown(&d);
        assert!(md.contains("No subsystems detected"));
        assert!(!md.contains("## Core hubs"));
    }
}
