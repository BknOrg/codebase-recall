use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{Value, json};

use crate::cache::CacheDb;
use crate::cli::{DigestArgs, ExplainArgs, GraphQuery, ImpactArgs, PathArgs, PreciseArgs};
use crate::commands::{digest, dump, explain, graph, impact, path, report, sync};

pub fn call_tool(default_project: &Path, name: &str, args: &Value) -> Value {
    let result = match name {
        "code_rcl_digest" => execute_digest(default_project, args),
        "code_rcl_impact" => execute_impact(default_project, args),
        "code_rcl_path" => execute_path(default_project, args),
        "code_rcl_explain" => execute_explain(default_project, args),
        "code_rcl_report" => execute_report(default_project, args),
        "code_rcl_dump" => execute_dump(default_project, args),
        "code_rcl_search" => execute_search(default_project, args),
        "code_rcl_sync" => execute_sync(default_project, args),
        "code_rcl_graph" => execute_graph(default_project, args),
        _ => Err(anyhow::anyhow!("Unknown tool: {name}")),
    };

    match result {
        Ok(text) => json!({
            "content": [
                {
                    "type": "text",
                    "text": text
                }
            ],
            "isError": false
        }),
        Err(err) => json!({
            "content": [
                {
                    "type": "text",
                    "text": format!("Error: {err:#}")
                }
            ],
            "isError": true
        }),
    }
}

pub fn resolve_project(default_project: &Path, args: &Value) -> PathBuf {
    if let Some(p) = args.get("project").and_then(|v| v.as_str()) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    default_project.to_path_buf()
}

pub fn execute_digest(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let sub_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let doc_lines = args.get("doc_lines").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let with_docs = args
        .get("with_docs")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let digest_args = DigestArgs {
        path: sub_path,
        project: Some(project),
        output: None,
        all,
        doc_lines,
        with_docs,
        json: as_json,
        no_sync: false,
    };

    let report = digest::generate_digest(&digest_args)?;
    if as_json {
        Ok(serde_json::to_string_pretty(&report)?)
    } else {
        Ok(digest::format_markdown_digest(&report))
    }
}

pub fn execute_impact(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let diff = args.get("diff").and_then(|v| v.as_bool()).unwrap_or(false);
    let symbol = args.get("symbol").and_then(|v| v.as_str()).map(str::to_string);

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| (d as u32).clamp(1, 8))
        .unwrap_or(2);

    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("both")
        .to_string();

    let kinds: Vec<String> = args
        .get("kinds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "calls".to_string(),
                "imports".to_string(),
                "references".to_string(),
            ]
        });

    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);
    let precise = args
        .get("precise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let impact_args = ImpactArgs {
        symbol,
        diff,
        project,
        depth,
        direction,
        kinds,
        json: as_json,
        no_sync: false,
        precise: PreciseArgs {
            precise,
            precise_full: false,
            precise_timeout: 15,
        },
    };

    let mut reports = impact::generate_reports(&impact_args)?;
    let trim = if diff {
        impact::trim_diff_reports(&mut reports, impact::DIFF_TARGET_LIMIT)
    } else {
        impact::DiffTrim::default()
    };
    if as_json {
        // A note would break the JSON array clients parse.
        impact::render_json(&reports)
    } else {
        let mut out = impact::render_ascii(&reports);
        if let Some(note) = trim.note() {
            out.push_str(&format!("
{note}
"));
        }
        Ok(out)
    }
}

pub fn execute_path(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(from) = args.get("from").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'from'");
    };
    let Some(to) = args.get("to").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'to'");
    };

    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("forward")
        .to_string();
    let max_depth = args
        .get("max_depth")
        .and_then(|v| v.as_u64())
        .map(|d| (d as u32).clamp(1, 20))
        .unwrap_or(8);
    let kinds: Vec<String> = args
        .get("kinds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "calls".to_string(),
                "imports".to_string(),
                "references".to_string(),
            ]
        });
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let path_args = PathArgs {
        from: from.to_string(),
        to: to.to_string(),
        project,
        kinds,
        direction,
        max_depth,
        json: as_json,
        no_sync: false,
        precise: PreciseArgs::default(),
    };

    let reports = path::generate_reports(&path_args)?;
    if as_json {
        path::render_json(&reports)
    } else {
        Ok(path::render_ascii(&reports))
    }
}

pub fn execute_explain(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(symbol) = args.get("symbol").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'symbol'");
    };
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let explain_args = ExplainArgs {
        symbol: symbol.to_string(),
        project,
        json: as_json,
        no_sync: false,
        precise: PreciseArgs::default(),
    };

    let reports = explain::generate_reports(&explain_args)?;
    if as_json {
        explain::render_json(&reports)
    } else {
        Ok(explain::render_ascii(&reports))
    }
}

pub fn execute_report(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let data = report::generate(&project, false)?;
    if as_json {
        Ok(serde_json::to_string_pretty(&data)?)
    } else {
        Ok(report::render_markdown(&data))
    }
}

pub fn execute_dump(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(target) = args.get("target").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'target'");
    };

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| (d as u32).clamp(1, 5))
        .unwrap_or(2);

    let max_size_kb = args
        .get("max_size_kb")
        .and_then(|v| v.as_u64())
        .unwrap_or(50);

    let (content, _) = dump::generate_relation_bundle(&project, target, depth, max_size_kb, false)?;
    Ok(content)
}

pub fn execute_search(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
        anyhow::bail!("Missing required argument 'query'");
    };
    let kind = args.get("kind").and_then(|v| v.as_str()).map(String::from);
    let exported = args
        .get("exported_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let strings = args
        .get("strings")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(25)
        .clamp(1, 100) as usize;
    let as_json = args.get("json").and_then(|v| v.as_bool()).unwrap_or(false);

    let search_args = crate::cli::SearchArgs {
        query: query.to_string(),
        project,
        kind,
        exported,
        strings,
        limit,
        json: as_json,
        no_fuzzy: false,
        no_grep: false,
        no_sync: false,
    };

    let result = crate::commands::search::execute_search(&search_args)?;
    if as_json {
        Ok(serde_json::to_string_pretty(&result)?)
    } else {
        Ok(crate::commands::search::render_search_result(&result))
    }
}

pub fn execute_sync(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let mut db = CacheDb::open(&project)?;
    let precise = args
        .get("precise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let full = args.get("full").and_then(|v| v.as_bool()).unwrap_or(false);
    let languages: Vec<String> = args
        .get("languages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|k| k.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let sync_args = crate::cli::SyncArgs {
        project: project.clone(),
        max_file_kb: 512,
        language: languages,
        no_report: false,
        precise: PreciseArgs {
            precise,
            precise_full: full,
            precise_timeout: 15,
        },
    };

    let stats = sync::sync_cache(&mut db, &sync_args)?;
    let (total_symbols, total_imports) = db.totals()?;
    let mut out = format!(
        "Sync completed: {} scanned (+{} added, ~{} changed, ={} unchanged, -{} removed), {} symbols ({} re-parsed), {} imports ({} re-parsed)",
        stats.scanned,
        stats.added,
        stats.changed,
        stats.unchanged,
        stats.removed,
        total_symbols,
        stats.symbols,
        total_imports,
        stats.imports
    );

    if let Err(e) = report::refresh_after_sync(&project, &stats, precise) {
        eprintln!("warning: could not refresh the code report: {e:#}");
    }

    if precise {
        let precise_stats = sync::run_precise(&mut db, &sync_args)?;
        sync::report_precise(&precise_stats);

        out.push_str("\n\nPrecise Resolution (L0 Ground Truth):\n");
        for outcome in &precise_stats.languages {
            out.push_str(&format!(
                "- **{}** (via `{}`): resolved {}/{} refs in {} file(s) ({} external, {} unresolved)\n",
                outcome.language,
                outcome.server,
                outcome.hits,
                outcome.queried,
                outcome.files,
                outcome.external,
                outcome.unresolved,
            ));
        }
        if precise_stats.languages.is_empty() {
            out.push_str("- All references were already up to date in cache.\n");
        }
        if !precise_stats.warnings.is_empty() {
            out.push_str("\nWarnings:\n");
            for w in &precise_stats.warnings {
                out.push_str(&format!("- {w}\n"));
            }
        }
    }

    Ok(out)
}

pub fn execute_graph(default_project: &Path, args: &Value) -> Result<String> {
    let project = resolve_project(default_project, args);
    let scope = args
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("both")
        .to_string();

    let focus = args
        .get("focus")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let depth = args
        .get("depth")
        .and_then(|v| v.as_u64())
        .map(|d| d as u32)
        .unwrap_or(2);

    let precise = args
        .get("precise")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let query = GraphQuery {
        project,
        scope,
        kinds: vec![
            "imports".to_string(),
            "calls".to_string(),
            "contains".to_string(),
        ],
        path: None,
        focus,
        depth,
        min_confidence: 0.0,
        include_external: false,
        max_nodes: 2000,
        no_sync: false,
        precise: PreciseArgs {
            precise,
            precise_full: false,
            precise_timeout: 15,
        },
    };

    let code_graph = graph::build_graph(&query)?;
    Ok(serde_json::to_string_pretty(&code_graph)?)
}
