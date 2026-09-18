use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cache::CacheDb;
use crate::cli::SearchArgs;
use crate::commands::digest::extractor::extract_signature;

#[derive(Debug, Serialize)]
pub struct SearchResult {
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

pub fn run(args: SearchArgs) -> Result<()> {
    let result = execute_search(&args)?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", render_search_result(&result));
    }

    Ok(())
}

pub fn execute_search(args: &SearchArgs) -> Result<SearchResult> {
    let project_root = resolve_project_root(&args.project);

    let mut db = CacheDb::open(&project_root)
        .context("failed to open graph cache DB; run `code-rcl init` first")?;

    if !args.no_sync {
        let _ = crate::service::auto_sync(&mut db, &project_root);
    }

    // 1. Exact symbol search
    let sym_rows = db.search_symbols(
        &args.query,
        args.kind.as_deref(),
        args.exported,
        args.limit,
    )?;

    // No analyzer records a signature, so read the declaration from the source
    // the same way `digest` does; the bare name is only the last resort.
    let mut source_lines: HashMap<String, Vec<String>> = HashMap::new();
    let exact_symbols: Vec<SymbolSearchItem> = sym_rows
        .into_iter()
        .map(|(s, path)| {
            let signature = s.signature.clone().unwrap_or_else(|| {
                let lines = source_lines.entry(path.clone()).or_insert_with(|| {
                    fs::read_to_string(project_root.join(&path))
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
    let string_rows = if args.strings || exact_symbols.is_empty() {
        db.search_string_literals(&args.query, args.limit).unwrap_or_default()
    } else {
        Vec::new()
    };

    let string_literals: Vec<StringLiteralSearchItem> = string_rows
        .into_iter()
        .map(|(sl, path)| StringLiteralSearchItem {
            value: sl.value,
            callee: sl.callee,
            path,
            line: sl.line,
        })
        .collect();

    // 3. Fuzzy suggestions if exact symbols empty
    let fuzzy_suggestions = if exact_symbols.is_empty() && !args.no_fuzzy {
        let f = db.fuzzy_search_symbols(&args.query, 6).unwrap_or_default();
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
    let grep_matches = if exact_symbols.is_empty()
        && string_literals.is_empty()
        && !args.no_grep
    {
        perform_grep_fallback(&project_root, &db, &args.query, 15).unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(SearchResult {
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
    project_root: &Path,
    db: &CacheDb,
    query: &str,
    limit: usize,
) -> Result<Vec<GrepSearchItem>> {
    let files = db.all_files()?;
    let q_lower = query.to_ascii_lowercase();
    let mut matches = Vec::new();

    for f in files {
        let abs_path = project_root.join(&f.path);
        let Ok(content) = fs::read_to_string(&abs_path) else {
            continue;
        };

        for (idx, line) in content.lines().enumerate() {
            if line.to_ascii_lowercase().contains(&q_lower) {
                matches.push(GrepSearchItem {
                    path: f.path.clone(),
                    line: idx + 1,
                    snippet: line.trim().to_string(),
                });
                if matches.len() >= limit {
                    return Ok(matches);
                }
            }
        }
    }

    Ok(matches)
}

pub fn render_search_result(res: &SearchResult) -> String {
    let mut out = String::new();

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
                s.name, s.kind, loc, vis, s.signature.replace('|', "\\|")
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
