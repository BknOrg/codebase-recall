use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::cache::CacheDb;
use crate::cache::models::SymbolRow;

/// Compare both sides without changing the user's checkout or index.
pub fn generate_diff_reports(args: &crate::cli::ImpactArgs) -> Result<Vec<super::ImpactReport>> {
    use crate::graph::symbol_id;
    use std::collections::BTreeMap;
    use std::fs;
    let repo = run_git(&args.project, &["rev-parse", "--show-toplevel"])?;
    let repo = Path::new(repo.trim()).canonicalize()?;
    let project = args.project.canonicalize()?;
    let sub = project.strip_prefix(&repo)?;
    let state_dir = project.join(".code-rcl");
    fs::create_dir_all(&state_dir)?;
    let snapshot = tempfile::Builder::new()
        .prefix("head-")
        .tempdir_in(&state_dir)?;
    let tree = git_bytes(&repo, &["ls-tree", "-r", "-z", "HEAD"])?;
    for entry in tree.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        crate::operation::check()?;
        let entry = std::str::from_utf8(entry).context("Git path is not UTF-8")?;
        let (header, name) = entry.split_once('\t').context("invalid Git tree entry")?;
        if !header.starts_with("100") {
            continue;
        } // Never materialize symlinks/submodules.
        let rel = Path::new(name);
        anyhow::ensure!(
            rel.components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "unsafe Git tree path"
        );
        let target = snapshot.path().join(rel);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let oid = header
            .split_whitespace()
            .nth(2)
            .context("missing blob ID")?;
        fs::write(target, git_bytes(&repo, &["cat-file", "blob", oid])?)?;
    }
    let baseline_project = snapshot.path().join(sub);
    fs::create_dir_all(&baseline_project)?;
    // Apply the same analysis configuration to both sides.
    let cfg = project.join(".code-rcl/config.toml");
    if cfg.is_file() {
        fs::create_dir_all(baseline_project.join(".code-rcl"))?;
        fs::copy(cfg, baseline_project.join(".code-rcl/config.toml"))?;
    }
    let mut baseline_precise = args.precise.clone();
    baseline_precise.precise = false; // Baseline is never allowed to execute historical build scripts.
    let before = crate::commands::graph::build_graph(&crate::service::analysis_query(
        &baseline_project,
        args.kinds.clone(),
        false,
        baseline_precise,
    ))?;
    let after = crate::commands::graph::build_graph(&crate::service::analysis_query(
        &project,
        args.kinds.clone(),
        args.no_sync,
        args.precise.clone(),
    ))?;
    let paths = git_bytes(
        &repo,
        &["diff", "--name-only", "-z", "--no-renames", "HEAD", "--"],
    )?;
    let mut old_ranges = Vec::new();
    let mut new_ranges = Vec::new();
    for path in paths.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        crate::operation::check()?;
        let path = std::str::from_utf8(path).context("Git path is not UTF-8")?;
        let Ok(rel) = Path::new(path).strip_prefix(sub) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let patch = run_git(
            &repo,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--unified=0",
                "HEAD",
                "--",
                path,
            ],
        )?;
        for line in patch.lines().filter(|l| l.starts_with("@@ ")) {
            for (prefix, ranges) in [('-', &mut old_ranges), ('+', &mut new_ranges)] {
                if let Some(spec) = line.split_whitespace().find(|s| s.starts_with(prefix)) {
                    let mut parts = spec[1..].split(',');
                    let start: i64 = parts.next().unwrap_or("0").parse()?;
                    let count: i64 = parts.next().unwrap_or("1").parse()?;
                    if count > 0 {
                        ranges.push(ChangedRange {
                            path: rel.clone(),
                            start_line: start,
                            end_line: start + count - 1,
                        });
                    }
                }
            }
        }
    }
    let select = |root: &Path, ranges: &[ChangedRange]| -> Result<HashSet<String>> {
        let db = CacheDb::open(root)?;
        let files = db.all_files()?;
        Ok(resolve_changed_symbols(&db, ranges)?
            .iter()
            .filter_map(|s| {
                files
                    .iter()
                    .find(|f| f.id == s.file_id)
                    .map(|f| symbol_id(&f.path, &s.name, s.start_line.unwrap_or(0)))
            })
            .collect())
    };
    let old_ids = select(&baseline_project, &old_ranges)?;
    let new_ids = select(&project, &new_ranges)?;
    let old_identity = symbol_identities(&baseline_project)?;
    let new_identity = symbol_identities(&project)?;
    let mut results: BTreeMap<(Option<String>, String, String, String), super::ImpactReport> =
        BTreeMap::new();
    for (graph, ids, root, old) in [
        (&before, old_ids, &baseline_project, true),
        (&after, new_ids, &project, false),
    ] {
        let local_args = crate::cli::ImpactArgs {
            project: root.clone(),
            symbol: None,
            diff: true,
            depth: args.depth,
            direction: args.direction.clone(),
            kinds: args.kinds.clone(),
            json: true,
            no_sync: true,
            precise: Default::default(),
        };
        for mut report in super::reports_for_graph(
            &local_args,
            graph,
            graph.nodes.iter().filter(|n| ids.contains(&n.id)).collect(),
        )? {
            let identity = if old { &old_identity } else { &new_identity };
            let identity = identity.get(&report.target_id).cloned().unwrap_or_default();
            let key = (
                report.target_path.clone(),
                report.target_symbol.clone(),
                report.target_kind.clone(),
                identity.clone(),
            );
            let other_graph = if old { &after } else { &before };
            let other_identity = if old { &new_identity } else { &old_identity };
            let exists = other_graph.nodes.iter().any(|n| {
                n.path == report.target_path
                    && n.label == report.target_symbol
                    && n.kind == report.target_kind
                    && other_identity.get(&n.id) == Some(&identity)
            });
            report.change_kind = Some(
                if exists {
                    "modified"
                } else if old {
                    "deleted"
                } else {
                    "added"
                }
                .into(),
            );
            if let Some(previous) = results.remove(&key) {
                merge_items(&mut report.callers, previous.callers);
                merge_items(&mut report.callees, previous.callees);
                report.direct_callers_count = report.callers.len();
                report.direct_callees_count = report.callees.len();
                let mut up = HashSet::new();
                collect_items(&report.callers, &mut up);
                let mut down = HashSet::new();
                collect_items(&report.callees, &mut down);
                report.max_depth_reached = 0;
                super::calculate_max_depth(&report.callers, &mut report.max_depth_reached);
                super::calculate_max_depth(&report.callees, &mut report.max_depth_reached);
                report.risky_affected_count = 0;
                let mut risky = HashSet::new();
                super::count_risky(
                    &report.callers,
                    &mut risky,
                    &mut report.risky_affected_count,
                );
                super::count_risky(
                    &report.callees,
                    &mut risky,
                    &mut report.risky_affected_count,
                );
                report.total_callers_count = up.len();
                report.total_callees_count = down.len();
                up.extend(down);
                report.total_affected_count = up.len();
                report.total_affected_files = up
                    .iter()
                    .filter_map(|(p, _)| p.as_ref())
                    .collect::<HashSet<_>>()
                    .len();
            }
            results.insert(key, report);
        }
    }
    Ok(results.into_values().collect())
}

/// Keep same-named methods on unrelated owners separate across revisions.
fn symbol_identities(root: &Path) -> Result<std::collections::HashMap<String, String>> {
    let db = CacheDb::open(root)?;
    let files = db.all_files()?;
    let symbols = db.all_symbols()?;
    let by_id = symbols
        .iter()
        .map(|s| (s.id, s))
        .collect::<std::collections::HashMap<_, _>>();
    Ok(symbols
        .iter()
        .filter_map(|s| {
            let file = files.iter().find(|f| f.id == s.file_id)?;
            let path = &file.path;
            let mut owners = Vec::new();
            let mut parent = s.parent_symbol_id;
            let mut seen = HashSet::new();
            while let Some(p) = parent
                .and_then(|id| by_id.get(&id))
                .filter(|p| seen.insert(p.id))
            {
                owners.push(p.name.clone());
                parent = p.parent_symbol_id;
            }
            Some((
                crate::graph::symbol_id(path, &s.name, s.start_line.unwrap_or(0)),
                format!(
                    "{:?}|{:?}|{:?}",
                    owners,
                    s.type_name,
                    if matches!(file.language.as_str(), "java" | "kotlin") {
                        s.param_count
                    } else {
                        None
                    }
                ),
            ))
        })
        .collect())
}

fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git").current_dir(root).args(args).output()?;
    anyhow::ensure!(
        out.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(out.stdout)
}

fn merge_items(items: &mut Vec<super::ImpactItem>, others: Vec<super::ImpactItem>) {
    for item in others {
        if let Some(existing) = items
            .iter_mut()
            .find(|i| i.path == item.path && i.label == item.label && i.edge_kind == item.edge_kind)
        {
            merge_items(&mut existing.callers, item.callers);
        } else {
            items.push(item);
        }
    }
}

fn collect_items(items: &[super::ImpactItem], seen: &mut HashSet<(Option<String>, String)>) {
    for item in items {
        seen.insert((item.path.clone(), item.label.clone()));
        collect_items(&item.callers, seen);
    }
}

/// One project-relative, `/`-normalized new-file line range touched by the working tree diff.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangedRange {
    pub path: String,
    /// 1-based, inclusive.
    pub start_line: i64,
    /// 1-based, inclusive.
    pub end_line: i64,
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .context("git not found on PATH; `impact --diff` requires git to detect changed symbols")?;
    if !out.status.success() {
        bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse a unified-diff hunk header (`@@ -a,b +c,d @@...`) and return the
/// new-file `(start_line, line_count)`. `line_count` defaults to `1` when
/// omitted, per unified-diff convention.
#[cfg(test)]
fn parse_hunk_header(line: &str) -> Option<(i64, i64)> {
    let rest = line.strip_prefix("@@ ")?;
    let plus_start = rest.find('+')?;
    let after_plus = &rest[plus_start + 1..];
    let end = after_plus.find(' ')?;
    let spec = &after_plus[..end];

    let mut parts = spec.splitn(2, ',');
    let start: i64 = parts.next()?.parse().ok()?;
    let count: i64 = match parts.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

/// Resolve each `ChangedRange` to the overlapping symbol(s) in the cache,
/// deduplicated by symbol id.
pub fn resolve_changed_symbols(db: &CacheDb, ranges: &[ChangedRange]) -> Result<Vec<SymbolRow>> {
    let mut seen = HashSet::new();
    let mut symbols = Vec::new();

    for range in ranges {
        let Some(file) = db.file_by_path(&range.path)? else {
            continue;
        };
        for sym in db.symbols_overlapping_lines(file.id, range.start_line, range.end_line)? {
            if seen.insert(sym.id) {
                symbols.push(sym);
            }
        }
    }

    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunk_header_with_explicit_count() {
        assert_eq!(
            parse_hunk_header("@@ -1,2 +3,4 @@ fn foo() {"),
            Some((3, 4))
        );
    }

    #[test]
    fn parses_hunk_header_with_omitted_count() {
        assert_eq!(parse_hunk_header("@@ -10,0 +15 @@"), Some((15, 1)));
    }

    #[test]
    fn parses_pure_deletion_hunk_as_zero_count() {
        assert_eq!(parse_hunk_header("@@ -5,2 +7,0 @@"), Some((7, 0)));
    }

    #[test]
    fn ignores_non_hunk_lines() {
        assert_eq!(parse_hunk_header("diff --git a/foo.rs b/foo.rs"), None);
        assert_eq!(parse_hunk_header("Binary files a/x and b/x differ"), None);
    }
}
