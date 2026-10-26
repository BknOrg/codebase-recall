use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::CacheDb;
use crate::cli::SearchArgs;
use crate::commands::digest::extractor::extract_signature;

#[cfg(all(test, feature = "embeddings"))]
#[path = "jina_experiment.rs"]
mod jina_experiment;
#[path = "search_retrieval.rs"]
pub(crate) mod retrieval;

#[derive(Debug, Serialize)]
pub struct SearchResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matches: Option<Vec<crate::embeddings::Match>>,
    pub total_matches: usize,
    pub match_counts: std::collections::BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    pub query: String,
    pub exact_symbols: Vec<SymbolSearchItem>,
    pub string_literals: Vec<StringLiteralSearchItem>,
    pub fuzzy_suggestions: Vec<FuzzySearchItem>,
    pub grep_matches: Vec<GrepSearchItem>,
}

#[derive(Debug, Serialize)]
pub struct SymbolSearchItem {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: Option<i64>,
    pub is_exported: bool,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct StringLiteralSearchItem {
    pub value: String,
    pub callee: Option<String>,
    pub path: String,
    pub line: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct FuzzySearchItem {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub line: Option<i64>,
    pub distance: usize,
}

#[derive(Debug, Serialize)]
pub struct GrepSearchItem {
    pub path: String,
    pub line: usize,
    pub snippet: String,
}

pub(crate) fn resolve_mode(args: &SearchArgs, automatic: bool) -> &'static str {
    if args.semantic {
        "semantic"
    } else if args.hybrid {
        "hybrid"
    } else if args.lexical
        || args.strings
        || args.no_grep
        || args.no_fuzzy
        || args.match_mode.is_some()
    {
        "lexical"
    } else if automatic && args.query.trim().chars().any(char::is_whitespace) {
        "hybrid"
    } else {
        "lexical"
    }
}
pub(crate) fn mode_reason(args: &SearchArgs, automatic: bool) -> &'static str {
    if args.semantic || args.hybrid || args.lexical {
        "explicit_mode"
    } else if args.strings || args.no_grep || args.no_fuzzy || args.match_mode.is_some() {
        "lexical_options"
    } else if !automatic && args.query.trim().chars().any(char::is_whitespace) {
        "quality_gate_pending"
    } else if args.query.trim().chars().any(char::is_whitespace) {
        "task_description"
    } else {
        "identifier_or_qualified_name"
    }
}

fn name_candidates(
    db: &CacheDb,
    args: &SearchArgs,
) -> Result<Vec<(crate::cache::models::SymbolRow, String)>> {
    let normalized = args.query.replace("::", ".");
    let parts = normalized.split('.').collect::<Vec<_>>();
    let identifier = |s: &str| {
        s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_alphanumeric() || c == '_')
    };
    if parts.len() < 2 || !parts.iter().all(|s| identifier(s)) {
        return db.search_symbols(
            &args.query,
            args.match_mode.as_deref().unwrap_or("substring"),
            args.kind.as_deref(),
            args.exported,
            usize::MAX,
        );
    }
    let symbols: HashMap<_, _> = db.all_symbols()?.into_iter().map(|s| (s.id, s)).collect();
    let wanted = parts[..parts.len() - 1]
        .iter()
        .map(|s| s.to_lowercase())
        .collect::<Vec<_>>();
    let mut rows = db.search_symbols(
        parts[parts.len() - 1],
        args.match_mode.as_deref().unwrap_or("substring"),
        args.kind.as_deref(),
        args.exported,
        usize::MAX,
    )?;
    rows.retain(|(s, path)| {
        let mut owner = vec![];
        let mut id = s.parent_symbol_id;
        let mut seen = std::collections::HashSet::new();
        while let Some(parent) = id {
            if !seen.insert(parent) {
                break;
            }
            let Some(parent) = symbols.get(&parent) else {
                break;
            };
            owner.push(parent.name.to_lowercase());
            id = parent.parent_symbol_id;
        }
        owner.reverse();
        let mut namespace = Path::new(path)
            .with_extension("")
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(v) => Some(v.to_string_lossy().to_lowercase()),
                _ => None,
            })
            .collect::<Vec<_>>();
        namespace.extend(owner);
        namespace.ends_with(&wanted)
    });
    Ok(rows)
}

pub fn run(args: SearchArgs) -> Result<()> {
    if resolve_mode(&args, retrieval::profile().promoted) != "lexical" {
        crate::progress::install_interrupt()?;
        crate::progress::sink(Some(crate::progress::cli_sink("auto")));
    }
    let result = execute_search(&args)?;

    if args.json {
        println!(
            "{}",
            crate::output::json(&resolve_project_root(&args.project), args.no_sync, &result)?
        );
    } else {
        print!("{}", render_search_result(&result));
    }

    Ok(())
}

pub fn execute_search(args: &SearchArgs) -> Result<SearchResult> {
    let profile = retrieval::profile();
    anyhow::ensure!(
        !args.automatic || profile.promoted || resolve_mode(args, true) == "lexical",
        "automatic feature retrieval has not passed release gates; use explicit lexical or the current semantic mode"
    );
    anyhow::ensure!(args.limit > 0, "search limit must be greater than zero");
    anyhow::ensure!(
        usize::from(args.semantic) + usize::from(args.hybrid) + usize::from(args.lexical) <= 1,
        "semantic and hybrid are mutually exclusive"
    );
    anyhow::ensure!(
        !(args.semantic || args.hybrid) || (!args.strings && !args.no_grep),
        "strings and grep options are lexical-only"
    );
    anyhow::ensure!(
        !(args.semantic || args.hybrid) || (1..=100).contains(&args.limit),
        "semantic/hybrid limit must be 1..=100"
    );
    let project_root = resolve_project_root(&args.project);
    let selection = crate::selection::Selection::new(&project_root, &args.filters)?;

    let mut db = CacheDb::open(&project_root)
        .context("failed to open graph cache DB; run `code-rcl init` first")?;

    if !args.no_sync {
        crate::service::auto_sync(&mut db, &project_root)?;
    }

    let mode = resolve_mode(args, profile.promoted);
    crate::operation::retrieval(
        serde_json::json!({"mode":mode,"selection_reason":mode_reason(args,profile.promoted),"automatic_available":profile.promoted}),
    );
    if mode != "lexical" && profile.promoted {
        return crate::embeddings::with_model(&project_root, |embedder| {
            retrieval::execute_with(&mut db, &project_root, args, embedder, &profile)
        });
    }
    if args.semantic || args.hybrid {
        return execute_retrieval(&mut db, &project_root, args);
    }
    // 1. Exact symbol search
    let config = crate::config::ProjectConfig::load(&project_root)?;
    let allowed: std::collections::HashSet<_> = crate::embeddings::scope(&db, &config)?
        .into_iter()
        .map(|f| f.path)
        .collect();
    let mut sym_rows = name_candidates(&db, args)?;
    sym_rows.retain(|(_, path)| allowed.contains(path) && selection.accepts(path));
    let exact_count = sym_rows.len();
    sym_rows.truncate(args.limit);
    let files: HashMap<_, _> = db
        .all_files()?
        .into_iter()
        .filter(|f| allowed.contains(&f.path))
        .map(|f| (f.path.clone(), f))
        .collect();

    // No analyzer records a signature, so read the declaration from the source
    // the same way `digest` does; the bare name is only the last resort.
    let mut source_lines: HashMap<String, Vec<String>> = HashMap::new();
    let mut exact_symbols: Vec<SymbolSearchItem> = sym_rows
        .into_iter()
        .map(|(s, path)| {
            let signature = s.signature.clone().unwrap_or_else(|| {
                let lines = source_lines.entry(path.clone()).or_insert_with(|| {
                    files
                        .get(&path)
                        .and_then(|f| db.source_snapshot(f).ok())
                        .map(|c| c.lines().map(String::from).collect())
                        .unwrap_or_default()
                });
                let sig = extract_signature(lines, s.start_line, s.end_line);
                if sig.is_empty() { s.name.clone() } else { sig }
            });
            SymbolSearchItem {
                name: s.name,
                kind: s.kind,
                path,
                line: s.start_line,
                is_exported: s.is_exported,
                signature,
            }
        })
        .collect();

    // 2. String literal search (if requested or if exact symbols is empty)
    let constrained = args.kind.is_some() || args.exported;
    let mut eligible_locations: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    if constrained && (args.strings || exact_symbols.is_empty()) {
        let file_paths: HashMap<_, _> = files.values().map(|f| (f.id, f.path.as_str())).collect();
        for symbol in db.all_symbols()? {
            if args.kind.as_ref().is_some_and(|k| k != &symbol.kind)
                || (args.exported && !symbol.is_exported)
            {
                continue;
            }
            if let (Some(path), Some(start), Some(end)) = (
                file_paths.get(&symbol.file_id),
                symbol.start_line,
                symbol.end_line,
            ) {
                eligible_locations
                    .entry((*path).into())
                    .or_default()
                    .push((start, end));
            }
        }
    }
    let string_rows = if args.strings || exact_symbols.is_empty() {
        db.search_string_literals(&args.query, usize::MAX)?
            .into_iter()
            .filter(|(s, p)| {
                files.contains_key(p)
                    && selection.accepts(p)
                    && (!constrained
                        || s.line.is_some_and(|line| {
                            eligible_locations.get(p).is_some_and(|spans| {
                                spans.iter().any(|(lo, hi)| *lo <= line && line <= *hi)
                            })
                        }))
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut string_literals: Vec<StringLiteralSearchItem> = string_rows
        .into_iter()
        .map(|(sl, path)| StringLiteralSearchItem {
            value: sl.value,
            callee: sl.callee,
            path,
            line: sl.line,
        })
        .collect();

    // 3. Fuzzy suggestions if exact symbols empty
    let mut fuzzy_suggestions =
        if exact_symbols.is_empty() && string_literals.is_empty() && !args.no_fuzzy {
            let f = db
                .fuzzy_search_symbols(&args.query, usize::MAX)?
                .into_iter()
                .filter(|(s, p, _)| {
                    files.contains_key(p)
                        && selection.accepts(p)
                        && args.kind.as_ref().is_none_or(|k| k == &s.kind)
                        && (!args.exported || s.is_exported)
                })
                .collect::<Vec<_>>();
            f.into_iter()
                .map(|(s, path, distance)| FuzzySearchItem {
                    name: s.name,
                    kind: s.kind,
                    path,
                    line: s.start_line,
                    distance,
                })
                .collect()
        } else {
            Vec::new()
        };

    // 4. Hybrid grep fallback if exact symbols & string literals are both empty
    let mut grep_matches =
        if exact_symbols.is_empty() && string_literals.is_empty() && !args.no_grep {
            perform_grep_fallback(&db, args, &selection, &allowed)?
        } else {
            Vec::new()
        };

    let match_counts = std::collections::BTreeMap::from([
        ("exact_symbols".into(), exact_count),
        ("string_literals".into(), string_literals.len()),
        ("fuzzy_suggestions".into(), fuzzy_suggestions.len()),
        ("grep_matches".into(), grep_matches.len()),
    ]);
    let total_matches = match_counts.values().sum::<usize>();
    let mut remaining = args.limit;
    macro_rules! budget {
        ($items:ident) => {
            $items.truncate(remaining);
            remaining = remaining.saturating_sub($items.len());
        };
    }
    budget!(exact_symbols);
    budget!(string_literals);
    budget!(grep_matches);
    budget!(fuzzy_suggestions);
    let _ = remaining;
    crate::operation::omitted(total_matches.saturating_sub(args.limit));
    Ok(SearchResult {
        matches: None,
        total_matches,
        match_counts,
        quality: None,
        query: args.query.clone(),
        exact_symbols,
        string_literals,
        fuzzy_suggestions,
        grep_matches,
    })
}

fn resolve_project_root(path: &Path) -> PathBuf {
    if path.is_file() {
        let mut cur = path.parent().unwrap_or_else(|| Path::new("."));
        while let Some(parent) = cur.parent() {
            if cur.join(".code-rcl").exists()
                || cur.join("Cargo.toml").exists()
                || cur.join("package.json").exists()
                || cur.join("pyproject.toml").exists()
            {
                return cur.to_path_buf();
            }
            cur = parent;
        }
    }
    path.to_path_buf()
}

fn perform_grep_fallback(
    db: &CacheDb,
    args: &SearchArgs,
    selection: &crate::selection::Selection,
    allowed: &std::collections::HashSet<String>,
) -> Result<Vec<GrepSearchItem>> {
    let files = db.all_files()?;
    let q_lower = args.query.to_lowercase();
    let wanted: std::collections::HashSet<_> = crate::cache::content::terms(&args.query)
        .into_iter()
        .collect();
    let mut matches = Vec::new();

    for f in files {
        if !allowed.contains(&f.path) || !selection.accepts(&f.path) {
            continue;
        }
        let content = db.source_snapshot(&f)?;
        let symbols = db.symbols_for_file(f.id)?;
        let eligible = symbols
            .iter()
            .filter(|s| {
                args.kind.as_ref().is_none_or(|k| k == &s.kind) && (!args.exported || s.is_exported)
            })
            .collect::<Vec<_>>();
        let mut offset = 0;
        for (idx, line) in content.lines().enumerate() {
            let at = offset;
            // Account for CRLF as well as LF using source bytes rather than normalized lines.
            offset = content[at..]
                .find('\n')
                .map_or(content.len(), |n| at + n + 1);
            if (args.kind.is_some() || args.exported)
                && !eligible.iter().any(|s| {
                    s.start_byte
                        .is_some_and(|lo| lo >= 0 && (lo as usize) < at + line.len())
                        && s.end_byte.is_some_and(|hi| hi as usize > at)
                })
            {
                continue;
            }
            let full = line.to_lowercase().contains(&q_lower);
            let terms: std::collections::HashSet<_> =
                crate::cache::content::terms(line).into_iter().collect();
            let coverage = wanted.intersection(&terms).count();
            if full || coverage > 0 {
                matches.push((
                    full,
                    coverage,
                    GrepSearchItem {
                        path: f.path.clone(),
                        line: idx + 1,
                        snippet: line.trim().to_string(),
                    },
                ));
            }
        }
    }

    matches.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.path.cmp(&b.2.path))
            .then(a.2.line.cmp(&b.2.line))
    });
    Ok(matches.into_iter().map(|(_, _, item)| item).collect())
}

pub fn render_search_result(res: &SearchResult) -> String {
    let mut out = String::new();
    if res.quality.as_deref() == Some("weak") {
        out.push_str("Belum ditemukan kecocokan kuat; berikut kandidat terdekat.\n\n");
    }
    if let Some(matches) = &res.matches {
        for item in matches {
            out.push_str(&format!(
                "{}:{} {} [{}] cosine={:?} rrf={:?}\n{}\n\n",
                item.path,
                item.line,
                item.name,
                item.retrieval,
                item.cosine_score,
                item.rrf_score,
                item.snippet
            ));
            if let Some(score) = item.relevance_score {
                out.push_str(&format!(
                    "relevance={score} (ranking signal, not probability)\n"
                ));
            }
            for support in &item.supporting_locations {
                out.push_str(&format!(
                    "  related {}:{} {} [{}; {}]\n",
                    support.path,
                    support.line,
                    support.name,
                    support.relation,
                    support.resolution_mode
                ));
            }
        }
        return out;
    }

    if !res.exact_symbols.is_empty() {
        out.push_str(&format!(
            "Found {} symbol(s) matching `{}`:\n\n",
            res.exact_symbols.len(),
            res.query
        ));
        out.push_str("| Symbol | Kind | Location | Visibility | Signature |\n");
        out.push_str("| :--- | :--- | :--- | :--- | :--- |\n");

        for s in &res.exact_symbols {
            let vis = if s.is_exported { "public" } else { "private" };
            let loc = match s.line {
                Some(l) => format!("{}:L{l}", s.path),
                None => s.path.clone(),
            };
            out.push_str(&format!(
                "| **`{}`** | `{}` | `{}` | {} | `{}` |\n",
                s.name,
                s.kind,
                loc,
                vis,
                s.signature.replace('|', "\\|")
            ));
        }
        out.push('\n');
    }

    if !res.string_literals.is_empty() {
        out.push_str(&format!(
            "Found {} string literal argument(s) matching `{}`:\n\n",
            res.string_literals.len(),
            res.query
        ));
        out.push_str("| String Literal | Callee / Pattern | Location |\n");
        out.push_str("| :--- | :--- | :--- |\n");

        for sl in &res.string_literals {
            let callee = sl.callee.as_deref().unwrap_or("-");
            let loc = match sl.line {
                Some(l) => format!("{}:L{l}", sl.path),
                None => sl.path.clone(),
            };
            out.push_str(&format!(
                "| \"`{}`\" | `{}` | `{}` |\n",
                sl.value, callee, loc
            ));
        }
        out.push('\n');
    }

    if res.exact_symbols.is_empty() && res.string_literals.is_empty() {
        out.push_str(&format!("No exact matches found for `{}`.\n\n", res.query));

        if !res.fuzzy_suggestions.is_empty() {
            let names: Vec<String> = res
                .fuzzy_suggestions
                .iter()
                .map(|s| format!("`{}` ({})", s.name, s.kind))
                .collect();
            out.push_str(&format!("Did you mean: {}?\n\n", names.join(", ")));
            out.push_str("| Suggested Symbol | Kind | Location | Edit Distance |\n");
            out.push_str("| :--- | :--- | :--- | :--- |\n");
            for s in &res.fuzzy_suggestions {
                let loc = match s.line {
                    Some(l) => format!("{}:L{l}", s.path),
                    None => s.path.clone(),
                };
                out.push_str(&format!(
                    "| **`{}`** | `{}` | `{}` | {} |\n",
                    s.name, s.kind, loc, s.distance
                ));
            }
            out.push('\n');
        }

        if !res.grep_matches.is_empty() {
            out.push_str(&format!(
                "Hybrid search fallback: found {} line(s) via full-text grep:\n\n",
                res.grep_matches.len()
            ));
            for m in &res.grep_matches {
                out.push_str(&format!("- `{}:{}`: `{}`\n", m.path, m.line, m.snippet));
            }
            out.push('\n');
        }
    }

    out
}

fn execute_retrieval(db: &mut CacheDb, project: &Path, args: &SearchArgs) -> Result<SearchResult> {
    let (semantic, refreshed) = crate::embeddings::with_model(project, |m| {
        let refreshed = retrieval::refresh_index(db, project, args, m)?;
        Ok((
            crate::embeddings::semantic_with(db, project, args, m)?,
            refreshed,
        ))
    })?;
    let selection = crate::selection::Selection::new(project, &args.filters)?;
    let semantic_count = semantic.len();
    let mut lexical_count = 0;
    let mut hits = if args.hybrid {
        let mut rows = hybrid_lexical_candidates(db, args, &selection)?;
        let config = crate::config::ProjectConfig::load(project)?;
        let allowed: std::collections::HashSet<_> = crate::embeddings::scope(db, &config)?
            .into_iter()
            .map(|f| f.path)
            .collect();
        rows.retain(|(_, path)| allowed.contains(path));
        crate::operation::omitted_for(rows.len().saturating_sub(100), "candidate_limit");
        rows.truncate(100);
        lexical_count = rows.len();
        let mut lexical = vec![];
        let files: HashMap<_, _> = db
            .all_files()?
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect();
        for (s, path) in rows {
            let source =
                db.source_snapshot(files.get(&path).context("cached source file missing")?)?;
            let start = s.start_byte.unwrap_or(0).max(0) as usize;
            let full_end = (s.end_byte.unwrap_or(0).max(0) as usize).min(source.len());
            let text = source
                .get(start..full_end)
                .context("cached symbol source span invalid")?;
            let end = text
                .char_indices()
                .nth(1600)
                .map_or(full_end, |(i, _)| start + i);
            let snippet = source[start..end].to_string();
            let key = crate::embeddings::chunk::symbol_key(&path, &s);
            lexical.push(crate::embeddings::Match {
                path,
                name: s.name,
                kind: s.kind,
                line: s.start_line.unwrap_or(1) as usize,
                end_line: source[..end].bytes().filter(|b| *b == b'\n').count() + 1,
                start_byte: start,
                end_byte: end,
                snippet,
                symbol_key: key,
                retrieval: "lexical".into(),
                cosine_score: None,
                relevance_score: None,
                auxiliary_cosine: None,
                supporting_locations: vec![],
                rrf_score: None,
                representation: None,
                symbol_range: if end < full_end {
                    Some([start, full_end])
                } else {
                    None
                },
            });
        }
        crate::embeddings::fuse(semantic, lexical)
    } else {
        semantic
    };
    crate::operation::retrieval(
        serde_json::json!({"mode":if args.hybrid{"hybrid"}else{"semantic"},"selection_reason":mode_reason(args,false),"pipeline":"legacy","automatic_available":false,"index_refresh":refreshed,"source_verified":!args.no_sync,"semantic_candidates":semantic_count,"lexical_candidates":lexical_count,"fused_candidates":hits.len(),"total_matches_scope":"candidate_pool","both_branches":hits.iter().filter(|h|h.retrieval=="hybrid").count(),"rrf_k":60}),
    );
    let total_matches = hits.len();
    crate::operation::omitted(total_matches.saturating_sub(args.limit));
    hits.truncate(args.limit);
    Ok(SearchResult {
        matches: Some(hits),
        total_matches,
        match_counts: std::collections::BTreeMap::from([("candidates".into(), total_matches)]),
        quality: None,
        query: args.query.clone(),
        exact_symbols: vec![],
        string_literals: vec![],
        fuzzy_suggestions: vec![],
        grep_matches: vec![],
    })
}

/// Preserve full-query lookup; only expand identifiers the query singles out.
/// Expanding every prose token can drown out semantic results, particularly for
/// container questions mentioning member names as supporting context.
fn explicit_identifiers(query: &str) -> Vec<&str> {
    fn identifier(term: &str) -> bool {
        term.as_bytes()
            .first()
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
            && term.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    }
    let mut terms = Vec::new();
    let first = query
        .trim_start()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or_default();
    if identifier(first)
        && (first.contains('_')
            || (first.bytes().any(|c| c.is_ascii_lowercase())
                && first.bytes().skip(1).any(|c| c.is_ascii_uppercase()))
            || query
                .trim_start()
                .get(first.len()..)
                .is_some_and(|s| s.starts_with('(')))
    {
        terms.push(first);
    }
    let mut quoted = query.split('`').skip(1);
    while let Some(term) = quoted.next() {
        if quoted.next().is_none() {
            break; // An unmatched backtick does not mark an explicit identifier.
        }
        if identifier(term) && !terms.iter().any(|t| t.eq_ignore_ascii_case(term)) {
            terms.push(term);
        }
        if terms.len() >= 8 {
            break;
        }
    }
    terms
}

fn hybrid_lexical_candidates(
    db: &CacheDb,
    args: &SearchArgs,
    selection: &crate::selection::Selection,
) -> Result<Vec<(crate::cache::models::SymbolRow, String)>> {
    let _timer = crate::operation::time("hybrid_lexical");
    let mut rows = name_candidates(db, args)?;
    rows.retain(|(_, path)| selection.accepts(path));
    if rows.is_empty() && args.match_mode.as_deref().unwrap_or("substring") == "substring" {
        let mut seen = std::collections::HashSet::new();
        for term in explicit_identifiers(&args.query) {
            for (symbol, path) in db.search_symbols(
                term,
                "exact",
                args.kind.as_deref(),
                args.exported,
                usize::MAX,
            )? {
                if selection.accepts(&path) && seen.insert(symbol.id) {
                    rows.push((symbol, path));
                }
            }
        }
    }
    if rows.is_empty() && !args.no_fuzzy {
        rows = db
            .fuzzy_search_symbols(&args.query, usize::MAX)?
            .into_iter()
            .filter(|(s, path, _)| {
                selection.accepts(path)
                    && args.kind.as_ref().is_none_or(|k| k == &s.kind)
                    && (!args.exported || s.is_exported)
            })
            .map(|(s, p, _)| (s, p))
            .collect();
    }
    Ok(rows)
}

#[cfg(test)]
mod hybrid_identifier_tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn sentence_verbs_do_not_masquerade_as_explicit_code_names() {
        for query in [
            "Read SQL quoted text",
            "Close the Go database handle",
            "Search vector values",
            "Run work when a task fails",
        ] {
            assert!(explicit_identifiers(query).is_empty(), "{query}");
        }
        assert_eq!(
            explicit_identifiers("Rollback(tx) after an error"),
            vec!["Rollback"]
        );
        assert_eq!(
            explicit_identifiers("Use `Rollback` after an error"),
            vec!["Rollback"]
        );
        assert_eq!(
            explicit_identifiers("RequestCoordinator handles staged writes"),
            vec!["RequestCoordinator"]
        );
    }

    #[test]
    fn automatic_mode_has_explicit_overrides_and_keeps_identifier_queries_lexical() {
        for (extra, query, mode) in [
            (vec![], "finish_request", "lexical"),
            (vec![], "db::Transaction.commit", "lexical"),
            (vec![], "roll back writes when a task fails", "hybrid"),
            (vec!["--lexical"], "roll back failed writes", "lexical"),
            (
                vec!["--match-mode", "substring"],
                "roll back failed writes",
                "lexical",
            ),
            (vec!["--semantic"], "finish_request", "semantic"),
            (vec!["--strings"], "job failed", "lexical"),
        ] {
            let mut cli = vec!["code-rcl", "search", query];
            cli.extend(extra);
            let crate::cli::Command::Search(args) =
                crate::cli::Cli::try_parse_from(cli).unwrap().command
            else {
                unreachable!()
            };
            assert_eq!(resolve_mode(&args, true), mode);
        }
        assert!(crate::cli::Cli::try_parse_from(["code-rcl", "search", "x", "--rerank"]).is_err());
        assert!(
            crate::cli::Cli::try_parse_from(["code-rcl", "search", "x", "--lexical", "--semantic"])
                .is_err()
        );
    }

    #[test]
    fn explicit_identifiers_do_not_expand_incidental_prose_or_members() {
        assert_eq!(
            explicit_identifiers("write_tx roll back writes"),
            ["write_tx"]
        );
        assert!(explicit_identifiers("Begin an explicit transaction").is_empty());
        assert_eq!(
            explicit_identifiers("`Begin` an explicit transaction"),
            ["Begin"]
        );
        assert_eq!(
            explicit_identifiers("cancelRequest when token is set"),
            ["cancelRequest"]
        );
        assert_eq!(
            explicit_identifiers("find `prepare` and `PREPARE`"),
            ["prepare"]
        );
        assert!(explicit_identifiers("container with open and in_memory factories").is_empty());
        assert!(explicit_identifiers("roll back writes when a closure fails").is_empty());
        assert!(explicit_identifiers("find `unclosed").is_empty());
        assert!(explicit_identifiers("find `two words`").is_empty());
        assert_eq!(
            explicit_identifiers("find `函数` then `valid_name`"),
            ["valid_name"]
        );
    }

    #[test]
    fn mixed_identifier_lookup_preserves_filters_exactness_and_match_mode() {
        crate::operation::begin(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            false,
        )));
        let project = tempfile::tempdir().unwrap();
        fs::write(
            project.path().join("lib.rs"),
            "pub fn write_tx() {}\nfn pre_write_tx() {}\nfn private_tx() {}\n",
        )
        .unwrap();
        fs::write(
            project.path().join("binding.py"),
            "def write_tx():\n    pass\n",
        )
        .unwrap();
        let mut db = CacheDb::open(project.path()).unwrap();
        crate::service::auto_sync(&mut db, project.path()).unwrap();
        let mut args = match crate::cli::Cli::parse_from([
            "code-rcl",
            "search",
            "write_tx roll back writes",
            "--hybrid",
            "--no-fuzzy",
        ])
        .command
        {
            crate::cli::Command::Search(args) => args,
            _ => unreachable!(),
        };
        let selection = crate::selection::Selection::new(project.path(), &args.filters).unwrap();
        let rows = hybrid_lexical_candidates(&db, &args, &selection).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|(symbol, _)| symbol.name == "write_tx"));
        args.filters.languages = vec!["rust".into()];
        let selection = crate::selection::Selection::new(project.path(), &args.filters).unwrap();
        assert_eq!(
            hybrid_lexical_candidates(&db, &args, &selection)
                .unwrap()
                .len(),
            1
        );
        args.filters.languages.clear();
        args.filters.paths = vec!["lib.rs".into()];
        let selection = crate::selection::Selection::new(project.path(), &args.filters).unwrap();
        assert_eq!(
            hybrid_lexical_candidates(&db, &args, &selection)
                .unwrap()
                .len(),
            1
        );
        args.kind = Some("class".into());
        assert!(
            hybrid_lexical_candidates(&db, &args, &selection)
                .unwrap()
                .is_empty()
        );
        args.kind = None;
        args.query = "private_tx explain the private API".into();
        args.exported = true;
        assert!(
            hybrid_lexical_candidates(&db, &args, &selection)
                .unwrap()
                .is_empty()
        );
        args.exported = false;
        args.query = "write_tx roll back writes".into();
        args.match_mode = Some("exact".into());
        assert!(
            hybrid_lexical_candidates(&db, &args, &selection)
                .unwrap()
                .is_empty()
        );
        args.match_mode = Some("substring".into());
        args.query = "find `write_tx` and `WRITE_TX`".into();
        assert_eq!(
            hybrid_lexical_candidates(&db, &args, &selection)
                .unwrap()
                .len(),
            1
        );
    }
}
